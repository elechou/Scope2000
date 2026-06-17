pub mod csv;
pub mod data;
pub mod dnd;
pub mod pane;
pub mod panel;
pub mod selection;
pub mod tiles;
pub mod viewer_panel;

use serde::{Deserialize, Serialize};

use crate::source::{ScopeMode, TriggerEdge, VarDescriptor};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AcquisitionSettings {
    pub prescaler: u16,
    pub block_ticks: u16,
    pub trigger_source: Option<String>,
    pub trigger_level: f32,
    pub pre_trigger_percent: u8,
    pub trigger_edge: TriggerEdge,
    pub max_points: usize,
}

impl Default for AcquisitionSettings {
    fn default() -> Self {
        Self {
            prescaler: 1,
            block_ticks: 10,
            trigger_source: None,
            trigger_level: 0.0,
            pre_trigger_percent: 50,
            trigger_edge: TriggerEdge::Rise,
            max_points: 50_000,
        }
    }
}

impl AcquisitionSettings {
    pub fn clamp(&mut self) {
        self.prescaler = self.prescaler.clamp(1, 10_000);
        self.block_ticks = self.block_ticks.clamp(1, 100);
        self.pre_trigger_percent = self.pre_trigger_percent.min(100);
        self.max_points = self.max_points.clamp(1_000, 2_000_000);
    }
}

pub struct WaveState {
    pub active: bool,
    pub restart_pending: Option<ScopeMode>,
    pub binding: Vec<VarDescriptor>,
    pub pending_binding: Vec<VarDescriptor>,
    pub bind_sequence: Option<u16>,
    pub settings: AcquisitionSettings,
    pub settings_snapshot: AcquisitionSettings,
    pub pane_vars_snapshot: Vec<String>,
    pub mode: ScopeMode,
}

impl Default for WaveState {
    fn default() -> Self {
        Self {
            active: false,
            restart_pending: None,
            binding: Vec::new(),
            pending_binding: Vec::new(),
            bind_sequence: None,
            settings: AcquisitionSettings::default(),
            settings_snapshot: AcquisitionSettings::default(),
            pane_vars_snapshot: Vec::new(),
            mode: ScopeMode::Off,
        }
    }
}

impl WaveState {
    pub fn clear_binding(&mut self) {
        self.active = false;
        self.restart_pending = None;
        self.binding.clear();
        self.pending_binding.clear();
        self.bind_sequence = None;
        self.mode = ScopeMode::Off;
    }
}
