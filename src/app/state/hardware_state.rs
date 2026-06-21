use crate::source::{DeviceInfo, DeviceStatus, PerformanceSample, ScopeMode, TransportEndpoint};

#[derive(Default)]
pub struct PerformanceState {
    available: bool,
    sample: Option<PerformanceSample>,
}

impl PerformanceState {
    pub fn set_available(&mut self, available: bool) {
        self.available = available;
        if !available {
            self.sample = None;
        }
    }

    pub fn clear(&mut self) {
        self.available = false;
        self.sample = None;
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    pub fn sample(&self) -> Option<PerformanceSample> {
        self.sample
    }

    pub fn ingest_status(&mut self, sample: Option<PerformanceSample>) {
        self.available = true;
        if let Some(sample) = sample {
            self.sample = Some(sample);
        }
    }
}

pub(crate) struct HardwareState {
    pub port: String,
    pub baud: u32,
    pub serial_ports: Vec<String>,
    pub connected: bool,
    pub connecting: bool,
    pub info: Option<DeviceInfo>,
    pub status: Option<DeviceStatus>,
    pub version: Option<String>,
    pub performance: PerformanceState,
}

impl Default for HardwareState {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: 115_200,
            serial_ports: Vec::new(),
            connected: false,
            connecting: false,
            info: None,
            status: None,
            version: None,
            performance: PerformanceState::default(),
        }
    }
}

impl HardwareState {
    pub fn can_configure_connection(&self) -> bool {
        !self.connected && !self.connecting
    }

    pub fn endpoint(&self) -> Option<TransportEndpoint> {
        (!self.port.is_empty()).then(|| TransportEndpoint::Serial {
            port: self.port.clone(),
            baud: self.baud,
        })
    }

    pub fn endpoint_label(&self) -> String {
        if self.port.is_empty() {
            "No serial port".to_owned()
        } else {
            format!("{} @ {}", self.port, self.baud)
        }
    }

    pub fn is_running(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status.system_state.is_running())
    }

    pub fn version_text(&self) -> Option<String> {
        self.info
            .as_ref()
            .map(|info| format!("Viewer2000 · {}", tick_rate_text(info.tick_hz)))
    }

    pub fn version_hover_text(&self) -> Option<String> {
        self.info.as_ref().map(|info| {
            format!(
                "Viewer2000 Device\nfirmware {}\nwire={} contract={}\nbuild=0x{:08X}\ntick={}Hz",
                info.firmware_name,
                info.protocol_version,
                info.contract_version,
                info.build_hash,
                info.tick_hz
            )
        })
    }

    pub fn scope_mode_label(&self) -> &'static str {
        let Some(status) = &self.status else {
            return "unknown";
        };
        match status.scope_mode {
            ScopeMode::Off => "off",
            ScopeMode::Stream => "stream",
            ScopeMode::CaptureArmed => "capture armed",
            ScopeMode::CapturePost => "capture post",
            ScopeMode::CaptureFrozen => "capture frozen",
            ScopeMode::Unknown(_) => "unknown",
        }
    }
}

fn tick_rate_text(tick_hz: u32) -> String {
    if tick_hz != 0 && tick_hz.is_multiple_of(1_000) {
        format!("{}kHz", tick_hz / 1_000)
    } else {
        format!("{tick_hz}Hz")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{DeviceInfo, PerformanceSample};

    #[test]
    fn connection_settings_are_locked_while_connecting_or_connected() {
        let mut hardware = HardwareState::default();
        assert!(hardware.can_configure_connection());

        hardware.connecting = true;
        assert!(!hardware.can_configure_connection());

        hardware.connecting = false;
        hardware.connected = true;
        assert!(!hardware.can_configure_connection());
    }

    #[test]
    fn version_text_is_a_compact_viewer2000_rate_summary() {
        let hardware = HardwareState {
            info: Some(DeviceInfo {
                protocol_version: 1,
                contract_version: 1,
                build_hash: 0x3C31_3C66,
                descriptor_count: 0,
                firmware_name: "viewer2000".to_owned(),
                tick_hz: 20_000,
                capabilities: 0,
                project_name: String::new(),
                build_time_utc: 0,
            }),
            ..HardwareState::default()
        };

        assert_eq!(
            hardware.version_text().as_deref(),
            Some("Viewer2000 · 20kHz")
        );
    }

    #[test]
    fn version_text_does_not_repeat_project_identity() {
        let hardware = HardwareState {
            info: Some(DeviceInfo {
                protocol_version: 1,
                contract_version: 13,
                build_hash: 0x3C31_3C66,
                descriptor_count: 0,
                firmware_name: "viewer2000".to_owned(),
                tick_hz: 20_000,
                capabilities: 0,
                project_name: "untitled".to_owned(),
                build_time_utc: 0,
            }),
            ..HardwareState::default()
        };

        assert_eq!(
            hardware.version_text().as_deref(),
            Some("Viewer2000 · 20kHz")
        );
    }

    #[test]
    fn performance_state_tracks_status_availability() {
        let mut performance = PerformanceState::default();
        assert!(!performance.is_available());
        assert_eq!(performance.sample(), None);

        performance.set_available(true);
        assert!(performance.is_available());
        assert_eq!(performance.sample(), None);

        performance.clear();
        assert!(!performance.is_available());
    }

    #[test]
    fn performance_state_keeps_last_valid_status_sample() {
        let mut performance = PerformanceState::default();
        let sample = PerformanceSample {
            sequence: 7,
            cycle_budget: 2_000,
            load_average: 824,
            load_peak: 1_224,
            control_at_peak: 700,
            scope_at_peak: 300,
            latency_at_peak: 24,
            peak_tick: 99,
            violations: 0,
            overflows: 0,
        };
        performance.ingest_status(Some(sample));

        assert_eq!(sample.runtime_at_peak(), 200);
        assert_eq!(sample.headroom_at_peak(), 776);
        assert_eq!(sample.peak_percent(), 61.2);
        assert_eq!(sample.average_percent(), 41.2);
        assert_eq!(performance.sample(), Some(sample));

        performance.ingest_status(None);
        assert_eq!(performance.sample(), Some(sample));

        let overloaded = PerformanceSample {
            sequence: 8,
            cycle_budget: 1_000,
            load_average: 930,
            load_peak: 1_530,
            control_at_peak: 800,
            scope_at_peak: 400,
            latency_at_peak: 30,
            peak_tick: 100,
            violations: 0,
            overflows: 0,
        };
        performance.ingest_status(Some(overloaded));
        let overloaded = performance.sample().expect("overloaded sample");
        assert_eq!(overloaded.peak_percent(), 153.0);
        assert_eq!(overloaded.headroom_at_peak(), 0);
        assert!(overloaded.has_violation());
    }
}
