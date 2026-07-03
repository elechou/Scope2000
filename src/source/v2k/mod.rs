pub mod codec;
pub mod transport;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use self::codec::{Frame, FrameDecoder};
use self::transport::ByteTransport;
use crate::source::{
    CAP_ENUM, CatalogCommand, DataSource, DeviceInfo, DeviceStatus, ParamWrite, ScopeBlock,
    ScopeMode, SourceCommand, SourceEvent, SourceHandle, TransportEndpoint,
};

const EXPECTED_CONTRACT_VERSION: u16 = 14;
const ENUM_PAGE_SIZE: u8 = 8;
#[cfg(not(test))]
const REQUEST_TIMEOUT: Duration = Duration::from_millis(150);
#[cfg(test)]
const REQUEST_TIMEOUT: Duration = Duration::from_millis(5);
const MAX_RETRIES: usize = 2;
const CAPTURE_REPLAY_IDLE_DELAY: Duration = Duration::from_millis(20);
const CAPTURE_REPLAY_MAX_BLOCKS: u8 = 8;

/// The device answered with a non-zero ACK status. The link itself is
/// healthy, so callers must not tear down the session for this.
#[derive(Debug)]
struct DeviceNak {
    request_type: u8,
    status: u8,
}

impl std::fmt::Display for DeviceNak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "device rejected message 0x{:02X}: status={} ({})",
            self.request_type,
            self.status,
            ack_status_label(self.status)
        )
    }
}

impl std::error::Error for DeviceNak {}

#[derive(Debug)]
struct RequestTimeout {
    message_type: u8,
    attempts: usize,
}

impl std::fmt::Display for RequestTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "message 0x{:02X} timed out after {} attempts",
            self.message_type, self.attempts
        )
    }
}

impl std::error::Error for RequestTimeout {}

fn ack_status_label(status: u8) -> &'static str {
    match status {
        1 => "bad parameter",
        2 => "busy",
        3 => "bad state",
        4 => "unsupported",
        5 => "internal error",
        _ => "unknown",
    }
}

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
    expected_push_sequence: Option<u16>,
    active_bind_sequence: Option<u16>,
    capture: Option<CaptureAssembly>,
    completed_capture_id: Option<u16>,
    pending_build_hash: Option<u32>,
}

struct CaptureAssembly {
    capture_id: u16,
    total_blocks: u16,
    trigger_tick: u32,
    bind_sequence: Option<u16>,
    blocks: Vec<Option<ScopeBlock>>,
    received: usize,
    last_progress: Instant,
}

impl CaptureAssembly {
    fn new(
        capture_id: u16,
        total_blocks: u16,
        trigger_tick: u32,
        bind_sequence: Option<u16>,
    ) -> Self {
        Self {
            capture_id,
            total_blocks,
            trigger_tick,
            bind_sequence,
            blocks: vec![None; usize::from(total_blocks)],
            received: 0,
            last_progress: Instant::now(),
        }
    }

    fn update_metadata(&mut self, trigger_tick: u32, bind_sequence: Option<u16>) {
        self.trigger_tick = trigger_tick;
        if bind_sequence.is_some() {
            self.bind_sequence = bind_sequence;
        }
    }

    fn insert_batch(&mut self, batch: codec::CaptureBatch) -> Result<bool> {
        if batch.capture_id != self.capture_id || batch.total_blocks != self.total_blocks {
            bail!("capture batch metadata mismatch");
        }
        if usize::from(batch.first_block_index) + batch.blocks.len() > self.blocks.len() {
            bail!("capture batch extends past total_blocks");
        }
        if let Some(bind_sequence) = self.bind_sequence {
            for block in &batch.blocks {
                if block.bind_seq != bind_sequence {
                    bail!(
                        "capture batch bind sequence mismatch: expected {bind_sequence}, received {}",
                        block.bind_seq
                    );
                }
            }
        }
        self.trigger_tick = batch.trigger_tick;
        let mut progressed = false;
        for (offset, block) in batch.blocks.into_iter().enumerate() {
            let index = usize::from(batch.first_block_index) + offset;
            if self.blocks[index].is_none() {
                self.blocks[index] = Some(block);
                self.received += 1;
                progressed = true;
            }
        }
        if progressed || batch.remaining_hint == 0 {
            self.last_progress = Instant::now();
        }
        Ok(progressed)
    }

