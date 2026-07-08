use std::time::{Duration, Instant};

use crate::source::{
    DeviceInfo, DeviceStatus, PerformanceSample, ScopeMode, SystemCommand, TransportEndpoint,
    command_result_text,
};

pub(crate) const DEFAULT_SERIAL_BAUD: u32 = 3_125_000;
const SYSTEM_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

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
    pub device_summary: Option<String>,
    pub performance: PerformanceState,
    pub pending_system_command: Option<PendingSystemCommand>,
    pub last_system_command: Option<CompletedSystemCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingSystemCommand {
    pub command: SystemCommand,
    pub sequence: Option<u32>,
    sent_at: Instant,
    accepted_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompletedSystemCommand {
    pub command: SystemCommand,
    pub sequence: u32,
    pub result: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExpiredSystemCommand {
    pub command: SystemCommand,
    pub sequence: Option<u32>,
}

impl Default for HardwareState {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: DEFAULT_SERIAL_BAUD,
            serial_ports: Vec::new(),
            connected: false,
            connecting: false,
            info: None,
            status: None,
            device_summary: None,
            performance: PerformanceState::default(),
            pending_system_command: None,
            last_system_command: None,
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

    pub fn device_summary_text(&self) -> Option<String> {
        self.info.as_ref().map(|info| {
            format!(
                "{} · {}",
                info.mcu_model_label(),
                tick_rate_text(info.tick_hz)
            )
        })
    }

    pub fn device_info_hover_text(&self) -> Option<String> {
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

    pub fn begin_system_command(&mut self, command: SystemCommand, now: Instant) {
        self.pending_system_command = Some(PendingSystemCommand {
            command,
            sequence: None,
            sent_at: now,
            accepted_at: None,
        });
    }

    pub fn accept_system_command(
        &mut self,
        command: SystemCommand,
        sequence: u32,
        now: Instant,
    ) -> bool {
        let Some(pending) = &mut self.pending_system_command else {
            return false;
        };
        if pending.command != command {
            return false;
        }
        pending.sequence = Some(sequence);
        pending.accepted_at = Some(now);
        true
    }

    pub fn complete_pending_system_command(
        &mut self,
        status: &DeviceStatus,
    ) -> Option<CompletedSystemCommand> {
        let pending = self.pending_system_command?;
        let sequence = pending.sequence?;
        if status.command_ack_seq != Some(sequence) {
            return None;
        }
        let completed = CompletedSystemCommand {
            command: pending.command,
            sequence,
            result: status.command_result.unwrap_or_default(),
        };
        self.pending_system_command = None;
        self.last_system_command = Some(completed);
        Some(completed)
    }

    pub fn expire_pending_system_command(&mut self, now: Instant) -> Option<ExpiredSystemCommand> {
        let pending = self.pending_system_command?;
        let reference = pending.accepted_at.unwrap_or(pending.sent_at);
        if now.saturating_duration_since(reference) < SYSTEM_COMMAND_TIMEOUT {
            return None;
        }
        self.pending_system_command = None;
        Some(ExpiredSystemCommand {
            command: pending.command,
            sequence: pending.sequence,
        })
    }

    pub fn clear_system_command_state(&mut self) {
        self.pending_system_command = None;
        self.last_system_command = None;
    }

    pub fn pending_system_command_text(&self) -> Option<String> {
        let pending = self.pending_system_command?;
        Some(match pending.sequence {
            Some(sequence) => {
                format!(
                    "Command: {} pending seq {sequence}",
                    pending.command.label()
                )
            }
            None => format!("Command: {} sending...", pending.command.label()),
        })
    }

    pub fn last_system_command_text(&self) -> Option<String> {
        let completed = self.last_system_command?;
        Some(format!(
            "Last command: {} {} (seq {})",
            completed.command.label(),
            command_result_text(completed.result),
            completed.sequence
        ))
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
    use crate::source::{
        DeviceInfo, DeviceStatus, PerformanceSample, ScopeMode, SystemCommand, SystemState,
    };

    fn device_status(command_ack_seq: Option<u32>, command_result: Option<u16>) -> DeviceStatus {
        DeviceStatus {
            system_state: SystemState::Idle,
            fault_code: 0,
            status_flags: 0,
            tick: 0,
            cpu1_heartbeat: 0,
            cpu2_heartbeat: 0,
            applied_seq: 0,
            calibration_result: 0,
            calibration_fail_index: 0,
            build_hash: 0,
            scope_mode: ScopeMode::Off,
            scope_flags: 0,
            command_ack_seq,
            command_result,
            performance: None,
            scope_state_seq: 0,
            scope_frozen_count: 0,
            scope_trigger_tick: 0,
            scope_bind_seq: 0,
        }
    }

    #[test]
    fn default_baud_is_viewer2000_native_rate() {
        let hardware = HardwareState::default();

        assert_eq!(hardware.baud, DEFAULT_SERIAL_BAUD);
        assert_eq!(hardware.baud, 3_125_000);
    }

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
    fn device_summary_combines_mcu_model_and_tick_rate() {
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
                mcu_model: 2,
                scope_max_ch: 32,
                scope_block_ticks: 7,
                scope_ring_words: 0x7000,
            }),
            ..HardwareState::default()
        };

        assert_eq!(
            hardware.device_summary_text().as_deref(),
            Some("F28379D · 20kHz")
        );
    }

    #[test]
    fn device_summary_does_not_repeat_project_identity() {
        let hardware = HardwareState {
            info: Some(DeviceInfo {
                protocol_version: 1,
                contract_version: 14,
                build_hash: 0x3C31_3C66,
                descriptor_count: 0,
                firmware_name: "viewer2000".to_owned(),
                tick_hz: 20_000,
                capabilities: 0,
                project_name: "untitled".to_owned(),
                build_time_utc: 0,
                mcu_model: 1,
                scope_max_ch: 32,
                scope_block_ticks: 7,
                scope_ring_words: 0x7000,
            }),
            ..HardwareState::default()
        };

        assert_eq!(
            hardware.device_summary_text().as_deref(),
            Some("F28P65x · 20kHz")
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

    #[test]
    fn pending_system_command_completes_on_matching_status_sequence() {
        let mut hardware = HardwareState::default();
        let now = Instant::now();
        hardware.begin_system_command(SystemCommand::Start, now);
        assert_eq!(
            hardware.pending_system_command_text().as_deref(),
            Some("Command: Start sending...")
        );

        assert!(hardware.accept_system_command(SystemCommand::Start, 42, now));
        assert_eq!(
            hardware.pending_system_command_text().as_deref(),
            Some("Command: Start pending seq 42")
        );

        let completed = hardware
            .complete_pending_system_command(&device_status(Some(42), Some(0)))
            .expect("matching sequence completes");

        assert_eq!(completed.command, SystemCommand::Start);
        assert_eq!(completed.sequence, 42);
        assert_eq!(completed.result, 0);
        assert_eq!(hardware.pending_system_command, None);
        assert_eq!(
            hardware.last_system_command_text().as_deref(),
            Some("Last command: Start OK (seq 42)")
        );
    }

    #[test]
    fn non_matching_status_sequence_does_not_complete_pending_command() {
        let mut hardware = HardwareState::default();
        let now = Instant::now();
        hardware.begin_system_command(SystemCommand::Start, now);
        assert!(hardware.accept_system_command(SystemCommand::Start, 42, now));

        assert_eq!(
            hardware.complete_pending_system_command(&device_status(Some(41), Some(0))),
            None
        );
        assert!(hardware.pending_system_command.is_some());
        assert_eq!(hardware.last_system_command, None);
    }

    #[test]
    fn system_command_state_clear_drops_pending_and_last_command() {
        let mut hardware = HardwareState::default();
        let now = Instant::now();
        hardware.begin_system_command(SystemCommand::Stop, now);
        assert!(hardware.accept_system_command(SystemCommand::Stop, 7, now));
        assert!(
            hardware
                .complete_pending_system_command(&device_status(Some(7), Some(4)))
                .is_some()
        );
        hardware.begin_system_command(SystemCommand::ClearFault, now);

        hardware.clear_system_command_state();

        assert_eq!(hardware.pending_system_command, None);
        assert_eq!(hardware.last_system_command, None);
    }

    #[test]
    fn pending_system_command_expires_when_status_ack_never_matches() {
        let mut hardware = HardwareState::default();
        let now = Instant::now();
        hardware.begin_system_command(SystemCommand::ClearFault, now);
        assert!(hardware.accept_system_command(SystemCommand::ClearFault, 99, now));

        assert_eq!(
            hardware.expire_pending_system_command(now + Duration::from_secs(1)),
            None
        );

        let expired = hardware
            .expire_pending_system_command(now + SYSTEM_COMMAND_TIMEOUT)
            .expect("pending command expires");

        assert_eq!(expired.command, SystemCommand::ClearFault);
        assert_eq!(expired.sequence, Some(99));
        assert_eq!(hardware.pending_system_command, None);
    }
}
