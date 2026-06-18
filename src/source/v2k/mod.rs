pub mod codec;
pub mod transport;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use self::codec::{Frame, FrameDecoder};
use self::transport::ByteTransport;
use crate::source::{
    CAP_ENUM, DataSource, DeviceInfo, ParamWrite, ScopeMode, SourceCommand, SourceEvent,
    SourceHandle, TransportEndpoint,
};

const EXPECTED_CONTRACT_VERSION: u16 = 6;
#[cfg(not(test))]
const REQUEST_TIMEOUT: Duration = Duration::from_millis(150);
#[cfg(test)]
const REQUEST_TIMEOUT: Duration = Duration::from_millis(5);
const STATUS_PERIOD: Duration = Duration::from_millis(250);
const BLOCK_POLL_PERIOD: Duration = Duration::from_millis(8);
const MAX_RETRIES: usize = 2;

#[derive(Default)]
pub struct V2kSource;

impl DataSource for V2kSource {
    fn spawn(self: Box<Self>) -> SourceHandle {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        thread::Builder::new()
            .name("v2k-source".into())
            .spawn(move || worker(command_rx, event_tx))
            .expect("spawn V2kSource worker");
        SourceHandle {
            commands: command_tx,
            events: event_rx,
        }
    }
}

struct Session {
    transport: Box<dyn ByteTransport>,
    decoder: FrameDecoder,
    next_sequence: u16,
    info: DeviceInfo,
    scope_active: bool,
    expected_block_sequence: Option<u16>,
    next_status: Instant,
    next_block_poll: Instant,
}

impl Session {
    fn connect(endpoint: &TransportEndpoint, events: &mpsc::Sender<SourceEvent>) -> Result<Self> {
        let transport = transport::open(endpoint)?;
        let label = transport.label().to_owned();
        let now = Instant::now();
        let mut session = Self {
            transport,
            decoder: FrameDecoder::default(),
            next_sequence: 1,
            info: DeviceInfo {
                protocol_version: 0,
                contract_version: 0,
                build_hash: 0,
                descriptor_count: 0,
                firmware_name: String::new(),
                tick_hz: 0,
                capabilities: 0,
            },
            scope_active: false,
            expected_block_sequence: None,
            next_status: now,
            next_block_poll: now,
        };
        let hello = session.request(codec::message::HELLO, &codec::hello_request(), events)?;
        let info = codec::parse_hello(&hello.payload)?;
        validate_device_info(&info)?;
        session.info = info;
        send_event(
            events,
            SourceEvent::Log(format!("connected through {label}")),
        );
        Ok(session)
    }

    fn request(
        &mut self,
        message_type: u8,
        payload: &[u8],
        events: &mpsc::Sender<SourceEvent>,
    ) -> Result<Frame> {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        let wire = codec::encode_frame(message_type, sequence, payload);
        let expected_type = message_type | 0x80;

        for attempt in 0..=MAX_RETRIES {
            self.transport
                .write_all(&wire)
                .with_context(|| format!("send message 0x{message_type:02X}"))?;
            let deadline = Instant::now() + REQUEST_TIMEOUT;
            let mut read_buffer = [0_u8; 512];
            while Instant::now() < deadline {
                let count = self.transport.read(&mut read_buffer)?;
                if count == 0 {
                    continue;
                }
                for result in self.decoder.push(&read_buffer[..count]) {
                    match result {
                        Ok(frame)
                            if frame.sequence == sequence
                                && frame.message_type == expected_type =>
                        {
                            return Ok(frame);
                        }
                        Ok(frame) => send_event(
                            events,
                            SourceEvent::Log(format!(
                                "discarded unmatched response type=0x{:02X} seq={}",
                                frame.message_type, frame.sequence
                            )),
                        ),
                        Err(error) => send_event(
                            events,
                            SourceEvent::Log(format!("discarded malformed frame: {error}")),
                        ),
                    }
                }
            }
            if attempt < MAX_RETRIES {
                send_event(
                    events,
                    SourceEvent::Log(format!(
                        "timeout for message 0x{message_type:02X}; retry {}",
                        attempt + 1
                    )),
                );
            }
        }
        bail!(
            "message 0x{message_type:02X} timed out after {} attempts",
            MAX_RETRIES + 1
        )
    }

