use crate::source::{DeviceInfo, DeviceStatus, ScopeMode, TransportEndpoint};

pub(crate) struct HardwareState {
    pub port: String,
    pub baud: u32,
    pub serial_ports: Vec<String>,
    pub connected: bool,
    pub connecting: bool,
    pub info: Option<DeviceInfo>,
    pub status: Option<DeviceStatus>,
    pub version: Option<String>,
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
    use crate::source::DeviceInfo;

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
                contract_version: 11,
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
}
