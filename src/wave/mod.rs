pub mod csv;
pub mod data;
pub mod dnd;
pub mod pane;
pub mod panel;
pub mod selection;
pub mod tiles;
pub mod viewer_panel;

use serde::{Deserialize, Serialize};

use crate::source::{DeviceInfo, ScopeMode, TriggerEdge, VarDescriptor};

pub const DEFAULT_TICK_HZ: u32 = 20_000;
pub const MIN_PRESCALER: u16 = 1;
pub const MAX_PRESCALER: u16 = 10_000;
pub const DEFAULT_RECORD_POINTS: u16 = 1_000;
pub const MIN_RECORD_POINTS: u16 = 1;
pub const MAX_RECORD_POINTS_ABSOLUTE: u16 = u16::MAX;
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

    pub fn with_record_point_fallback(&self, max_points: Option<u16>) -> Self {
        let mut settings = self.clone();
        settings.clamp();
        settings.clamp_record_points(max_points);
        settings
    }

    pub fn sample_rate_hz(&self, tick_hz: u32) -> f64 {
        f64::from(effective_tick_hz(tick_hz)) / f64::from(self.prescaler.max(1))
    }

    pub fn sample_interval_us(&self, tick_hz: u32) -> f64 {
        f64::from(self.prescaler.max(1)) * 1_000_000.0 / f64::from(effective_tick_hz(tick_hz))
    }

    pub fn record_duration_seconds(&self, tick_hz: u32) -> f64 {
        f64::from(self.record_points) * self.sample_interval_us(tick_hz) / 1_000_000.0
    }
}

pub fn effective_tick_hz(tick_hz: u32) -> u32 {
    tick_hz.max(1)
}

pub(crate) fn sampling_prescaler_steps(tick_hz: u32) -> Vec<u16> {
    let tick_hz = f64::from(effective_tick_hz(tick_hz));
    let tick_period_us = 1_000_000.0 / tick_hz;
    let max_interval_us = f64::from(MAX_PRESCALER) * tick_period_us;
    let min_interval_us = tick_period_us;
    let mut steps = std::collections::BTreeSet::new();
    steps.insert(MIN_PRESCALER);
    steps.insert(MAX_PRESCALER);

    let mut decade = 10_f64.powf(min_interval_us.max(f64::MIN_POSITIVE).log10().floor());
    while decade <= max_interval_us * 10.0 {
        for multiplier in [1.0, 2.0, 5.0] {
            let interval_us = multiplier * decade;
            if interval_us < min_interval_us || interval_us > max_interval_us {
                continue;
            }
            let prescaler = (interval_us / tick_period_us).round() as u16;
            steps.insert(prescaler.clamp(MIN_PRESCALER, MAX_PRESCALER));
        }
        decade *= 10.0;
    }

    steps.into_iter().collect()
}

pub(crate) fn nearest_sampling_prescaler(tick_hz: u32, interval_us: f64, steps: &[u16]) -> u16 {
    let tick_hz = f64::from(effective_tick_hz(tick_hz));
    let target = if interval_us.is_finite() {
        interval_us.max(0.0)
    } else {
        0.0
    };
    steps
        .iter()
        .copied()
        .min_by(|left, right| {
            let left_us = f64::from(*left) * 1_000_000.0 / tick_hz;
            let right_us = f64::from(*right) * 1_000_000.0 / tick_hz;
            (left_us - target)
                .abs()
                .total_cmp(&(right_us - target).abs())
        })
        .unwrap_or(MIN_PRESCALER)
}

pub fn scope_channel_limit(info: Option<&DeviceInfo>) -> usize {
    info.and_then(|info| (info.scope_max_ch != 0).then_some(usize::from(info.scope_max_ch)))
        .unwrap_or(0)
}

pub fn max_record_points_for_binding(binding: &[VarDescriptor], info: &DeviceInfo) -> Option<u16> {
    let stride_words: u32 = binding
        .iter()
        .map(|descriptor| (descriptor.var.ty.wire_width() / 2) as u32)
        .sum();
    if stride_words == 0 || info.scope_block_ticks == 0 || info.scope_ring_words == 0 {
        return None;
    }
    let mut slot_words = BLOCK_HEADER_WORDS + u32::from(info.scope_block_ticks) * stride_words;
    if slot_words & 1 != 0 {
        slot_words += 1;
    }
    // Exact ring fit, mirroring the firmware's v2k_scope_layout (wire-spec
    // "capacity_blocks = floor(scope_ring_words / aligned_slot_words)"; the
    // former power-of-two rounding was removed).
    let block_capacity = info.scope_ring_words / slot_words;
    let points = block_capacity
        .saturating_sub(1)
        .saturating_mul(u32::from(info.scope_block_ticks));
    if points == 0 {
        return None;
    }
    Some(points.min(u32::from(MAX_RECORD_POINTS_ABSOLUTE)) as u16)
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
    pub auto_trigger_read_pending: Option<AutoTriggerReadPending>,
    pub binding: Vec<VarDescriptor>,
    pub pending_binding: Vec<VarDescriptor>,
    pub bind_sequence: Option<u16>,
    pub settings: AcquisitionSettings,
    pub settings_snapshot: AcquisitionSettings,
    pub pane_vars_snapshot: Vec<String>,
    pub mode: ScopeMode,
}

