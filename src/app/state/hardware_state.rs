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

    pub fn state_label(&self) -> String {
        match self.status.as_ref().map(|status| status.system_state) {
            Some(state) => state.label(),
            None if self.connected => "Connected".to_owned(),
            None => "Disconnected".to_owned(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status.system_state.is_running())
    }

    pub fn version_text(&self) -> Option<String> {
        self.info.as_ref().map(|info| {
            format!(
                "{}  build=0x{:08X}  tick={}Hz",
                info.firmware_name, info.build_hash, info.tick_hz
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
