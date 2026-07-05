use std::time::{Duration, Instant};

use crate::source::DeviceStatus;

pub(crate) const ABZ_ZEROING_READ_PERIOD: Duration = Duration::from_millis(250);
pub(crate) const ABZ_ZEROING_STATUS_READ_PERIOD: Duration = Duration::from_secs(1);
pub(crate) const ABZ_ZEROING_STATUS_READ_NAMES: &[&str] = &["v2k_abz_zeroing.ready"];
pub(crate) const ABZ_ZEROING_READ_NAMES: &[&str] = &[
    "v2k_abz_zeroing.ready",
    "v2k_abz_zeroing.state",
    "v2k_abz_zeroing.result",
    "v2k_abz_zeroing.block_reason",
    "v2k_abz_zeroing.attempt_seq",
    "v2k_abz.eqep2.raw_count",
    "v2k_abz.eqep2.index_count",
    "v2k_abz.eqep2.index_latch",
    "v2k_abz.eqep2.index_event",
    "v2k_abz.eqep2.dir_change",
    "v2k_abz.eqep2.status",
    "v2k_abz.eqep2.error_flags",
    "v2k_abz_zeroing.npe.z_good",
    "v2k_abz_zeroing.npe.z_seen",
    "v2k_abz_zeroing.npe.z_rejects",
    "v2k_abz_zeroing.npe.first_latch",
    "v2k_abz_zeroing.npe.last_latch",
    "v2k_abz_zeroing.npe.last_reject_latch",
    "v2k_abz_zeroing.npe.dir_changes",
    "v2k_abz_zeroing.npe.dir_resets",
    "v2k_abz_zeroing.npe.error_resets",
    "v2k_abz_zeroing.npe.last_error_flags",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AbzZeroingHealthLevel {
    Normal,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AbzZeroingHealth {
    pub level: AbzZeroingHealthLevel,
    pub label: &'static str,
    pub detail: &'static str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AbzZeroingSnapshot {
    pub ready: Option<u16>,
    pub state: Option<u16>,
    pub result: Option<u16>,
    pub block_reason: Option<u16>,
    pub attempt_sequence: Option<u32>,
    pub eqep2_raw_count: Option<u16>,
    pub eqep2_index_count: Option<u32>,
    pub eqep2_index_latch: Option<u16>,
    pub eqep2_index_event: Option<u16>,
    pub eqep2_dir_change: Option<u16>,
    pub eqep2_status: Option<u16>,
    pub eqep2_error_flags: Option<u32>,
    pub npe_z_good: Option<u16>,
    pub npe_z_seen: Option<u32>,
    pub npe_z_rejects: Option<u32>,
    pub npe_first_latch: Option<u16>,
    pub npe_last_latch: Option<u16>,
    pub npe_last_reject_latch: Option<u16>,
    pub npe_dir_changes: Option<u32>,
    pub npe_dir_resets: Option<u32>,
    pub npe_error_resets: Option<u32>,
    pub npe_last_error_flags: Option<u32>,
}

impl AbzZeroingSnapshot {
    pub fn health(self) -> AbzZeroingHealth {
        if self.ready == Some(1) || matches!((self.state, self.result), (Some(2), Some(1))) {
            return AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Normal,
                label: "Ready",
                detail: "ABZ angle-reference zeroing is ready.",
            };
        }

        if self.ready == Some(0) && matches!(self.state, Some(0) | None) {
            if self.eqep2_error_flags.unwrap_or_default() != 0
                || self.npe_error_resets.unwrap_or_default() != 0
                || self.npe_last_error_flags.unwrap_or_default() != 0
            {
                return AbzZeroingHealth {
                    level: AbzZeroingHealthLevel::Error,
                    label: "eQEP Error",
                    detail: "The passive ABZ observer is seeing eQEP error flags.",
                };
            }
            if self.npe_dir_resets.unwrap_or_default() != 0 {
                return AbzZeroingHealth {
                    level: AbzZeroingHealthLevel::Warning,
                    label: "Direction Reset",
                    detail: "Z events were observed with inconsistent direction.",
                };
            }
            if self.npe_z_rejects.unwrap_or_default() != 0 {
                return AbzZeroingHealth {
                    level: AbzZeroingHealthLevel::Warning,
                    label: "Z Repeat Check",
                    detail: "Z events were observed but the index latch did not repeat yet.",
                };
            }
            return AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Warning,
                label: "Waiting for Z",
                detail: "The passive ABZ observer is waiting for two qualified Z events. Rotate the shaft externally; firmware will not energize the motor.",
            };
        }

        match (self.state, self.result, self.block_reason) {
            (None, _, _) if self.ready.is_none() => AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Normal,
                label: "Unavailable",
                detail: "ABZ zeroing status is not available.",
            },
            (Some(1), _, _) => AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Warning,
                label: "Zeroing",
                detail: "ABZ angle-reference zeroing is in progress.",
            },
            (Some(3), _, Some(6)) => AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Error,
                label: "Timeout",
                detail: "ABZ zeroing timed out before the reference became ready.",
            },
            (Some(3), _, _) | (_, Some(2), _) => AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Error,
                label: "Failed",
                detail: "ABZ zeroing failed. Inspect the reported diagnostics.",
            },
            (_, Some(3), _) => AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Warning,
                label: "Cancelled",
                detail: "ABZ zeroing was cancelled before the reference became ready.",
            },
            (Some(0), _, _) | (None, _, _) if self.ready == Some(0) => AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Warning,
                label: "Required",
                detail: "ABZ angle-reference zeroing is required before Start.",
            },
            (Some(_), _, _) => AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Warning,
                label: "Unknown state",
                detail: "Viewer2000 reported an unknown ABZ zeroing state.",
            },
            (None, _, _) => AbzZeroingHealth {
                level: AbzZeroingHealthLevel::Warning,
                label: "Required",
                detail: "ABZ angle-reference zeroing is not ready.",
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingAbzZeroingCommand {
    pub sequence: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AbzZeroingCommandResult {
    Completed { sequence: u32, result: u16 },
    Failed { message: String },
}

pub(crate) struct AbzZeroingState {
    pub pending: Option<PendingAbzZeroingCommand>,
    pub last_result: Option<AbzZeroingCommandResult>,
    pub next_read: Instant,
}

impl AbzZeroingState {
    pub fn new() -> Self {
        Self {
            pending: None,
            last_result: None,
            next_read: Instant::now(),
        }
    }

    #[cfg(test)]
    pub fn accept(&mut self, sequence: u32) -> bool {
        let Some(pending) = &mut self.pending else {
            return false;
        };
        pending.sequence = Some(sequence);
        true
    }

    pub fn complete_from_status(
        &mut self,
        status: &DeviceStatus,
    ) -> Option<AbzZeroingCommandResult> {
        let sequence = self.pending?.sequence?;
        if status.command_ack_seq != Some(sequence) {
            return None;
        }
        let result = AbzZeroingCommandResult::Completed {
            sequence,
            result: status.command_result.unwrap_or_default(),
        };
        self.pending = None;
        self.last_result = Some(result.clone());
        Some(result)
    }

    pub fn fail(&mut self, message: String) -> AbzZeroingCommandResult {
        let result = AbzZeroingCommandResult::Failed { message };
        self.pending = None;
        self.last_result = Some(result.clone());
        result
    }

    pub fn fail_pending(&mut self, message: String) -> Option<AbzZeroingCommandResult> {
        self.pending?;
        Some(self.fail(message))
    }

    pub fn reset_session(&mut self) {
        self.pending = None;
        self.next_read = Instant::now();
    }
}

impl Default for AbzZeroingState {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn abz_zeroing_state_label(value: Option<u16>) -> String {
    match value {
        Some(0) => "REQUIRED".to_owned(),
        Some(1) => "ZEROING".to_owned(),
        Some(2) => "READY".to_owned(),
        Some(3) => "FAILED".to_owned(),
        Some(other) => format!("STATE {other}"),
        None => "UNKNOWN".to_owned(),
    }
}

pub(crate) fn abz_zeroing_result_label(value: Option<u16>) -> String {
    match value {
        Some(0) => "NONE".to_owned(),
        Some(1) => "OK".to_owned(),
        Some(2) => "FAILED".to_owned(),
        Some(3) => "CANCELLED".to_owned(),
        Some(other) => format!("RESULT {other}"),
        None => "UNKNOWN".to_owned(),
    }
}

pub(crate) fn abz_zeroing_block_label(value: Option<u16>) -> String {
    match value {
        Some(0) => "NONE".to_owned(),
        Some(other) => format!("BLOCK {other}"),
        None => "UNKNOWN".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{ScopeMode, SystemState};

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
    fn completion_requires_matching_status_sequence() {
        let mut state = AbzZeroingState::new();
        state.pending = Some(PendingAbzZeroingCommand { sequence: None });
        assert!(state.accept(10));
        assert!(
            state
                .complete_from_status(&idle_status(Some(9), Some(0)))
                .is_none()
        );

        let completed = state
            .complete_from_status(&idle_status(Some(10), Some(2)))
            .expect("matching sequence completes");

        assert_eq!(
            completed,
            AbzZeroingCommandResult::Completed {
                sequence: 10,
                result: 2
            }
        );
        assert!(state.pending.is_none());
    }

    #[test]
    fn snapshot_health_tracks_ready_required_and_failed_states() {
        let ready = AbzZeroingSnapshot {
            ready: Some(1),
            state: Some(2),
            result: Some(1),
            ..AbzZeroingSnapshot::default()
        };
        assert_eq!(ready.health().level, AbzZeroingHealthLevel::Normal);

        let required = AbzZeroingSnapshot {
            ready: Some(0),
            state: Some(0),
            result: Some(0),
            block_reason: Some(0),
            ..AbzZeroingSnapshot::default()
        };
        assert_eq!(required.health().level, AbzZeroingHealthLevel::Warning);
        assert_eq!(required.health().label, "Waiting for Z");

        let failed = AbzZeroingSnapshot {
            ready: Some(0),
            state: Some(3),
            result: Some(2),
            block_reason: Some(6),
            ..AbzZeroingSnapshot::default()
        };
        assert_eq!(failed.health().level, AbzZeroingHealthLevel::Error);
    }

    #[test]
    fn labels_name_known_values() {
        assert_eq!(abz_zeroing_state_label(Some(1)), "ZEROING");
        assert_eq!(abz_zeroing_result_label(Some(3)), "CANCELLED");
        assert_eq!(abz_zeroing_block_label(Some(5)), "BLOCK 5");
    }
}