    fn enumerate(&mut self, events: &mpsc::Sender<SourceEvent>) -> Result<()> {
        let mut start = 0_u16;
        let mut all = Vec::new();
        loop {
            let response = self.request(
                codec::message::ENUMERATE,
                &codec::enum_request(start, 8),
                events,
            )?;
            let (total, returned_start, page) = codec::parse_descriptors(&response.payload)?;
            if returned_start != start {
                bail!("descriptor page index mismatch");
            }
            let count = page.len();
            all.extend(page);
            start = start.saturating_add(count as u16);
            if count == 0 || start >= total {
                break;
            }
        }
        send_event(events, SourceEvent::Descriptors(all));
        Ok(())
    }

    fn handle_command(
        &mut self,
        command: SourceCommand,
        events: &mpsc::Sender<SourceEvent>,
    ) -> Result<bool> {
        match command {
            SourceCommand::Connect(_) => {
                send_event(events, SourceEvent::Error("already connected".to_owned()));
            }
            SourceCommand::Disconnect => return Ok(false),
            SourceCommand::WriteParams(writes) => {
                let values: Vec<_> = writes
                    .iter()
                    .map(|ParamWrite { var, value_bits }| (*var, *value_bits))
                    .collect();
                let response = self.request(
                    codec::message::CAL_WRITE,
                    &codec::cal_write_request(&values),
                    events,
                )?;
                require_ack(&response, codec::message::CAL_WRITE)?;
                send_event(events, SourceEvent::ParamsStaged);
            }
            SourceCommand::CommitParams => {
                let response = self.request(codec::message::CAL_COMMIT, &[], events)?;
                let ack = require_ack(&response, codec::message::CAL_COMMIT)?;
                send_event(events, SourceEvent::ParamsCommitted { sequence: ack.data });
            }
            SourceCommand::ReadValues { start, count } => {
                let response = self.request(
                    codec::message::CAL_READ,
                    &codec::cal_read_request(start, count),
                    events,
                )?;
                let (mirror_sequence, start, values) = codec::parse_cal_values(&response.payload)?;
                send_event(
                    events,
                    SourceEvent::Values {
                        mirror_sequence,
                        start,
                        values,
                    },
                );
            }
            SourceCommand::BindChannels { channels } => {
                let response = self.request(
                    codec::message::DAQ_BIND,
                    &codec::daq_bind_request(&channels),
                    events,
                )?;
                let ack = require_ack(&response, codec::message::DAQ_BIND)?;
                self.expected_block_sequence = None;
                send_event(
                    events,
                    SourceEvent::ChannelsBound {
                        bind_sequence: ack.data as u16,
                    },
                );
            }
            SourceCommand::ConfigureScope(config) => {
                let response = self.request(
                    codec::message::DAQ_CONTROL,
                    &codec::daq_control_request(&config),
                    events,
                )?;
                require_ack(&response, codec::message::DAQ_CONTROL)?;
                self.expected_block_sequence = None;
                self.scope_active = config.mode != ScopeMode::Off;
                send_event(events, SourceEvent::ScopeConfigured { mode: config.mode });
            }
            SourceCommand::SystemCommand(command) => {
                let response = self.request(
                    codec::message::SYSTEM_COMMAND,
                    &codec::system_command_request(command),
                    events,
                )?;
                require_ack(&response, codec::message::SYSTEM_COMMAND)?;
            }
        }
        Ok(true)
    }

    fn poll_status(&mut self, events: &mpsc::Sender<SourceEvent>) -> Result<()> {
        let response = self.request(codec::message::STATUS, &codec::status_request(), events)?;
        let status = codec::parse_status(&response.payload)?;
        if status.build_hash != self.info.build_hash {
            let old_hash = self.info.build_hash;
            self.info.build_hash = status.build_hash;
            send_event(
                events,
                SourceEvent::DeviceChanged {
                    old_hash,
                    new_hash: status.build_hash,
                },
            );
            if self.info.has(CAP_ENUM) {
                self.enumerate(events)?;
            }
        }
        send_event(events, SourceEvent::Status(status));
        self.next_status = Instant::now() + STATUS_PERIOD;
        Ok(())
    }