    fn is_complete(&self) -> bool {
        self.received == self.blocks.len()
    }

    fn first_missing_range(&self) -> Option<(u16, u8)> {
        let start = self.blocks.iter().position(Option::is_none)?;
        let mut count = 0_usize;
        for block in &self.blocks[start..] {
            if block.is_some() || count == usize::from(CAPTURE_REPLAY_MAX_BLOCKS) {
                break;
            }
            count += 1;
        }
        Some((start as u16, count as u8))
    }

    fn finish(mut self) -> Vec<ScopeBlock> {
        self.blocks
            .drain(..)
            .map(|block| block.expect("capture completion bitmap is full"))
            .collect()
    }
}

impl Session {
    fn connect(endpoint: &TransportEndpoint, events: &mpsc::Sender<SourceEvent>) -> Result<Self> {
        let transport = transport::open(endpoint)?;
        Self::connect_transport(transport, events)
    }

    fn connect_transport(
        transport: Box<dyn ByteTransport>,
        events: &mpsc::Sender<SourceEvent>,
    ) -> Result<Self> {
        let label = transport.label().to_owned();
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
                mcu_model: 0,
                scope_max_ch: 0,
                scope_block_ticks: 0,
                scope_ring_words: 0,
            },
            scope_active: false,
            expected_block_sequence: None,
            expected_push_sequence: None,
            active_bind_sequence: None,
            capture: None,
            completed_capture_id: None,
            pending_build_hash: None,
        };
        let hello = match session.request(codec::message::HELLO, &codec::hello_request(), events) {
            Ok(frame) => frame,
            Err(error) if error.is::<RequestTimeout>() => {
                bail!(
                    "Viewer2000 device did not answer HELLO; check device power, SCI cable, serial port, and baud rate"
                );
            }
            Err(error) => return Err(error),
        };
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
        let mut wire_version_mismatch = None;

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
                let mut response = None;
                for result in self.decoder.push(&read_buffer[..count]) {
                    match result {
                        Ok(frame)
                            if frame.sequence == sequence
                                && frame.message_type == expected_type =>
                        {
                            response = Some(frame);
                        }
                        Ok(frame) => self.handle_frame(frame, events)?,
                        Err(error) => {
                            if let codec::CodecError::VersionMismatch(device_magic) = &error {
                                wire_version_mismatch = Some(*device_magic);
                            }
                            send_event(
                                events,
                                SourceEvent::Log(format!("discarded malformed frame: {error}")),
                            );
                        }
                    }
                }
                if let Some(frame) = response {
                    return Ok(frame);
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
        if let Some(device_magic) = wire_version_mismatch {
            bail!(
                "firmware/Scope2000 wire version mismatch; reflash v{} firmware or use a matching Scope2000 build (device magic=0x{device_magic:02X}, host magic=0x{:02X})",
                codec::WIRE_VERSION,
                codec::VERSION_MAGIC
            );
        }
        Err(RequestTimeout {
            message_type,
            attempts: MAX_RETRIES + 1,
        }
        .into())
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
                // A rejected CAL_READ answers with an 8-octet ACK payload
                // instead of CAL_VALUES; a successful reply for a non-empty
                // read list is always longer than 8 octets.
                if response.payload.len() == 8
                    && let Ok(ack) = codec::parse_ack(&response.payload)
                    && ack.echoed_type == codec::message::CAL_READ
                    && ack.status != 0
                {
                    return Err(DeviceNak {
                        request_type: codec::message::CAL_READ,
                        status: ack.status,
                    }
                    .into());
                }
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
                self.active_bind_sequence = Some(ack.data as u16);
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
                if config.mode != ScopeMode::CaptureArmed {
                    self.capture = None;
                    self.completed_capture_id = None;
                }
                send_event(events, SourceEvent::ScopeConfigured { mode: config.mode });
            }
        }
        Ok(())
    }

    fn service_rx(&mut self, events: &mpsc::Sender<SourceEvent>) -> Result<()> {
        let mut read_buffer = [0_u8; 1024];
        let count = self.transport.read(&mut read_buffer)?;
        if count == 0 {
            return Ok(());
        }
        for result in self.decoder.push(&read_buffer[..count]) {
            match result {
                Ok(frame) => self.handle_frame(frame, events)?,
                Err(error) => send_event(
                    events,
                    SourceEvent::Log(format!("discarded malformed frame: {error}")),
                ),
            }
        }
        Ok(())
    }

    fn handle_frame(&mut self, frame: Frame, events: &mpsc::Sender<SourceEvent>) -> Result<()> {
        if self.info.protocol_version == 0 {
            return Ok(());
        }
        match frame.message_type {
            codec::message::STATUS_PUSH => self.handle_status_payload(&frame.payload, events),
            codec::message::SCOPE_BLOCK_PUSH => {
                if let Some(expected) = self.expected_push_sequence
                    && frame.sequence != expected
                {
                    send_event(
                        events,
                        SourceEvent::PushFrameGap {
                            expected,
                            received: frame.sequence,
                        },
                    );
                }
                self.expected_push_sequence = Some(frame.sequence.wrapping_add(1));
                let batch = codec::parse_block_batch(&frame.payload)?;
                self.handle_block_batch(batch, events)
            }
            codec::message::CAPTURE_BATCH_PUSH => {
                if let Some(expected) = self.expected_push_sequence
                    && frame.sequence != expected
                {
                    send_event(
                        events,
                        SourceEvent::PushFrameGap {
                            expected,
                            received: frame.sequence,
                        },
                    );
                }
                self.expected_push_sequence = Some(frame.sequence.wrapping_add(1));
                let batch = codec::parse_capture_batch(&frame.payload)?;
                self.handle_capture_batch(batch, events)
            }
            _ => {
                send_event(
                    events,
                    SourceEvent::Log(format!(
                        "discarded unmatched frame type=0x{:02X} seq={}",
                        frame.message_type, frame.sequence
                    )),
                );
                Ok(())
            }
        }
    }

    fn handle_status_payload(
        &mut self,
        payload: &[u8],
        events: &mpsc::Sender<SourceEvent>,
    ) -> Result<()> {
        let status = codec::parse_status(payload)?;
        if status.build_hash != self.info.build_hash && self.pending_build_hash.is_none() {
            self.scope_active = false;
            self.expected_block_sequence = None;
            self.expected_push_sequence = None;
            self.capture = None;
            self.completed_capture_id = None;
            self.pending_build_hash = Some(status.build_hash);
        }
        self.update_capture_from_status(&status);
        send_event(events, SourceEvent::Status(status));
        Ok(())
    }

    fn update_capture_from_status(&mut self, status: &DeviceStatus) {
        // After the host turns the scope off, status frames generated before
        // the device applied the mode change can still report CaptureFrozen;
        // they must not resurrect an abandoned capture assembly.
        if !self.scope_active {
            return;
        }
        if status.scope_mode != ScopeMode::CaptureFrozen || status.scope_frozen_count == 0 {
            return;
        }
        if self.completed_capture_id == Some(status.scope_state_seq) {
            return;
        }
        let bind_sequence = Some(status.scope_bind_seq);
        match &mut self.capture {
            Some(capture) if capture.capture_id == status.scope_state_seq => {
                capture.update_metadata(status.scope_trigger_tick, bind_sequence);
            }
            _ => {
                self.capture = Some(CaptureAssembly::new(
                    status.scope_state_seq,
                    status.scope_frozen_count,
                    status.scope_trigger_tick,
                    bind_sequence,
                ));
            }
        }
    }

    fn handle_block_batch(
        &mut self,
        batch: codec::BlockBatch,
        events: &mpsc::Sender<SourceEvent>,
    ) -> Result<()> {
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
                    mode: ScopeMode::Stream,
                    blocks: batch.blocks,
                },
            );
        }
        Ok(())
    }

    fn handle_capture_batch(
        &mut self,
        batch: codec::CaptureBatch,
        events: &mpsc::Sender<SourceEvent>,
    ) -> Result<()> {
        if !self.scope_active || batch.total_blocks == 0 {
            return Ok(());
        }
        if self.completed_capture_id == Some(batch.capture_id) {
            return Ok(());
        }
        let bind_sequence = self
            .active_bind_sequence
            .or_else(|| batch.blocks.first().map(|block| block.bind_seq));
        let capture_id = batch.capture_id;
        match &mut self.capture {
            Some(capture) if capture.capture_id == capture_id => {
                capture.update_metadata(batch.trigger_tick, bind_sequence);
            }
            _ => {
                self.capture = Some(CaptureAssembly::new(
                    capture_id,
                    batch.total_blocks,
                    batch.trigger_tick,
                    bind_sequence,
                ));
            }
        }
        let capture = self.capture.as_mut().expect("capture assembly exists");
        let is_replay = batch.is_replay;
        let progressed = capture.insert_batch(batch)?;
        if is_replay && progressed {
            send_event(
                events,
                SourceEvent::Log("capture replay filled missing block(s)".to_owned()),
            );
        }
        if capture.is_complete() {
            let capture = self.capture.take().expect("capture complete");
            self.completed_capture_id = Some(capture.capture_id);
            send_event(
                events,
                SourceEvent::CaptureFrame {
                    capture_id: capture.capture_id,
                    trigger_tick: capture.trigger_tick,
                    blocks: capture.finish(),
                },
            );
        }
        Ok(())
    }

    fn service_capture_replay(&mut self, events: &mpsc::Sender<SourceEvent>) -> Result<()> {
        let request = self.capture.as_mut().and_then(|capture| {
            if capture.is_complete() || capture.last_progress.elapsed() < CAPTURE_REPLAY_IDLE_DELAY
            {
                return None;
            }
            let (first, max_blocks) = capture.first_missing_range()?;
            capture.last_progress = Instant::now();
            Some((capture.capture_id, first, max_blocks))
        });
        let Some((capture_id, first_block_index, max_blocks)) = request else {
            return Ok(());
        };
        let response = self.request(
            codec::message::CAPTURE_REPLAY,
            &codec::capture_replay_request(capture_id, first_block_index, max_blocks),
            events,
        )?;
        match require_ack(&response, codec::message::CAPTURE_REPLAY) {
            Ok(_) => send_event(
                events,
                SourceEvent::Log(format!(
                    "requested capture replay: id={capture_id} first={first_block_index} max={max_blocks}"
                )),
            ),
            // The device no longer holds this capture (scope stopped,
            // re-armed, or superseded); drop the partial assembly and mark
            // the id as done so stale status frames cannot recreate it.
            Err(error) if error.is::<DeviceNak>() => {
                self.capture = None;
                self.completed_capture_id = Some(capture_id);
                send_event(
                    events,
                    SourceEvent::Log(format!(
                        "abandoned incomplete capture {capture_id}: {error}"
                    )),
                );
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn refresh_device_if_needed(&mut self, events: &mpsc::Sender<SourceEvent>) -> Result<()> {
        let Some(status_hash) = self.pending_build_hash.take() else {
            return Ok(());
        };
        let old_hash = self.info.build_hash;
        let hello = self.request(codec::message::HELLO, &codec::hello_request(), events)?;
        let info = codec::parse_hello(&hello.payload)?;
        validate_device_info(&info)?;
        if info.build_hash != status_hash {
            bail!(
                "firmware changed while refreshing device information: STATUS=0x{status_hash:08X}, HELLO=0x{:08X}",
                info.build_hash
            );
        }
        self.info = info.clone();
        self.pending_build_hash = None;
        send_event(events, SourceEvent::DeviceChanged { old_hash, info });
        if self.info.has(CAP_ENUM) {
            self.enumerate(events)?;
        }
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
                let result = connected
                    .service_rx(&events)
                    .and_then(|_| connected.refresh_device_if_needed(&events))
                    .and_then(|_| connected.service_capture_replay(&events))
                    .map(|_| WorkerStep::Continue);
                thread::sleep(Duration::from_millis(1));
                result
            }
        };
        match result {
            Ok(WorkerStep::Continue) => {}
            Ok(WorkerStep::Disconnect) => {
                session = None;
                send_event(&events, SourceEvent::Disconnected);
            }
            Ok(WorkerStep::Shutdown) => break,
            // A NAK is a well-formed answer over a healthy link; report it
            // and keep the session instead of forcing a reconnect.
            Err(error) if error.is::<DeviceNak>() => {
                send_event(&events, SourceEvent::Error(error.to_string()));
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
    Shutdown,
}

fn require_ack(frame: &Frame, request_type: u8) -> Result<codec::Ack> {
    let ack = codec::parse_ack(&frame.payload)?;
    if ack.echoed_type != request_type {
        bail!("ACK type mismatch");
    }
    if ack.status != 0 {
        return Err(DeviceNak {
            request_type,
            status: ack.status,
        }
        .into());
    }
    Ok(ack)
}

fn validate_device_info(info: &DeviceInfo) -> Result<()> {
    if info.protocol_version != u16::from(codec::WIRE_VERSION) {
        bail!(
            "firmware/Scope2000 wire version mismatch; reflash v10 firmware or use a matching Scope2000 build (device={}, host={})",
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
    if info.scope_max_ch == 0 {
        bail!("HELLO scope_max_ch must be nonzero");
    }
    if info.scope_block_ticks == 0 {
        bail!("HELLO scope_block_ticks must be nonzero");
    }
    if info.scope_ring_words == 0 {
        bail!("HELLO scope_ring_words must be nonzero");
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
            mcu_model: 1,
            scope_max_ch: 16,
            scope_block_ticks: 10,
            scope_ring_words: 0x7000,
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
            expected_push_sequence: None,
            active_bind_sequence: None,
            capture: None,
            completed_capture_id: None,
            pending_build_hash: None,
        }
    }

    fn encode_frame_with_magic(
        version_magic: u8,
        message_type: u8,
        sequence: u16,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut raw = Vec::with_capacity(11 + payload.len());
        raw.push(version_magic);
        raw.push(message_type);
        raw.push(0);
        raw.extend_from_slice(&sequence.to_le_bytes());
        raw.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        raw.extend_from_slice(payload);
        raw.extend_from_slice(&codec::crc32c(&raw).to_le_bytes());
        let mut wire = codec::cobs_encode(&raw);
        wire.push(0);
        wire
    }

    fn enum_payload(total: u16, start: u16, count: u8) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&total.to_le_bytes());
        payload.extend_from_slice(&start.to_le_bytes());
        payload.extend_from_slice(&[count, 0]);
        for index in start..start + u16::from(count) {
            let name = format!("var{index:03}");
            payload.extend_from_slice(&(0xB000_u32 + u32::from(index) * 2).to_le_bytes());
            payload.extend_from_slice(&(VarType::F32 as u16).to_le_bytes());
            payload.extend_from_slice(&0x0003_u16.to_le_bytes());
            payload.extend_from_slice(&1_u16.to_le_bytes());
            payload.push(name.len() as u8);
            payload.push(0);
            payload.extend_from_slice(name.as_bytes());
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
        payload.extend_from_slice(&info.mcu_model.to_le_bytes());
        payload.extend_from_slice(&info.scope_max_ch.to_le_bytes());
        payload.extend_from_slice(&info.scope_block_ticks.to_le_bytes());
        payload.extend_from_slice(&0_u16.to_le_bytes());
        payload.extend_from_slice(&info.scope_ring_words.to_le_bytes());
        payload
    }

    fn status_payload(build_hash: u32) -> Vec<u8> {
        let mut payload = vec![0_u8; 96];
        payload[..2].copy_from_slice(&1_u16.to_le_bytes());
        payload[26..30].copy_from_slice(&build_hash.to_le_bytes());
        payload
    }

    fn ack_payload(echoed_type: u8, data: u32) -> Vec<u8> {
        let mut payload = vec![0_u8, echoed_type, 0, 0];
        payload.extend_from_slice(&data.to_le_bytes());
        payload
    }

    fn nak_payload(echoed_type: u8, status: u8) -> Vec<u8> {
        let mut payload = ack_payload(echoed_type, 0);
        payload[0] = status;
        payload
    }

    fn scope_block(block_seq: u16, bind_seq: u16) -> ScopeBlock {
        ScopeBlock {
            start_tick: u32::from(block_seq) * 10,
            block_seq,
            flags: 0,
            sample_count: 1,
            channel_count: 1,
            bind_seq,
            stride_octets: 4,
            samples: (block_seq as f32).to_le_bytes().to_vec(),
        }
    }

    fn capture_batch(
        capture_id: u16,
        total_blocks: u16,
        first_block_index: u16,
        blocks: Vec<ScopeBlock>,
        is_replay: bool,
        remaining_hint: u16,
    ) -> codec::CaptureBatch {
        codec::CaptureBatch {
            capture_id,
            total_blocks,
            first_block_index,
            is_replay,
            remaining_hint,
            trigger_tick: 1234,
            blocks,
        }
    }

    fn frozen_status(capture_id: u16, total_blocks: u16) -> DeviceStatus {
        DeviceStatus {
            system_state: crate::source::SystemState::Running,
            fault_code: 0,
            status_flags: 0,
            tick: 0,
            cpu1_heartbeat: 0,
            cpu2_heartbeat: 0,
            applied_seq: 0,
            calibration_result: 0,
            calibration_fail_index: 0,
            build_hash: 0,
            scope_mode: ScopeMode::CaptureFrozen,
            scope_flags: 0,
            command_ack_seq: None,
            command_result: None,
            performance: None,
            scope_state_seq: capture_id,
            scope_frozen_count: total_blocks,
            scope_trigger_tick: 1234,
            scope_bind_seq: 3,
        }
    }

    #[test]
    fn capture_assembly_completes_out_of_order_and_ignores_duplicates() {
        let mut capture = CaptureAssembly::new(22, 3, 1234, Some(3));

        assert!(
            capture
                .insert_batch(capture_batch(22, 3, 2, vec![scope_block(12, 3)], false, 0))
                .expect("insert last")
        );
        assert!(
            capture
                .insert_batch(capture_batch(22, 3, 0, vec![scope_block(10, 3)], false, 1))
                .expect("insert first")
        );
        assert_eq!(capture.first_missing_range(), Some((1, 1)));
        assert!(
            !capture
                .insert_batch(capture_batch(22, 3, 2, vec![scope_block(12, 3)], true, 0))
                .expect("duplicate replay")
        );
        assert!(
            capture
                .insert_batch(capture_batch(22, 3, 1, vec![scope_block(11, 3)], true, 0))
                .expect("insert replay")
        );

        assert!(capture.is_complete());
        let blocks = capture.finish();
        assert_eq!(
            blocks
                .iter()
                .map(|block| block.block_seq)
                .collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
    }

    #[test]
    fn capture_assembly_accepts_wrapped_block_sequence() {
        let mut capture = CaptureAssembly::new(22, 4, 1234, Some(3));

        assert!(
            capture
                .insert_batch(capture_batch(
                    22,
                    4,
                    0,
                    vec![
                        scope_block(0xFFFE, 3),
                        scope_block(0xFFFF, 3),
                        scope_block(0, 3),
                        scope_block(1, 3),
                    ],
                    false,
                    0,
                ))
                .expect("insert wrapped block sequence")
        );

        assert!(capture.is_complete());
        let blocks = capture.finish();
        assert_eq!(
            blocks
                .iter()
                .map(|block| block.block_seq)
                .collect::<Vec<_>>(),
            vec![0xFFFE, 0xFFFF, 0, 1]
        );
    }

    fn capture_push_payload(
        capture_id: u16,
        total_blocks: u16,
        first_block_index: u16,
        block: &ScopeBlock,
        is_replay: bool,
        remaining_hint: u16,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&capture_id.to_le_bytes());
        payload.extend_from_slice(&total_blocks.to_le_bytes());
        payload.extend_from_slice(&first_block_index.to_le_bytes());
        payload.push(1);
        payload.push(if is_replay { 1 } else { 0 });
        payload.extend_from_slice(&remaining_hint.to_le_bytes());
        payload.extend_from_slice(&0_u16.to_le_bytes());
        payload.extend_from_slice(&1234_u32.to_le_bytes());
        payload.extend_from_slice(&block.start_tick.to_le_bytes());
        payload.extend_from_slice(&block.block_seq.to_le_bytes());
        payload.extend_from_slice(&block.flags.to_le_bytes());
        payload.extend_from_slice(&block.sample_count.to_le_bytes());
        payload.extend_from_slice(&block.channel_count.to_le_bytes());
        payload.extend_from_slice(&block.bind_seq.to_le_bytes());
        payload.extend_from_slice(&block.stride_octets.to_le_bytes());
        payload.extend_from_slice(&block.samples);
        payload
    }

    #[test]
    fn request_drains_push_frames_after_matching_response() {
        let ack = codec::encode_frame(
            codec::message::CAPTURE_REPLAY | 0x80,
            1,
            &ack_payload(codec::message::CAPTURE_REPLAY, 22),
        );
        let push = codec::encode_frame(
            codec::message::CAPTURE_BATCH_PUSH,
            7,
            &capture_push_payload(22, 1, 0, &scope_block(99, 3), true, 0),
        );
        let mut read = ack;
        read.extend_from_slice(&push);
        let (event_tx, event_rx) = mpsc::channel();
        let mut session = session(vec![read], device_info(0, 0, 0));
        session.scope_active = true;
        session.capture = Some(CaptureAssembly::new(22, 1, 1234, Some(3)));

        let response = session
            .request(
                codec::message::CAPTURE_REPLAY,
                &codec::capture_replay_request(22, 0, 1),
                &event_tx,
            )
            .expect("replay ACK");
        let ack = require_ack(&response, codec::message::CAPTURE_REPLAY).expect("ACK payload");

        assert_eq!(ack.data, 22);
        assert!(session.capture.is_none());
        let mut saw_capture = false;
        while let Ok(event) = event_rx.try_recv() {
            if let SourceEvent::CaptureFrame {
                capture_id, blocks, ..
            } = event
            {
                assert_eq!(capture_id, 22);
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].block_seq, 99);
                saw_capture = true;
            }
        }
        assert!(saw_capture);
    }

    #[test]
    fn missing_first_capture_batch_recovers_from_status_metadata() {
        let mut session = session(Vec::new(), device_info(0, 0, 0));
        session.scope_active = true;

        session.update_capture_from_status(&frozen_status(22, 4));

        let capture = session.capture.as_ref().expect("capture from status");
        assert_eq!(capture.capture_id, 22);
        assert_eq!(capture.total_blocks, 4);
        assert_eq!(capture.first_missing_range(), Some((0, 4)));

        session.capture = None;
        session.completed_capture_id = Some(22);
        session.update_capture_from_status(&frozen_status(22, 4));
        assert!(session.capture.is_none());
    }

    #[test]
    fn stale_frozen_status_after_scope_off_is_ignored() {
        let (event_tx, _event_rx) = mpsc::channel();
        let mut session = session(Vec::new(), device_info(0, 0, 0));
        assert!(!session.scope_active);

        session.update_capture_from_status(&frozen_status(22, 4));
        assert!(session.capture.is_none());

        session
            .handle_capture_batch(
                capture_batch(22, 4, 0, vec![scope_block(10, 3)], false, 3),
                &event_tx,
            )
            .expect("ignore capture batch while scope is off");
        assert!(session.capture.is_none());
    }

    #[test]
    fn capture_replay_nak_abandons_capture_without_error() {
        let nak = codec::encode_frame(
            codec::message::CAPTURE_REPLAY | 0x80,
            1,
            &nak_payload(codec::message::CAPTURE_REPLAY, 3),
        );
        let (event_tx, event_rx) = mpsc::channel();
        let mut session = session(vec![nak], device_info(0, 0, 0));
        session.scope_active = true;
        let mut capture = CaptureAssembly::new(22, 2, 1234, Some(3));
        capture.last_progress = Instant::now() - CAPTURE_REPLAY_IDLE_DELAY;
        session.capture = Some(capture);

        session
            .service_capture_replay(&event_tx)
            .expect("replay NAK is recoverable");

        assert!(session.capture.is_none());
        assert_eq!(session.completed_capture_id, Some(22));
        let mut saw_abandon = false;
        while let Ok(event) = event_rx.try_recv() {
            if let SourceEvent::Log(message) = event
                && message.contains("abandoned incomplete capture 22")
            {
                saw_abandon = true;
            }
        }
        assert!(saw_abandon);
    }

    #[test]
    fn cal_read_nak_is_reported_as_device_nak() {
        let response = codec::encode_frame(
            codec::message::CAL_READ | 0x80,
            1,
            &nak_payload(codec::message::CAL_READ, 5),
        );
        let (event_tx, _event_rx) = mpsc::channel();
        let mut session = session(vec![response], device_info(0, 1, CAP_ENUM));
        let command = SourceCommand::Catalog {
            build_hash: 0,
            command: CatalogCommand::ReadValues(vec![ValueRead {
                descriptor_index: 0,
                var: VarRef {
                    addr: 0xB000,
                    ty: VarType::F32,
                },
            }]),
        };

        let error = session
            .handle_command(command, &event_tx)
            .expect_err("CAL_READ NAK surfaces as an error");
        assert!(error.is::<DeviceNak>());
    }

    #[test]
    fn require_ack_rejection_is_downcastable_to_device_nak() {
        let frame = Frame {
            message_type: codec::message::DAQ_CONTROL | 0x80,
            sequence: 1,
            payload: nak_payload(codec::message::DAQ_CONTROL, 2),
        };

        let error = require_ack(&frame, codec::message::DAQ_CONTROL).expect_err("NAK");

        assert!(error.is::<DeviceNak>());
        assert_eq!(
            error.to_string(),
            "device rejected message 0x20: status=2 (busy)"
        );
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
    fn request_reports_valid_wrong_wire_magic_as_version_mismatch() {
        let response = encode_frame_with_magic(
            codec::VERSION_MAGIC - 1,
            codec::message::HELLO | 0x80,
            1,
            &[],
        );
        let (event_tx, _event_rx) = mpsc::channel();
        let mut session = session(vec![response], device_info(0, 0, 0));

        let error = session
            .request(codec::message::HELLO, &[], &event_tx)
            .expect_err("wire mismatch should be reported");
        let message = error.to_string();

        assert!(message.contains("wire version mismatch"), "{message}");
        assert!(message.contains("device magic=0x59"), "{message}");
    }

    #[test]
    fn connect_without_hello_reports_device_not_answering() {
        let (event_tx, _event_rx) = mpsc::channel();
        let transport = Box::new(ScriptedTransport {
            reads: Vec::new(),
            writes: Vec::new(),
        });

        let error = match Session::connect_transport(transport, &event_tx) {
            Ok(_) => panic!("connection should fail without a HELLO response"),
            Err(error) => error,
        };
        let message = error.to_string();

        assert!(message.contains("did not answer HELLO"), "{message}");
        assert!(!message.contains("wire version mismatch"), "{message}");
    }

    #[test]
    fn incompatible_contract_is_rejected() {
        let mut source = device_info(0, 0, 0);
        source.contract_version = 9;
        let payload = hello_payload(&source);
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
            codec::message::STATUS_PUSH,
            0,
            &status_payload(new_info.build_hash),
        );
        let hello = codec::encode_frame(codec::message::HELLO | 0x80, 1, &hello_payload(&new_info));
        let enumeration =
            codec::encode_frame(codec::message::ENUMERATE | 0x80, 2, &enum_payload(1, 0, 1));
        let confirmation =
            codec::encode_frame(codec::message::HELLO | 0x80, 3, &hello_payload(&new_info));
        let (event_tx, event_rx) = mpsc::channel();
        let mut session = session(
            vec![status, hello, enumeration, confirmation],
            device_info(old_hash, 4, CAP_ENUM),
        );
        session.scope_active = true;
        session.expected_block_sequence = Some(7);

        session.service_rx(&event_tx).expect("receive status push");
        session
            .refresh_device_if_needed(&event_tx)
            .expect("refresh session");

        assert_eq!(session.info.build_hash, new_info.build_hash);
        assert!(!session.scope_active);
        assert_eq!(session.expected_block_sequence, None);
        assert!(matches!(
            event_rx.recv().expect("status event"),
            SourceEvent::Status(_)
        ));
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
        let error = session
            .request(codec::message::HELLO, &[], &event_tx)
            .expect_err("request should time out");

        assert!(error.is::<RequestTimeout>());
    }
}
