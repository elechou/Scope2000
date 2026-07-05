use std::time::{Duration, Instant};

use crate::source::{CalibrationCommand, DeviceStatus, SystemState};

pub(crate) const CALIBRATION_READ_PERIOD: Duration = Duration::from_millis(250);
pub(crate) const CALIBRATION_STATUS_READ_PERIOD: Duration = Duration::from_secs(1);
pub(crate) const CALIBRATION_STATUS_READ_NAMES: &[&str] = &[
    "v2k_cal.state",
    "v2k_cal.result",
    "v2k_cal.applied_src",
    "v2k_cal.store_valid",
    "v2k_cal.store_result",
    "v2k_cal.store_seq",
];
pub(crate) const CALIBRATION_READ_NAMES: &[&str] = &[
    "v2k_cal.state",
    "v2k_cal.result",
    "v2k_cal.applied_src",
    "v2k_cal.zero_meas.ct1",
    "v2k_cal.zero_meas.ct2",
    "v2k_cal.zero_meas.ct3",
    "v2k_cal.zero_meas.ct4",
    "v2k_cal.zero_meas.ct5",
    "v2k_cal.zero_meas.ct6",
    "v2k_cal.zero_stored.ct1",
    "v2k_cal.zero_stored.ct2",
    "v2k_cal.zero_stored.ct3",
    "v2k_cal.zero_stored.ct4",
    "v2k_cal.zero_stored.ct5",
    "v2k_cal.zero_stored.ct6",
    "v2k_cal.noise_pp.ct1",
    "v2k_cal.noise_pp.ct2",
    "v2k_cal.noise_pp.ct3",
    "v2k_cal.noise_pp.ct4",
    "v2k_cal.noise_pp.ct5",
    "v2k_cal.noise_pp.ct6",
    "v2k_cal.settle_max",
    "v2k_cal.settle_ch",
    "v2k_cal.store_valid",
    "v2k_cal.store_result",
    "v2k_cal.store_seq",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CalibrationHealthLevel {
    Normal,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CalibrationHealth {
    pub level: CalibrationHealthLevel,
    pub label: &'static str,
    pub detail: &'static str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CalibrationSnapshot {
    pub state: Option<u16>,
    pub result: Option<u16>,
    pub applied_source: Option<u16>,
    pub store_valid: Option<u16>,
    pub store_result: Option<u16>,
    pub store_sequence: Option<u32>,
}

impl CalibrationSnapshot {
    pub fn start_ready(self) -> bool {
        matches!(
            (self.state, self.result, self.applied_source),
            (Some(2), Some(1), Some(2))
        )
    }

    pub fn health(self) -> CalibrationHealth {
        if matches!(self.store_result, Some(2..)) {
            return CalibrationHealth {
                level: CalibrationHealthLevel::Error,
                label: "Flash storage failed",
                detail: "The Golden current-sensor reference could not be written to flash.",
            };
        }

        match (self.state, self.result, self.applied_source) {
            (None, _, _) => CalibrationHealth {
                level: CalibrationHealthLevel::Normal,
                label: "Unavailable",
                detail: "Current-sensor calibration status is not available.",
            },
            (Some(0), _, _) => CalibrationHealth {
                level: CalibrationHealthLevel::Warning,
                label: "Pending",
                detail: "Automatic current-sensor calibration has not completed yet. Start is refused until it passes.",
            },
            (Some(1), _, _) => CalibrationHealth {
                level: CalibrationHealthLevel::Warning,
                label: "Measuring",
                detail: "Automatic current-sensor zero measurement is in progress. Start is refused until it passes.",
            },
            (Some(2), Some(1), Some(2)) if self.store_valid == Some(1) => CalibrationHealth {
                level: CalibrationHealthLevel::Normal,
                label: "Normal",
                detail: "The automatic zero measurement is active and the Golden reference is available.",
            },
            (Some(2), Some(1), Some(2)) => CalibrationHealth {
                level: CalibrationHealthLevel::Warning,
                label: "Golden reference missing",
                detail: "The automatic zero measurement is active, but no Golden flash reference is available.",
            },
            (Some(2), Some(1), Some(1)) => CalibrationHealth {
                level: CalibrationHealthLevel::Error,
                label: "Golden drift exceeded",
                detail: "The fresh zero exceeded the Golden drift limit and start is refused. Inspect the hardware; if the drift is legitimate, Commit a new Golden reference and Measure again.",
            },
            (Some(2), Some(1), _) => CalibrationHealth {
                level: CalibrationHealthLevel::Error,
                label: "Offset not applied",
                detail: "The zero measurement passed, but no measured or Golden offset is active. Start is refused.",
            },
            (Some(2), Some(2..), Some(1)) => CalibrationHealth {
                level: CalibrationHealthLevel::Error,
                label: "Measurement rejected",
                detail: "The automatic zero measurement was rejected and start is refused. The Golden offset remains active for display; run Measure again once the cause is cleared.",
            },
            (Some(2), Some(2..), _) => CalibrationHealth {
                level: CalibrationHealthLevel::Error,
                label: "Calibration unavailable",
                detail: "The automatic zero measurement was rejected and no Golden offset is active. Start is refused.",
            },
            (Some(2), _, _) => CalibrationHealth {
                level: CalibrationHealthLevel::Warning,
                label: "Result unavailable",
                detail: "Current-sensor calibration completed without a recognized result.",
            },
            (Some(_), _, _) => CalibrationHealth {
                level: CalibrationHealthLevel::Warning,
                label: "Unknown state",
                detail: "Viewer2000 reported an unknown current-sensor calibration state.",
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingCalibrationCommand {
    pub command: CalibrationCommand,
    pub sequence: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CalibrationCommandResult {
    Measure {
        sequence: u32,
        result: u16,
    },
    Commit {
        commit_sequence: u32,
    },
    Failed {
        command: CalibrationCommand,
        message: String,
    },
}

pub(crate) struct CalibrationState {
    pub pending: Option<PendingCalibrationCommand>,
    pub last_result: Option<CalibrationCommandResult>,
    pub show_commit_confirmation: bool,
    pub next_read: Instant,
}

impl CalibrationState {
    pub fn new() -> Self {
        Self {
            pending: None,
            last_result: None,
            show_commit_confirmation: false,
            next_read: Instant::now(),
        }
    }

    pub fn begin(&mut self, command: CalibrationCommand) {
        self.pending = Some(PendingCalibrationCommand {
            command,
            sequence: None,
        });
    }

    pub fn accept_measure(&mut self, sequence: u32) -> bool {
        let Some(pending) = &mut self.pending else {
            return false;
        };
        if pending.command != CalibrationCommand::MeasureZero {
            return false;
        }
        pending.sequence = Some(sequence);
        true
    }

    pub fn complete_measure_from_status(
        &mut self,
        status: &DeviceStatus,
    ) -> Option<CalibrationCommandResult> {
        let pending = self.pending?;
        if pending.command != CalibrationCommand::MeasureZero {
            return None;
        }
        let sequence = pending.sequence?;
        if status.command_ack_seq != Some(sequence) {
            return None;
        }
        let result = CalibrationCommandResult::Measure {
            sequence,
            result: status.command_result.unwrap_or_default(),
        };
        self.pending = None;
        self.last_result = Some(result.clone());
        Some(result)
    }

    pub fn complete_commit(&mut self, commit_sequence: u32) -> CalibrationCommandResult {
        let result = CalibrationCommandResult::Commit { commit_sequence };
        self.pending = None;
        self.last_result = Some(result.clone());
        result
    }

    pub fn fail(
        &mut self,
        command: CalibrationCommand,
        message: String,
    ) -> CalibrationCommandResult {
        let result = CalibrationCommandResult::Failed { command, message };
        if self
            .pending
            .is_some_and(|pending| pending.command == command)
        {
            self.pending = None;
        }
        self.last_result = Some(result.clone());
        result
    }

    pub fn fail_pending(&mut self, message: String) -> Option<CalibrationCommandResult> {
        let command = self.pending?.command;
        Some(self.fail(command, message))
    }

    pub fn reset_session(&mut self) {
        self.pending = None;
        self.show_commit_confirmation = false;
        self.next_read = Instant::now();
    }
}

impl Default for CalibrationState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CalibrationGateInput {
    pub connected: bool,
    pub catalog_ready: bool,
    pub has_cal: bool,
    pub has_ct_zero_cal: bool,
    pub can_write_calibration: bool,
    pub system_state: Option<SystemState>,
    pub system_command_pending: bool,
    pub calibration_command_pending: bool,
    pub measurement_done_ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CalibrationGate {
    pub can_measure: bool,
    pub can_commit: bool,
    pub reason: Option<&'static str>,
}

pub(crate) fn calibration_gate(input: CalibrationGateInput) -> CalibrationGate {
    let reason = if !input.connected {
        Some("Not connected")
    } else if !input.catalog_ready {
        Some("Catalog not ready")
    } else if !input.has_cal {
        Some("CAL capability is not available")
    } else if !input.has_ct_zero_cal {
        Some("CT_ZERO_CAL capability is not available")
    } else if !input.can_write_calibration {
        Some("Calibration is blocked by project safety state")
    } else if input.system_state != Some(SystemState::Idle) {
        Some("User system is not IDLE")
    } else if input.system_command_pending {
        Some("A system command is pending")
    } else if input.calibration_command_pending {
        Some("A calibration command is pending")
    } else {
        None
    };

    let can_measure = reason.is_none();
    let can_commit = can_measure && input.measurement_done_ok;
    CalibrationGate {
        can_measure,
        can_commit,
        reason: if can_commit || can_measure {
            None
        } else {
            reason
        },
    }
}

pub(crate) fn cal_state_label(value: Option<u16>) -> String {
    match value {
        Some(0) => "IDLE".to_owned(),
        Some(1) => "MEASURING".to_owned(),
        Some(2) => "DONE".to_owned(),
        Some(other) => format!("STATE {other}"),
        None => "UNKNOWN".to_owned(),
    }
}

pub(crate) fn cal_result_label(value: Option<u16>) -> String {
    match value {
        Some(0) => "NONE".to_owned(),
        Some(1) => "OK".to_owned(),
        Some(2) => "IMPLAUSIBLE".to_owned(),
        Some(3) => "NOISY".to_owned(),
        Some(4) => "ABORTED".to_owned(),
        Some(5) => "UNSTABLE".to_owned(),
        Some(other) => format!("RESULT {other}"),
        None => "UNKNOWN".to_owned(),
    }
}

pub(crate) fn applied_source_label(value: Option<u16>) -> String {
    match value {
        Some(0) => "DEFAULT".to_owned(),
        Some(1) => "STORED".to_owned(),
        Some(2) => "MEASURED".to_owned(),
        Some(other) => format!("SOURCE {other}"),
        None => "UNKNOWN".to_owned(),
    }
}

pub(crate) fn store_result_label(value: Option<u16>) -> String {
    match value {
        Some(0) => "NONE".to_owned(),
        Some(1) => "OK".to_owned(),
        Some(2) => "ERASE_FAIL".to_owned(),
        Some(3) => "PROG_FAIL".to_owned(),
        Some(4) => "VERIFY_FAIL".to_owned(),
        Some(5) => "FULL".to_owned(),
        Some(other) => format!("STORE_RESULT {other}"),
        None => "UNKNOWN".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{DeviceStatus, ScopeMode};

    fn idle_status(sequence: Option<u32>, result: Option<u16>) -> DeviceStatus {
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
            command_ack_seq: sequence,
            command_result: result,
            performance: None,
            scope_state_seq: 0,
            scope_frozen_count: 0,
            scope_trigger_tick: 0,
            scope_bind_seq: 0,
        }
    }

    #[test]
    fn measure_completion_requires_matching_sequence() {
        let mut state = CalibrationState::new();
        state.begin(CalibrationCommand::MeasureZero);
        assert!(state.accept_measure(10));
        assert!(
            state
                .complete_measure_from_status(&idle_status(Some(9), Some(0)))
                .is_none()
        );

        let completed = state
            .complete_measure_from_status(&idle_status(Some(10), Some(5)))
            .expect("matching sequence completes");

        assert_eq!(
            completed,
            CalibrationCommandResult::Measure {
                sequence: 10,
                result: 5
            }
        );
        assert!(state.pending.is_none());
    }

    #[test]
    fn gate_requires_idle_capable_safe_device() {
        let ready = CalibrationGateInput {
            connected: true,
            catalog_ready: true,
            has_cal: true,
            has_ct_zero_cal: true,
            can_write_calibration: true,
            system_state: Some(SystemState::Idle),
            system_command_pending: false,
            calibration_command_pending: false,
            measurement_done_ok: true,
        };

        assert_eq!(
            calibration_gate(ready),
            CalibrationGate {
                can_measure: true,
                can_commit: true,
                reason: None
            }
        );
        assert_eq!(
            calibration_gate(CalibrationGateInput {
                measurement_done_ok: false,
                ..ready
            })
            .can_commit,
            false
        );
        assert_eq!(
            calibration_gate(CalibrationGateInput {
                system_state: Some(SystemState::Running),
                ..ready
            })
            .reason,
            Some("User system is not IDLE")
        );
    }

    #[test]
    fn calibration_labels_name_known_values() {
        assert_eq!(cal_state_label(Some(1)), "MEASURING");
        assert_eq!(cal_result_label(Some(5)), "UNSTABLE");
        assert_eq!(applied_source_label(Some(2)), "MEASURED");
        assert_eq!(store_result_label(Some(3)), "PROG_FAIL");
    }

    #[test]
    fn calibration_health_distinguishes_normal_fallback_and_failure() {
        let normal = CalibrationSnapshot {
            state: Some(2),
            result: Some(1),
            applied_source: Some(2),
            store_valid: Some(1),
            store_result: Some(0),
            store_sequence: Some(3),
        };
        assert_eq!(normal.health().level, CalibrationHealthLevel::Normal);
        assert!(normal.start_ready());

        let no_golden = CalibrationSnapshot {
            store_valid: Some(0),
            ..normal
        };
        assert_eq!(no_golden.health().level, CalibrationHealthLevel::Warning);

        let drifted = CalibrationSnapshot {
            applied_source: Some(1),
            ..normal
        };
        assert_eq!(drifted.health().level, CalibrationHealthLevel::Error);
        assert_eq!(drifted.health().label, "Golden drift exceeded");

        let rejected_with_fallback = CalibrationSnapshot {
            result: Some(5),
            applied_source: Some(1),
            ..normal
        };
        // The firmware refuses APP_START on a rejected measurement even with
        // the Golden fallback active, so this is an error, not a warning.
        assert_eq!(
            rejected_with_fallback.health().level,
            CalibrationHealthLevel::Error
        );

        let rejected_without_fallback = CalibrationSnapshot {
            applied_source: Some(0),
            ..rejected_with_fallback
        };
        assert_eq!(
            rejected_without_fallback.health().level,
            CalibrationHealthLevel::Error
        );

        let flash_failure = CalibrationSnapshot {
            store_result: Some(4),
            ..normal
        };
        assert_eq!(flash_failure.health().level, CalibrationHealthLevel::Error);

        let pending = CalibrationSnapshot {
            state: Some(0),
            ..CalibrationSnapshot::default()
        };
        assert_eq!(pending.health().level, CalibrationHealthLevel::Warning);
        assert!(!pending.start_ready());
    }
}
