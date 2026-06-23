pub mod codec;
pub mod transport;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use self::codec::{Frame, FrameDecoder};
use self::transport::ByteTransport;
use crate::source::{
    CAP_ENUM, CatalogCommand, DataSource, DeviceInfo, ParamWrite, ScopeMode, SourceCommand,
    SourceEvent, SourceHandle, TransportEndpoint,
};

const EXPECTED_CONTRACT_VERSION: u16 = 13;
const ENUM_PAGE_SIZE: u8 = 8;
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
        let worker = thread::Builder::new()
            .name("v2k-source".into())
            .spawn(move || worker(command_rx, event_tx))
            .expect("spawn V2kSource worker");
        SourceHandle::new(command_tx, event_rx, worker)
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
                project_name: String::new(),
                build_time_utc: 0,
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
        let expected_total = self.info.descriptor_count;
        let mut start = 0_u16;
        let mut all = Vec::new();
        loop {
            let response = self.request(
                codec::message::ENUMERATE,
                &codec::enum_request(start, ENUM_PAGE_SIZE),
                events,
            )?;
            let (total, returned_start, page) = codec::parse_descriptors(&response.payload)?;
            if total != expected_total {
                bail!(
                    "descriptor total changed during enumeration: HELLO={expected_total}, ENUM={total}"
                );
            }
            if returned_start != start {
                bail!(
                    "descriptor page index mismatch: requested={start}, returned={returned_start}"
                );
            }
            let count = page.len();
            if count > usize::from(ENUM_PAGE_SIZE) {
                bail!("descriptor page exceeds requested size");
            }
            if count == 0 && start < expected_total {
                bail!("descriptor enumeration ended before the advertised total");
            }
            let next = start
                .checked_add(u16::try_from(count).context("descriptor page too large")?)
                .context("descriptor index overflow")?;
            if next > expected_total {
                bail!("descriptor page extends beyond the advertised total");
            }
            all.extend(page);
            start = next;
            if start == expected_total {
                break;
            }
        }
        if all.len() != usize::from(expected_total) {
            bail!("descriptor enumeration count mismatch");
        }
        let hello = self.request(codec::message::HELLO, &codec::hello_request(), events)?;
        let confirmed = codec::parse_hello(&hello.payload)?;
        validate_device_info(&confirmed)?;
        if confirmed.build_hash != self.info.build_hash
            || confirmed.descriptor_count != expected_total
        {
            bail!(
                "firmware changed during descriptor enumeration: before=0x{:08X}/{expected_total}, after=0x{:08X}/{}",
                self.info.build_hash,
                confirmed.build_hash,
                confirmed.descriptor_count
            );
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
            SourceCommand::Shutdown => return Ok(false),
            SourceCommand::Catalog {
                build_hash,
                command,
            } => {
                if build_hash != self.info.build_hash {
                    send_event(
                        events,
                        SourceEvent::Log(format!(
                            "discarded stale catalog command for build 0x{build_hash:08X}; current build is 0x{:08X}",
                            self.info.build_hash
                        )),
                    );
                } else {
                    self.handle_catalog_command(command, events)?;
                }
            }
            SourceCommand::SystemCommand(command) => {
                let response = self.request(
                    codec::message::SYSTEM_COMMAND,
                    &codec::system_command_request(command),
                    events,
                )?;
                let ack = require_ack(&response, codec::message::SYSTEM_COMMAND)?;
                send_event(
                    events,
                    SourceEvent::SystemCommandAccepted {
                        command,
                        sequence: ack.data,
                    },
                );
            }
        }
        Ok(true)
    }

    fn handle_catalog_command(
        &mut self,
        command: CatalogCommand,
        events: &mpsc::Sender<SourceEvent>,
    ) -> Result<()> {
        match command {
            CatalogCommand::WriteParams(writes) => {
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
            CatalogCommand::CommitParams => {
                let response = self.request(codec::message::CAL_COMMIT, &[], events)?;
                let ack = require_ack(&response, codec::message::CAL_COMMIT)?;
                send_event(events, SourceEvent::ParamsCommitted { sequence: ack.data });
            }
            CatalogCommand::ReadValues(reads) => {
                let vars: Vec<_> = reads.iter().map(|read| read.var).collect();
                let response = self.request(
                    codec::message::CAL_READ,
                    &codec::cal_read_request(&vars),
                    events,
                )?;
                let (read_sequence, values) = codec::parse_cal_values(&response.payload)?;
                if values.len() != reads.len() {
                    bail!("CAL_READ response count mismatch");
                }
                send_event(
                    events,
                    SourceEvent::Values {
                        read_sequence,
                        indexes: reads.iter().map(|read| read.descriptor_index).collect(),
                        values,
                    },
                );
            }
            CatalogCommand::BindChannels { channels } => {
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
            CatalogCommand::ConfigureScope(config) => {
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
        }
        Ok(())
    }

    fn poll_status(&mut self, events: &mpsc::Sender<SourceEvent>) -> Result<()> {
        let response = self.request(codec::message::STATUS, &codec::status_request(), events)?;
        let status = codec::parse_status(&response.payload)?;
        if status.build_hash != self.info.build_hash {
            let old_hash = self.info.build_hash;
            self.scope_active = false;
            self.expected_block_sequence = None;
            let hello = self.request(codec::message::HELLO, &codec::hello_request(), events)?;
            let info = codec::parse_hello(&hello.payload)?;
            validate_device_info(&info)?;
            if info.build_hash != status.build_hash {
                bail!(
                    "firmware changed while refreshing device information: STATUS=0x{:08X}, HELLO=0x{:08X}",
                    status.build_hash,
                    info.build_hash
                );
            }
            self.info = info.clone();
            send_event(events, SourceEvent::DeviceChanged { old_hash, info });
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
                Ok(SourceCommand::Shutdown) => break,
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
            Ok(SourceCommand::Shutdown) => Ok(WorkerStep::Shutdown),
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
            Ok(WorkerStep::Shutdown) => break,
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
    Shutdown,
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
    if info.project_name.is_empty() {
        bail!(
            "HELLO project_name is empty; rebuild with the current Viewer2000 baker (use \"untitled\" for an unnamed demo)"
        );
    }
    if info.project_name.trim() != info.project_name {
        bail!("HELLO project_name contains leading or trailing whitespace");
    }
    if info.project_name.len() > 32 {
        bail!("HELLO project_name exceeds the 32-byte contract limit");
    }
    if !info
        .project_name
        .as_bytes()
        .iter()
        .all(|value| (0x20..=0x7e).contains(value))
    {
        bail!("HELLO project_name must contain printable ASCII only");
    }
    Ok(())
}

fn send_event(events: &mpsc::Sender<SourceEvent>, event: SourceEvent) {
    let _ = events.send(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{DataSource, SystemCommand, ValueRead, VarRef, VarType};

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

    fn device_info(build_hash: u32, descriptor_count: u16, capabilities: u32) -> DeviceInfo {
        DeviceInfo {
            protocol_version: u16::from(codec::WIRE_VERSION),
            contract_version: EXPECTED_CONTRACT_VERSION,
            build_hash,
            descriptor_count,
            firmware_name: "viewer2000".to_owned(),
            tick_hz: 20_000,
            capabilities,
            project_name: "phase4-demo".to_owned(),
            build_time_utc: 1_781_913_600,
        }
    }

    #[test]
    fn current_contract_requires_a_valid_project_identity() {
        let mut info = device_info(1, 0, 0);
        info.project_name.clear();
        assert!(
            validate_device_info(&info)
                .unwrap_err()
                .to_string()
                .contains("use \"untitled\"")
        );

        info.project_name = "untitled".to_owned();
        assert!(validate_device_info(&info).is_ok());

        info.project_name = "电机".to_owned();
        assert!(
            validate_device_info(&info)
                .unwrap_err()
                .to_string()
                .contains("printable ASCII")
        );

        info.project_name = " demo ".to_owned();
        assert!(
            validate_device_info(&info)
                .unwrap_err()
                .to_string()
                .contains("whitespace")
        );
    }

    #[test]
    fn spawned_source_shutdown_exits_worker() {
        let mut source = Box::new(V2kSource).spawn();

        source.shutdown();
    }

    fn session(reads: Vec<Vec<u8>>, info: DeviceInfo) -> Session {
        let now = Instant::now();
        Session {
            transport: Box::new(ScriptedTransport {
                reads,
                writes: Vec::new(),
            }),
            decoder: FrameDecoder::default(),
            next_sequence: 1,
            info,
            scope_active: false,
            expected_block_sequence: None,
            next_status: now,
            next_block_poll: now,
        }
    }

    fn enum_payload(total: u16, start: u16, count: u8) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&total.to_le_bytes());
        payload.extend_from_slice(&start.to_le_bytes());
        payload.extend_from_slice(&[count, 0]);
        for index in start..start + u16::from(count) {
            let name = format!("var{index:03}");
            let mut entry = [0_u8; 28];
            entry[..name.len()].copy_from_slice(name.as_bytes());
            entry[16..18].copy_from_slice(&(VarType::F32 as u16).to_le_bytes());
            entry[18..20].copy_from_slice(&0x0003_u16.to_le_bytes());
            entry[20..24].copy_from_slice(&(0xB000_u32 + u32::from(index) * 2).to_le_bytes());
            entry[24..26].copy_from_slice(&1_u16.to_le_bytes());
            payload.extend_from_slice(&entry);
        }
        payload
    }

    fn hello_payload(info: &DeviceInfo) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&info.protocol_version.to_le_bytes());
        payload.extend_from_slice(&info.contract_version.to_le_bytes());
        payload.extend_from_slice(&info.build_hash.to_le_bytes());
        payload.extend_from_slice(&info.descriptor_count.to_le_bytes());
        payload.extend_from_slice(&0_u16.to_le_bytes());
        let mut name = [0_u8; 16];
        let name_len = info.firmware_name.len().min(name.len());
        name[..name_len].copy_from_slice(&info.firmware_name.as_bytes()[..name_len]);
        payload.extend_from_slice(&name);
        payload.extend_from_slice(&info.tick_hz.to_le_bytes());
        payload.extend_from_slice(&info.capabilities.to_le_bytes());
        let mut project = [0_u8; 32];
        let project_len = info.project_name.len().min(project.len());
        project[..project_len].copy_from_slice(&info.project_name.as_bytes()[..project_len]);
        payload.extend_from_slice(&project);
        payload.extend_from_slice(&info.build_time_utc.to_le_bytes());
        payload
    }

    fn status_payload(build_hash: u32) -> Vec<u8> {
        let mut payload = vec![0_u8; 84];
        payload[..2].copy_from_slice(&1_u16.to_le_bytes());
        payload[26..30].copy_from_slice(&build_hash.to_le_bytes());
        payload
    }

    fn ack_payload(echoed_type: u8, data: u32) -> Vec<u8> {
        let mut payload = vec![0_u8, echoed_type, 0, 0];
        payload.extend_from_slice(&data.to_le_bytes());
        payload
    }

    #[test]
    fn request_discards_wrong_sequence_before_match() {
        let wrong = codec::encode_frame(codec::message::HELLO | 0x80, 99, &[]);
        let expected = codec::encode_frame(codec::message::HELLO | 0x80, 1, &[]);
        let (event_tx, _event_rx) = mpsc::channel();
        let mut session = session(vec![wrong, expected], device_info(0, 0, 0));
        let frame = session
            .request(codec::message::HELLO, &[], &event_tx)
            .expect("matching response");
        assert_eq!(frame.sequence, 1);
    }

    #[test]
    fn incompatible_contract_is_rejected() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&u16::from(codec::WIRE_VERSION).to_le_bytes());
        payload.extend_from_slice(&9_u16.to_le_bytes());
        payload.resize(36, 0);
        let info = codec::parse_hello(&payload).expect("parse old contract");
        assert!(validate_device_info(&info).is_err());
    }

    #[test]
    fn enumerates_full_176_entry_catalog() {
        let info = device_info(0x1234_5678, 176, CAP_ENUM);
        let mut reads: Vec<_> = (0_u16..176)
            .step_by(usize::from(ENUM_PAGE_SIZE))
            .enumerate()
            .map(|(page, start)| {
                codec::encode_frame(
                    codec::message::ENUMERATE | 0x80,
                    page as u16 + 1,
                    &enum_payload(176, start, ENUM_PAGE_SIZE),
                )
            })
            .collect();
        let confirm_sequence = reads.len() as u16 + 1;
        reads.push(codec::encode_frame(
            codec::message::HELLO | 0x80,
            confirm_sequence,
            &hello_payload(&info),
        ));
        let (event_tx, event_rx) = mpsc::channel();
        let mut session = session(reads, info);

        session.enumerate(&event_tx).expect("enumerate catalog");

        let SourceEvent::Descriptors(descriptors) = event_rx.recv().expect("descriptor event")
        else {
            panic!("expected descriptors");
        };
        assert_eq!(descriptors.len(), 176);
        assert_eq!(descriptors.first().expect("first").name, "var000");
        assert_eq!(descriptors.last().expect("last").name, "var175");
    }

    #[test]
    fn enumeration_rejects_total_that_differs_from_hello() {
        let response = codec::encode_frame(
            codec::message::ENUMERATE | 0x80,
            1,
            &enum_payload(127, 0, ENUM_PAGE_SIZE),
        );
        let (event_tx, _event_rx) = mpsc::channel();
        let mut session = session(vec![response], device_info(0, 176, CAP_ENUM));

        assert!(session.enumerate(&event_tx).is_err());
    }

    #[test]
    fn enumeration_rejects_wrong_start_and_premature_end() {
        let wrong_start =
            codec::encode_frame(codec::message::ENUMERATE | 0x80, 1, &enum_payload(8, 1, 7));
        let premature_end =
            codec::encode_frame(codec::message::ENUMERATE | 0x80, 1, &enum_payload(8, 0, 0));
        let (event_tx, _event_rx) = mpsc::channel();

        assert!(
            session(vec![wrong_start], device_info(0, 8, CAP_ENUM))
                .enumerate(&event_tx)
                .is_err()
        );
        assert!(
            session(vec![premature_end], device_info(0, 8, CAP_ENUM))
                .enumerate(&event_tx)
                .is_err()
        );
    }

    #[test]
    fn build_hash_change_refreshes_device_info_and_stops_scope() {
        let old_hash = 0x1111_1111;
        let new_info = device_info(0x2222_2222, 1, CAP_ENUM);
        let status = codec::encode_frame(
            codec::message::STATUS | 0x80,
            1,
            &status_payload(new_info.build_hash),
        );
        let hello = codec::encode_frame(codec::message::HELLO | 0x80, 2, &hello_payload(&new_info));
        let enumeration =
            codec::encode_frame(codec::message::ENUMERATE | 0x80, 3, &enum_payload(1, 0, 1));
        let confirmation =
            codec::encode_frame(codec::message::HELLO | 0x80, 4, &hello_payload(&new_info));
        let (event_tx, event_rx) = mpsc::channel();
        let mut session = session(
            vec![status, hello, enumeration, confirmation],
            device_info(old_hash, 4, CAP_ENUM),
        );
        session.scope_active = true;
        session.expected_block_sequence = Some(7);

        session.poll_status(&event_tx).expect("refresh session");

        assert_eq!(session.info.build_hash, new_info.build_hash);
        assert!(!session.scope_active);
        assert_eq!(session.expected_block_sequence, None);
        let SourceEvent::DeviceChanged {
            old_hash: event_old_hash,
            info,
        } = event_rx.recv().expect("device changed event")
        else {
            panic!("expected device change");
        };
        assert_eq!(event_old_hash, old_hash);
        assert_eq!(info.build_hash, new_info.build_hash);
        let SourceEvent::Descriptors(descriptors) = event_rx.recv().expect("descriptor event")
        else {
            panic!("expected descriptors");
        };
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].name, "var000");
        assert!(matches!(
            event_rx.recv().expect("status event"),
            SourceEvent::Status(_)
        ));
    }

    #[test]
    fn stale_catalog_command_is_rejected_without_transport_request() {
        let current_hash = 0x2222_2222;
        let (event_tx, event_rx) = mpsc::channel();
        let mut session = session(Vec::new(), device_info(current_hash, 1, CAP_ENUM));
        let command = SourceCommand::Catalog {
            build_hash: 0x1111_1111,
            command: CatalogCommand::ReadValues(vec![ValueRead {
                descriptor_index: 0,
                var: VarRef {
                    addr: 0xB000,
                    ty: VarType::F32,
                },
            }]),
        };

        assert!(
            session
                .handle_command(command, &event_tx)
                .expect("reject stale command")
        );
        let SourceEvent::Log(message) = event_rx.recv().expect("stale command log") else {
            panic!("expected log event");
        };
        assert!(message.contains("discarded stale catalog command"));
    }

    #[test]
    fn system_command_ack_data_is_emitted_as_sequence() {
        let response = codec::encode_frame(
            codec::message::SYSTEM_COMMAND | 0x80,
            1,
            &ack_payload(codec::message::SYSTEM_COMMAND, 42),
        );
        let (event_tx, event_rx) = mpsc::channel();
        let mut session = session(vec![response], device_info(0, 0, 0));

        assert!(
            session
                .handle_command(
                    SourceCommand::SystemCommand(SystemCommand::Start),
                    &event_tx,
                )
                .expect("system command accepted")
        );

        let SourceEvent::SystemCommandAccepted { command, sequence } =
            event_rx.recv().expect("system command accepted event")
        else {
            panic!("expected system command accepted event");
        };
        assert_eq!(command, SystemCommand::Start);
        assert_eq!(sequence, 42);
    }

    #[test]
    fn request_times_out_after_retries() {
        let (event_tx, _event_rx) = mpsc::channel();
        let mut session = session(Vec::new(), device_info(0, 0, 0));
        assert!(
            session
                .request(codec::message::STATUS, &[], &event_tx)
                .is_err()
        );
    }
}