    fn poll_blocks(&mut self, events: &mpsc::Sender<SourceEvent>) -> Result<()> {
        if !self.scope_active {
            self.next_block_poll = Instant::now() + BLOCK_POLL_PERIOD;
            return Ok(());
        }

        let response = self.request(
            codec::message::BLOCK_REQUEST,
            &codec::block_request(2),
            events,
        )?;
        let batch = codec::parse_block_batch(&response.payload)?;
        if batch.mode == ScopeMode::Off {
            self.scope_active = false;
        }
        if batch.mode == ScopeMode::Stream {
            for block in &batch.blocks {
                if let Some(expected) = self.expected_block_sequence
                    && block.block_seq != expected
                {
                    send_event(
                        events,
                        SourceEvent::StreamGap {
                            expected,
                            received: block.block_seq,
                        },
                    );
                }
                self.expected_block_sequence = Some(block.block_seq.wrapping_add(1));
            }
        } else {
            self.expected_block_sequence = None;
        }
        if batch.overrun_count != 0 {
            send_event(
                events,
                SourceEvent::Log(format!("scope producer overruns: {}", batch.overrun_count)),
            );
        }
        if !batch.blocks.is_empty() {
            send_event(
                events,
                SourceEvent::Blocks {
                    mode: batch.mode,
                    remaining_hint: batch.remaining_hint,
                    trigger_tick: batch.trigger_tick,
                    blocks: batch.blocks,
                },
            );
        }
        self.next_block_poll = if batch.remaining_hint != 0 {
            Instant::now()
        } else {
            Instant::now() + BLOCK_POLL_PERIOD
        };
        Ok(())
    }
}

fn worker(commands: mpsc::Receiver<SourceCommand>, events: mpsc::Sender<SourceEvent>) {
    let mut session: Option<Session> = None;
    loop {
        if session.is_none() {
            match commands.recv() {
                Ok(SourceCommand::Connect(endpoint)) => {
                    match Session::connect(&endpoint, &events) {
                        Ok(mut connected) => {
                            send_event(&events, SourceEvent::Connected(connected.info.clone()));
                            if connected.info.has(CAP_ENUM)
                                && let Err(error) = connected.enumerate(&events)
                            {
                                send_event(&events, SourceEvent::Error(error.to_string()));
                                send_event(&events, SourceEvent::Disconnected);
                            } else {
                                session = Some(connected);
                            }
                        }
                        Err(error) => send_event(&events, SourceEvent::Error(error.to_string())),
                    }
                }
                Ok(_) => send_event(
                    &events,
                    SourceEvent::Error("connect a transport first".to_owned()),
                ),
                Err(_) => break,
            }
            continue;
        }

        let connected = session.as_mut().expect("checked above");
        let result = match commands.try_recv() {
            Ok(command) => connected.handle_command(command, &events).map(|keep| {
                if keep {
                    WorkerStep::Continue
                } else {
                    WorkerStep::Disconnect
                }
            }),
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {
                let now = Instant::now();
                if now >= connected.next_status {
                    connected.poll_status(&events).map(|_| WorkerStep::Continue)
                } else if now >= connected.next_block_poll {
                    connected.poll_blocks(&events).map(|_| WorkerStep::Continue)
                } else {
                    thread::sleep(Duration::from_millis(1));
                    Ok(WorkerStep::Continue)
                }
            }
        };
        match result {
            Ok(WorkerStep::Continue) => {}
            Ok(WorkerStep::Disconnect) => {
                session = None;
                send_event(&events, SourceEvent::Disconnected);
            }
            Err(error) => {
                send_event(&events, SourceEvent::Error(error.to_string()));
                session = None;
                send_event(&events, SourceEvent::Disconnected);
            }
        }
    }
}

