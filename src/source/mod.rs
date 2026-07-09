pub mod v2k;

use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum VarType {
    I16 = 0,
    U16 = 1,
    I32 = 2,
    U32 = 3,
    F32 = 4,
}

impl VarType {
    pub fn from_wire(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::I16),
            1 => Some(Self::U16),
            2 => Some(Self::I32),
            3 => Some(Self::U32),
            4 => Some(Self::F32),
            _ => None,
        }
    }

    pub fn wire_width(self) -> usize {
        match self {
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
        }
    }

    pub fn decode(self, bytes: &[u8]) -> Option<f64> {
        match self {
            Self::I16 => Some(i16::from_le_bytes(bytes.get(..2)?.try_into().ok()?) as f64),
            Self::U16 => Some(u16::from_le_bytes(bytes.get(..2)?.try_into().ok()?) as f64),
            Self::I32 => Some(i32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as f64),
            Self::U32 => Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as f64),
            Self::F32 => Some(f32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as f64),
        }
    }

    pub fn encode_value_bits(self, value: f64) -> u32 {
        match self {
            Self::I16 => (value as i16 as i32) as u32,
            Self::U16 => value as u16 as u32,
            Self::I32 => value as i32 as u32,
            Self::U32 => value as u32,
            Self::F32 => (value as f32).to_bits(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VarRef {
    pub addr: u32,
    pub ty: VarType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarDescriptor {
    pub name: String,
    pub var: VarRef,
    pub kind: u16,
    pub prescaler: u16,
}

impl VarDescriptor {
    pub fn is_parameter(&self) -> bool {
        self.kind & 0x0001 != 0
    }

    pub fn is_scope(&self) -> bool {
        self.kind & 0x0002 != 0
    }

    pub fn is_user(&self) -> bool {
        self.kind & 0x0004 != 0
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub protocol_version: u16,
    pub contract_version: u16,
    pub build_hash: u32,
    pub descriptor_count: u16,
    pub firmware_name: String,
    pub tick_hz: u32,
    pub capabilities: u32,
    pub project_name: String,
    pub build_time_utc: u32,
    pub mcu_model: u16,
    pub scope_max_ch: u16,
    pub scope_block_ticks: u16,
    pub scope_ring_words: u32,
}

impl DeviceInfo {
    pub fn has(&self, capability: u32) -> bool {
        self.capabilities & capability != 0
    }

    pub fn project_display_name(&self) -> &str {
        if self.project_name.is_empty() {
            &self.firmware_name
        } else {
            &self.project_name
        }
    }

    pub fn build_time_local_text(&self) -> Option<String> {
        if self.build_time_utc == 0 {
            return None;
        }
        chrono::DateTime::from_timestamp(i64::from(self.build_time_utc), 0)
            .map(|time| time.with_timezone(&chrono::Local))
            .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
    }

    pub fn build_time_display_text(&self) -> String {
        let built = self
            .build_time_local_text()
            .unwrap_or_else(|| "unknown".to_owned());
        format!("Built Time {built}")
    }

    pub fn mcu_model_label(&self) -> &'static str {
        match self.mcu_model {
            MCU_MODEL_UNKNOWN => "unknown",
            MCU_MODEL_F28P65X => "F28P65x",
            MCU_MODEL_F28379D => "F28379D",
            _ => "unknown",
        }
    }
}

pub const MCU_MODEL_UNKNOWN: u16 = 0;
pub const MCU_MODEL_F28P65X: u16 = 1;
pub const MCU_MODEL_F28379D: u16 = 2;

pub const CAP_ENUM: u32 = 1 << 0;
pub const CAP_CAL: u32 = 1 << 1;
pub const CAP_SCOPE_STREAM: u32 = 1 << 2;
pub const CAP_SCOPE_CAPTURE: u32 = 1 << 3;
pub const CAP_PRE_TRIGGER: u32 = 1 << 4;
pub const CAP_SYSTEM_CMD: u32 = 1 << 5;
pub const CAP_NATIVE_BLOCK: u32 = 1 << 6;
pub const CAP_CT_ZERO_CAL: u32 = 1 << 7;
pub const CAP_ABZ_ZEROING: u32 = 1 << 8;
pub const CAP_CAPTURE_FORCE: u32 = 1 << 9;
pub const DAQ_FLAG_TRIGGER_DISABLED: u16 = 1 << 0;
pub const CAL_READ_MAX: usize = 32;
pub const NO_CAPTURE_ACK: u16 = 0xFFFF;
pub const FAULT_USER_BASE: u16 = 256;
pub const FAULT_USER_OVER_SPEED: u16 = FAULT_USER_BASE + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeMode {
    #[default]
    Off,
    Stream,
    CaptureArmed,
    CapturePost,
    CaptureFrozen,
    Unknown(u8),
}

impl ScopeMode {
    pub fn from_wire(value: u8) -> Self {
        match value {
            0 => Self::Off,
            1 => Self::Stream,
            2 => Self::CaptureArmed,
            3 => Self::CapturePost,
            4 => Self::CaptureFrozen,
            other => Self::Unknown(other),
        }
    }

    pub fn wire_value(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Stream => 1,
            Self::CaptureArmed => 2,
            Self::CapturePost => 3,
            Self::CaptureFrozen => 4,
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemState {
    Init,
    Idle,
    Running,
    Fault,
    Unknown(u16),
}

impl SystemState {
    pub fn from_wire(value: u16) -> Self {
        match value {
            0 => Self::Init,
            1 => Self::Idle,
            2 => Self::Running,
            3 => Self::Fault,
            other => Self::Unknown(other),
        }
    }

    pub fn wire_value(self) -> u16 {
        match self {
            Self::Init => 0,
            Self::Idle => 1,
            Self::Running => 2,
            Self::Fault => 3,
            Self::Unknown(value) => value,
        }
    }

    pub fn is_running(self) -> bool {
        self == Self::Running
    }

    pub fn label(self) -> String {
        match self {
            Self::Init => "Init".to_owned(),
            Self::Idle => "Idle".to_owned(),
            Self::Running => "Running".to_owned(),
            Self::Fault => "Fault".to_owned(),
            Self::Unknown(value) => format!("State {value}"),
        }
    }
}

impl fmt::Display for SystemState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerformanceSample {
    pub sequence: u32,
    pub cycle_budget: u32,
    pub load_average: u32,
    pub load_peak: u32,
    pub control_at_peak: u32,
    pub scope_at_peak: u32,
    pub latency_at_peak: u16,
    pub peak_tick: u32,
    pub violations: u32,
    pub overflows: u32,
}

impl PerformanceSample {
    pub fn adc_at_peak(&self) -> u32 {
        u32::from(self.latency_at_peak)
    }

    pub fn runtime_at_peak(&self) -> u32 {
        self.load_peak
            .saturating_sub(self.adc_at_peak())
            .saturating_sub(self.control_at_peak)
            .saturating_sub(self.scope_at_peak)
    }

    pub fn headroom_at_peak(&self) -> u32 {
        self.cycle_budget.saturating_sub(self.load_peak)
    }

    pub fn peak_percent(&self) -> f64 {
        f64::from(self.load_peak) * 100.0 / f64::from(self.cycle_budget)
    }

    pub fn average_percent(&self) -> f64 {
        f64::from(self.load_average) * 100.0 / f64::from(self.cycle_budget)
    }

    pub fn has_violation(&self) -> bool {
        self.violations != 0 || self.overflows != 0 || self.load_peak >= self.cycle_budget
    }
}

#[derive(Debug, Clone)]
pub struct DeviceStatus {
    pub system_state: SystemState,
    pub fault_code: u16,
    pub status_flags: u16,
    pub tick: u32,
    pub cpu1_heartbeat: u32,
    pub cpu2_heartbeat: u32,
    pub applied_seq: u32,
    pub calibration_result: u16,
    pub calibration_fail_index: u16,
    pub build_hash: u32,
    pub scope_mode: ScopeMode,
    pub scope_flags: u8,
    pub command_ack_seq: Option<u32>,
    pub command_result: Option<u16>,
    pub performance: Option<PerformanceSample>,
    pub scope_state_seq: u16,
    pub scope_frozen_count: u16,
    pub scope_trigger_tick: u32,
    pub scope_bind_seq: u16,
}

#[derive(Debug, Clone)]
pub struct ScopeBlock {
    pub start_tick: u32,
    pub block_seq: u16,
    pub flags: u16,
    pub sample_count: u16,
    pub channel_count: u16,
    pub bind_seq: u16,
    pub stride_octets: u16,
    pub samples: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerEdge {
    Rise,
    Fall,
}

#[derive(Debug, Clone)]
pub struct ScopeConfig {
    pub mode: ScopeMode,
    pub trigger_slot: u16,
    pub trigger_level: f32,
    pub trigger_hysteresis: f32,
    pub trigger_edge: TriggerEdge,
    pub pre_trigger_percent: u8,
    pub prescaler: u16,
    pub record_points: u16,
    pub ack_capture_id: u16,
    pub flags: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemCommand {
    Start,
    Stop,
    ClearFault,
}

impl SystemCommand {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::ClearFault => "Clear Fault",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationCommand {
    MeasureZero,
    CommitToFlash,
}

impl CalibrationCommand {
    pub fn label(self) -> &'static str {
        match self {
            Self::MeasureZero => "Measure Zero",
            Self::CommitToFlash => "Commit to Flash",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultSource {
    None,
    System,
    User,
}

impl FaultSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::System => "System",
            Self::User => "User",
        }
    }
}

pub fn fault_source(code: u16) -> FaultSource {
    match code {
        0 => FaultSource::None,
        1..=255 => FaultSource::System,
        _ => FaultSource::User,
    }
}

pub fn fault_user_code(code: u16) -> Option<u16> {
    if fault_source(code) == FaultSource::User {
        Some(code - FAULT_USER_BASE)
    } else {
        None
    }
}

pub fn fault_code_text(code: u16) -> String {
    match code {
        0 => "NONE".to_owned(),
        1 => "TZ1_EXT".to_owned(),
        2 => "OVERCURRENT".to_owned(),
        3 => "OVERVOLTAGE".to_owned(),
        4 => "OVERTEMP".to_owned(),
        5 => "STACK_GUARD".to_owned(),
        6 => "WD_RESET".to_owned(),
        7 => "ITRAP".to_owned(),
        FAULT_USER_OVER_SPEED => "Over Speed".to_owned(),
        other => fault_user_code(other)
            .map(|user_code| format!("USER_{user_code}"))
            .unwrap_or_else(|| format!("FAULT_{other}")),
    }
}

pub fn fault_status_text(code: u16) -> String {
    match fault_source(code) {
        FaultSource::None => "No fault (0)".to_owned(),
        source => format!(
            "{} fault: {} ({})",
            source.label(),
            fault_code_text(code),
            code
        ),
    }
}

pub fn command_result_text(result: u16) -> String {
    match result {
        0 => "OK".to_owned(),
        1 => "BAD_CMD".to_owned(),
        2 => "BAD_STATE".to_owned(),
        3 => "NOT_READY".to_owned(),
        4 => "START_FAILED".to_owned(),
        5 => "CAL_FAILED".to_owned(),
        other => format!("CMDR_{other}"),
    }
}

#[derive(Debug, Clone)]
pub enum TransportEndpoint {
    Serial {
        port: String,
        baud: u32,
    },
    #[allow(dead_code)]
    LocalByteStream(PathBuf),
}

#[derive(Debug, Clone)]
pub struct ParamWrite {
    pub var: VarRef,
    pub value_bits: u32,
}

#[derive(Debug, Clone)]
pub struct ValueRead {
    pub descriptor_index: usize,
    pub var: VarRef,
}

#[derive(Debug)]
pub enum CatalogCommand {
    WriteParams(Vec<ParamWrite>),
    CommitParams,
    ReadValues(Vec<ValueRead>),
    BindChannels { channels: Vec<VarRef> },
    ConfigureScope(ScopeConfig),
    ForceCapture,
}

#[derive(Debug)]
pub enum SourceCommand {
    Connect(TransportEndpoint),
    Disconnect,
    Shutdown,
    Catalog {
        build_hash: u32,
        command: CatalogCommand,
    },
    SystemCommand(SystemCommand),
    CalibrationCommand(CalibrationCommand),
    SrmOpenLoopAbz,
    #[cfg(test)]
    AbzZeroing,
}

#[derive(Debug)]
pub enum SourceEvent {
    Connected(DeviceInfo),
    Disconnected,
    /// A complete catalog replacement for the current firmware build.
    Descriptors(Vec<VarDescriptor>),
    Status(DeviceStatus),
    ParamsStaged,
    ParamsCommitted {
        sequence: u32,
    },
    Values {
        read_sequence: u32,
        indexes: Vec<usize>,
        values: Vec<u32>,
    },
    ChannelsBound {
        bind_sequence: u16,
    },
    SystemCommandAccepted {
        command: SystemCommand,
        sequence: u32,
    },
    CalibrationMeasureAccepted {
        sequence: u32,
    },
    CalibrationCommitCompleted {
        commit_sequence: u32,
    },
    CalibrationCommandFailed {
        command: CalibrationCommand,
        message: String,
    },
    SrmOpenLoopAbzAccepted {
        sequence: u32,
    },
    SrmOpenLoopAbzCommandFailed {
        message: String,
    },
    #[cfg(test)]
    AbzZeroingAccepted {
        sequence: u32,
    },
    #[cfg(test)]
    AbzZeroingCommandFailed {
        message: String,
    },
    ScopeConfigured {
        mode: ScopeMode,
    },
    CaptureForceAccepted {
        capture_state_sequence: u16,
    },
    CaptureForceFailed {
        message: String,
    },
    CaptureFrame {
        capture_id: u16,
        trigger_tick: u32,
        blocks: Vec<ScopeBlock>,
    },
    Blocks {
        mode: ScopeMode,
        blocks: Vec<ScopeBlock>,
    },
    StreamGap {
        expected: u16,
        received: u16,
    },
    PushFrameGap {
        expected: u16,
        received: u16,
    },
    ValueReadFailed {
        message: String,
    },
    DeviceChanged {
        old_hash: u32,
        info: DeviceInfo,
    },
    Error(String),
    Log(String),
}

pub struct SourceHandle {
    pub commands: mpsc::Sender<SourceCommand>,
    pub events: mpsc::Receiver<SourceEvent>,
    worker: Option<thread::JoinHandle<()>>,
}

impl SourceHandle {
    pub fn new(
        commands: mpsc::Sender<SourceCommand>,
        events: mpsc::Receiver<SourceEvent>,
        worker: thread::JoinHandle<()>,
    ) -> Self {
        Self {
            commands,
            events,
            worker: Some(worker),
        }
    }

    pub fn shutdown(&mut self) {
        let _ = self.commands.send(SourceCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for SourceHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub trait DataSource: Send + 'static {
    fn spawn(self: Box<Self>) -> SourceHandle;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fault_code_text_names_known_codes_and_preserves_unknowns() {
        assert_eq!(fault_code_text(0), "NONE");
        assert_eq!(fault_code_text(1), "TZ1_EXT");
        assert_eq!(fault_code_text(2), "OVERCURRENT");
        assert_eq!(fault_code_text(3), "OVERVOLTAGE");
        assert_eq!(fault_code_text(4), "OVERTEMP");
        assert_eq!(fault_code_text(5), "STACK_GUARD");
        assert_eq!(fault_code_text(6), "WD_RESET");
        assert_eq!(fault_code_text(7), "ITRAP");
        assert_eq!(fault_code_text(99), "FAULT_99");
        assert_eq!(fault_code_text(256), "USER_0");
        assert_eq!(fault_code_text(257), "Over Speed");
        assert_eq!(fault_code_text(u16::MAX), "USER_65279");
    }

    #[test]
    fn fault_source_distinguishes_system_and_user_ranges() {
        assert_eq!(fault_source(0), FaultSource::None);
        assert_eq!(fault_source(1), FaultSource::System);
        assert_eq!(fault_source(255), FaultSource::System);
        assert_eq!(fault_source(256), FaultSource::User);
        assert_eq!(fault_source(u16::MAX), FaultSource::User);
        assert_eq!(fault_user_code(255), None);
        assert_eq!(fault_user_code(256), Some(0));
        assert_eq!(fault_user_code(u16::MAX), Some(65279));
    }

    #[test]
    fn fault_status_text_includes_source() {
        assert_eq!(fault_status_text(0), "No fault (0)");
        assert_eq!(fault_status_text(2), "System fault: OVERCURRENT (2)");
        assert_eq!(fault_status_text(257), "User fault: Over Speed (257)");
    }

    #[test]
    fn command_result_text_names_known_results_and_preserves_unknowns() {
        assert_eq!(command_result_text(0), "OK");
        assert_eq!(command_result_text(1), "BAD_CMD");
        assert_eq!(command_result_text(2), "BAD_STATE");
        assert_eq!(command_result_text(3), "NOT_READY");
        assert_eq!(command_result_text(4), "START_FAILED");
        assert_eq!(command_result_text(5), "CAL_FAILED");
        assert_eq!(command_result_text(99), "CMDR_99");
    }
}
