pub mod v2k;

use std::path::PathBuf;
use std::sync::mpsc;

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

    pub fn label(self) -> &'static str {
        match self {
            Self::I16 => "i16",
            Self::U16 => "u16",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::F32 => "f32",
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
}

impl DeviceInfo {
    pub fn has(&self, capability: u32) -> bool {
        self.capabilities & capability != 0
    }
}

pub const CAP_ENUM: u32 = 1 << 0;
pub const CAP_CAL: u32 = 1 << 1;
pub const CAP_SCOPE_STREAM: u32 = 1 << 2;
pub const CAP_SCOPE_CAPTURE: u32 = 1 << 3;
pub const CAP_PRE_TRIGGER: u32 = 1 << 4;
pub const CAP_SYSTEM_CMD: u32 = 1 << 5;
pub const CAP_NATIVE_BLOCK: u32 = 1 << 6;

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

#[derive(Debug, Clone)]
pub struct DeviceStatus {
    pub system_state: u16,
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
}

#[derive(Debug, Clone, Copy)]
pub enum SystemCommand {
    Start,
    Stop,
    ClearFault,
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

#[derive(Debug)]
pub enum SourceCommand {
    Connect(TransportEndpoint),
    Disconnect,
    WriteParams(Vec<ParamWrite>),
    CommitParams,
    ReadValues { start: u16, count: u8 },
    BindChannels { channels: Vec<VarRef> },
    ConfigureScope(ScopeConfig),
    SystemCommand(SystemCommand),
}

#[derive(Debug)]
pub enum SourceEvent {
    Connected(DeviceInfo),
    Disconnected,
    Descriptors(Vec<VarDescriptor>),
    Status(DeviceStatus),
    ParamsStaged,
    ParamsCommitted {
        sequence: u32,
    },
    Values {
        mirror_sequence: u32,
        start: u16,
        values: Vec<u32>,
    },
    ChannelsBound {
        bind_sequence: u16,
    },
    ScopeConfigured {
        mode: ScopeMode,
    },
    Blocks {
        mode: ScopeMode,
        remaining_hint: u16,
        trigger_tick: Option<u32>,
        blocks: Vec<ScopeBlock>,
    },
    StreamGap {
        expected: u16,
        received: u16,
    },
    DeviceChanged {
        old_hash: u32,
        new_hash: u32,
    },
    Error(String),
    Log(String),
}

pub struct SourceHandle {
    pub commands: mpsc::Sender<SourceCommand>,
    pub events: mpsc::Receiver<SourceEvent>,
}

pub trait DataSource: Send + 'static {
    fn spawn(self: Box<Self>) -> SourceHandle;
}
