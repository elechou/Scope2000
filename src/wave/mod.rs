pub mod csv;
pub mod data;
pub mod dnd;
pub mod pane;
pub mod panel;
pub mod selection;
pub mod tiles;
pub mod viewer_panel;

use serde::{Deserialize, Serialize};

use crate::source::{ScopeBlock, ScopeMode, TriggerEdge, VarDescriptor};

pub const DEFAULT_TICK_HZ: u32 = 20_000;
pub const MIN_PRESCALER: u16 = 1;
pub const MAX_PRESCALER: u16 = 10_000;
pub const PLOT_MAX_POINTS: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AcquisitionSettings {
    pub prescaler: u16,
    pub trigger_source: Option<String>,
    pub trigger_level: f32,
    pub trigger_hysteresis: f32,
    pub pre_trigger_percent: u8,
    pub trigger_edge: TriggerEdge,
}

impl Default for AcquisitionSettings {
    fn default() -> Self {
        Self {
            prescaler: MIN_PRESCALER,
            trigger_source: None,
            trigger_level: 0.0,
            trigger_hysteresis: 0.0,
            pre_trigger_percent: 50,
            trigger_edge: TriggerEdge::Rise,
        }
    }
}

impl AcquisitionSettings {
    pub fn clamp(&mut self) {
        self.prescaler = self.prescaler.clamp(MIN_PRESCALER, MAX_PRESCALER);
        if !self.trigger_hysteresis.is_finite() || self.trigger_hysteresis < 0.0 {
            self.trigger_hysteresis = 0.0;
        }
        self.pre_trigger_percent = self.pre_trigger_percent.min(100);
    }

    pub fn sample_rate_hz(&self, tick_hz: u32) -> f64 {
        f64::from(effective_tick_hz(tick_hz)) / f64::from(self.prescaler.max(1))
    }

    pub fn sample_interval_us(&self, tick_hz: u32) -> f64 {
        f64::from(self.prescaler.max(1)) * 1_000_000.0 / f64::from(effective_tick_hz(tick_hz))
    }

    pub fn set_sample_interval_us(&mut self, tick_hz: u32, interval_us: f64) {
        let tick_hz = effective_tick_hz(tick_hz);
        let min_interval_us = 1_000_000.0 / f64::from(tick_hz);
        let max_interval_us = f64::from(MAX_PRESCALER) * min_interval_us;
        let interval_us = if interval_us.is_finite() {
            interval_us.clamp(min_interval_us, max_interval_us)
        } else {
            min_interval_us
        };
        self.prescaler = ((interval_us * f64::from(tick_hz) / 1_000_000.0).round() as u16)
            .clamp(MIN_PRESCALER, MAX_PRESCALER);
    }
}

pub fn effective_tick_hz(tick_hz: u32) -> u32 {
    tick_hz.max(1)
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
    pub capture_frame_blocks: Vec<ScopeBlock>,
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
            capture_frame_blocks: Vec::new(),
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
        self.capture_frame_blocks.clear();
        self.mode = ScopeMode::Off;
    }
}