enum WorkerStep {
    Continue,
    Disconnect,
}

fn require_ack(frame: &Frame, request_type: u8) -> Result<codec::Ack> {
    let ack = codec::parse_ack(&frame.payload)?;
    if ack.echoed_type != request_type {
        bail!("ACK type mismatch");
    }
    if ack.status != 0 {
        bail!(
            "device rejected message 0x{request_type:02X}: status={}",
            ack.status
        );
    }
    Ok(ack)
}

fn validate_device_info(info: &DeviceInfo) -> Result<()> {
    if info.protocol_version != u16::from(codec::WIRE_VERSION) {
        bail!(
            "wire version mismatch: device={}, host={}",
            info.protocol_version,
            codec::WIRE_VERSION
        );
    }
    if info.contract_version != EXPECTED_CONTRACT_VERSION {
        bail!(
            "contract version mismatch: device={}, host={EXPECTED_CONTRACT_VERSION}",
            info.contract_version
        );
    }
    Ok(())
}

fn send_event(events: &mpsc::Sender<SourceEvent>, event: SourceEvent) {
    let _ = events.send(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScriptedTransport {
        reads: Vec<Vec<u8>>,
        writes: Vec<Vec<u8>>,
    }

    impl ByteTransport for ScriptedTransport {
        fn write_all(&mut self, data: &[u8]) -> Result<(), transport::TransportError> {
            self.writes.push(data.to_vec());
            Ok(())
        }

        fn read(&mut self, data: &mut [u8]) -> Result<usize, transport::TransportError> {
            if self.reads.is_empty() {
                return Ok(0);
            }
            let bytes = self.reads.remove(0);
            data[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        }

        fn label(&self) -> &str {
            "scripted"
        }
    }

    #[test]
    fn request_discards_wrong_sequence_before_match() {
        let wrong = codec::encode_frame(codec::message::HELLO | 0x80, 99, &[]);
        let expected = codec::encode_frame(codec::message::HELLO | 0x80, 1, &[]);
        let (event_tx, _event_rx) = mpsc::channel();
        let now = Instant::now();
        let mut session = Session {
            transport: Box::new(ScriptedTransport {
                reads: vec![wrong, expected],
                writes: Vec::new(),
            }),
            decoder: FrameDecoder::default(),
            next_sequence: 1,
            info: DeviceInfo {
                protocol_version: 2,
                contract_version: EXPECTED_CONTRACT_VERSION,
                build_hash: 0,
                descriptor_count: 0,
                firmware_name: String::new(),
                tick_hz: 0,
                capabilities: 0,
            },
            scope_active: false,
            expected_block_sequence: None,
            next_status: now,
            next_block_poll: now,
        };
        let frame = session
            .request(codec::message::HELLO, &[], &event_tx)
            .expect("matching response");
        assert_eq!(frame.sequence, 1);
    }

    #[test]
    fn incompatible_contract_is_rejected() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&2_u16.to_le_bytes());
        payload.extend_from_slice(&2_u16.to_le_bytes());
        payload.resize(36, 0);
        let info = codec::parse_hello(&payload).expect("parse old contract");
        assert!(validate_device_info(&info).is_err());
    }

    #[test]
    fn request_times_out_after_retries() {
        let (event_tx, _event_rx) = mpsc::channel();
        let now = Instant::now();
        let mut session = Session {
            transport: Box::new(ScriptedTransport {
                reads: Vec::new(),
                writes: Vec::new(),
            }),
            decoder: FrameDecoder::default(),
            next_sequence: 1,
            info: DeviceInfo {
                protocol_version: 2,
                contract_version: EXPECTED_CONTRACT_VERSION,
                build_hash: 0,
                descriptor_count: 0,
                firmware_name: String::new(),
                tick_hz: 0,
                capabilities: 0,
            },
            scope_active: false,
            expected_block_sequence: None,
            next_status: now,
            next_block_poll: now,
        };
        assert!(
            session
                .request(codec::message::STATUS, &[], &event_tx)
                .is_err()
        );
    }
}
