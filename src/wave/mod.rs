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
pub const DEFAULT_RECORD_POINTS: u16 = 1_000;
pub const MIN_RECORD_POINTS: u16 = 1;
pub const MAX_RECORD_POINTS_ABSOLUTE: u16 = 0x7000;
pub const DEVICE_BLOCK_TICKS: u16 = 10;
pub const SCOPE_RING_WORDS: u32 = 0x7000;
const BLOCK_HEADER_WORDS: u32 = 8;
pub const PLOT_MAX_POINTS: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AcquisitionSettings {
    pub prescaler: u16,
    pub record_points: u16,
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
            record_points: DEFAULT_RECORD_POINTS,
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
        self.record_points = self
            .record_points
            .clamp(MIN_RECORD_POINTS, MAX_RECORD_POINTS_ABSOLUTE);
        if !self.trigger_hysteresis.is_finite() || self.trigger_hysteresis < 0.0 {
            self.trigger_hysteresis = 0.0;
        }
        self.pre_trigger_percent = self.pre_trigger_percent.min(100);
    }

    pub fn clamp_record_points(&mut self, max_points: Option<u16>) {
        let max_points = max_points
            .unwrap_or(MAX_RECORD_POINTS_ABSOLUTE)
            .clamp(MIN_RECORD_POINTS, MAX_RECORD_POINTS_ABSOLUTE);
        self.record_points = self.record_points.clamp(MIN_RECORD_POINTS, max_points);
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

    pub fn record_duration_seconds(&self, tick_hz: u32) -> f64 {
        f64::from(self.record_points) / self.sample_rate_hz(tick_hz).max(f64::MIN_POSITIVE)
    }
}

pub fn effective_tick_hz(tick_hz: u32) -> u32 {
    tick_hz.max(1)
}

pub fn max_record_points_for_binding(binding: &[VarDescriptor]) -> Option<u16> {
    let stride_words: u32 = binding
        .iter()
        .map(|descriptor| (descriptor.var.ty.wire_width() / 2) as u32)
        .sum();
    if stride_words == 0 {
        return None;
    }
    let mut slot_words = BLOCK_HEADER_WORDS + u32::from(DEVICE_BLOCK_TICKS) * stride_words;
    if slot_words & 1 != 0 {
        slot_words += 1;
    }
    let block_capacity = floor_pow2(SCOPE_RING_WORDS / slot_words);
    let points = block_capacity.saturating_mul(u32::from(DEVICE_BLOCK_TICKS));
    Some(points.min(u32::from(MAX_RECORD_POINTS_ABSOLUTE)) as u16)
}

fn floor_pow2(value: u32) -> u32 {
    if value == 0 {
        return 0;
    }
    1 << (31 - value.leading_zeros())
}

pub fn format_record_duration(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    if seconds < 0.001 {
        format!("{:.1} us", seconds * 1_000_000.0)
    } else if seconds < 1.0 {
        format!("{:.1} ms", seconds * 1_000.0)
    } else {
        format!("{seconds:.3} s")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{VarRef, VarType};

    fn descriptor(name: &str, ty: VarType) -> VarDescriptor {
        VarDescriptor {
            name: name.to_owned(),
            var: VarRef { addr: 0, ty },
            kind: 0x0002,
            prescaler: 1,
        }
    }

    #[test]
    fn record_duration_uses_adaptive_units() {
        let settings = AcquisitionSettings {
            record_points: 1_000,
            ..AcquisitionSettings::default()
        };

        assert_eq!(
            format_record_duration(settings.record_duration_seconds(20_000)),
            "50.0 ms"
        );
        assert_eq!(format_record_duration(0.000_5), "500.0 us");
        assert_eq!(format_record_duration(1.25), "1.250 s");
    }

    #[test]
    fn max_record_points_accounts_for_native_width_and_block_headers() {
        let one_f32 = vec![descriptor("f32", VarType::F32)];
        let eight_f32 = (0..8)
            .map(|index| descriptor(&format!("f32_{index}"), VarType::F32))
            .collect::<Vec<_>>();
        let mixed = vec![
            descriptor("i16", VarType::I16),
            descriptor("u32", VarType::U32),
            descriptor("f32", VarType::F32),
        ];

        assert_eq!(max_record_points_for_binding(&one_f32), Some(10_240));
        assert_eq!(max_record_points_for_binding(&eight_f32), Some(1_280));
        assert_eq!(max_record_points_for_binding(&mixed), Some(2_560));
        assert_eq!(max_record_points_for_binding(&[]), None);
    }

    #[test]
    fn record_points_clamp_uses_binding_limit() {
        let mut settings = AcquisitionSettings {
            record_points: 10_000,
            ..AcquisitionSettings::default()
        };

        settings.clamp_record_points(Some(1_280));

        assert_eq!(settings.record_points, 1_280);
    }
}