#[derive(Debug, Clone, Copy)]
pub struct AutoTriggerReadPending {
    pub mode: ScopeMode,
    pub descriptor_index: usize,
}

impl Default for WaveState {
    fn default() -> Self {
        Self {
            active: false,
            restart_pending: None,
            auto_trigger_read_pending: None,
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
        self.auto_trigger_read_pending = None;
        self.binding.clear();
        self.pending_binding.clear();
        self.bind_sequence = None;
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
        let info = DeviceInfo {
            protocol_version: 10,
            contract_version: 14,
            build_hash: 0,
            descriptor_count: 0,
            firmware_name: "viewer2000".to_owned(),
            tick_hz: 20_000,
            capabilities: 0,
            project_name: "phase4-demo".to_owned(),
            build_time_utc: 0,
            mcu_model: 1,
            scope_max_ch: 16,
            scope_block_ticks: 10,
            scope_ring_words: 0x7000,
        };
        let one_f32 = vec![descriptor("f32", VarType::F32)];
        let eight_f32 = (0..8)
            .map(|index| descriptor(&format!("f32_{index}"), VarType::F32))
            .collect::<Vec<_>>();
        let mixed = vec![
            descriptor("i16", VarType::I16),
            descriptor("u32", VarType::U32),
            descriptor("f32", VarType::F32),
        ];

        assert_eq!(max_record_points_for_binding(&one_f32, &info), Some(10_230));
        assert_eq!(
            max_record_points_for_binding(&eight_f32, &info),
            Some(1_690)
        );
        assert_eq!(max_record_points_for_binding(&mixed, &info), Some(4_930));
        assert_eq!(max_record_points_for_binding(&[], &info), None);
    }

    #[test]
    fn max_record_points_uses_hello_scope_resources() {
        let mut info = DeviceInfo {
            protocol_version: 10,
            contract_version: 14,
            build_hash: 0,
            descriptor_count: 0,
            firmware_name: "viewer2000".to_owned(),
            tick_hz: 20_000,
            capabilities: 0,
            project_name: "phase4-demo".to_owned(),
            build_time_utc: 0,
            mcu_model: 2,
            scope_max_ch: 16,
            scope_block_ticks: 10,
            scope_ring_words: 0xDFF8,
        };
        let eight_f32 = (0..8)
            .map(|index| descriptor(&format!("f32_{index}"), VarType::F32))
            .collect::<Vec<_>>();
        let sixteen_f32 = (0..16)
            .map(|index| descriptor(&format!("f32_{index}"), VarType::F32))
            .collect::<Vec<_>>();

        assert_eq!(scope_channel_limit(Some(&info)), 16);
        assert_eq!(
            max_record_points_for_binding(&eight_f32, &info),
            Some(3_400)
        );
        assert_eq!(
            max_record_points_for_binding(&sixteen_f32, &info),
            Some(1_730)
        );

        info.scope_block_ticks = 0;
        assert_eq!(max_record_points_for_binding(&eight_f32, &info), None);
    }

    #[test]
    fn sampling_prescaler_steps_follow_one_two_five_timebase() {
        let steps = sampling_prescaler_steps(20_000);

        assert_eq!(&steps[..8], &[1, 2, 4, 10, 20, 40, 100, 200]);
        assert!(steps.contains(&MAX_PRESCALER));
    }

    #[test]
    fn nearest_sampling_prescaler_snaps_to_supported_timebase_step() {
        let steps = sampling_prescaler_steps(20_000);

        assert_eq!(nearest_sampling_prescaler(20_000, 180.0, &steps), 4);
        assert_eq!(nearest_sampling_prescaler(20_000, 120.0, &steps), 2);
        assert_eq!(nearest_sampling_prescaler(20_000, 1_800.0, &steps), 40);
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

    #[test]
    fn record_points_fallback_preserves_requested_value() {
        let settings = AcquisitionSettings {
            record_points: 10_000,
            ..AcquisitionSettings::default()
        };

        let effective = settings.with_record_point_fallback(Some(1_280));

        assert_eq!(settings.record_points, 10_000);
        assert_eq!(effective.record_points, 1_280);
    }
}
