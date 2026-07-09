use std::time::{Duration, Instant};

pub(crate) const SRM_OPEN_LOOP_READ_PERIOD: Duration = Duration::from_millis(250);
pub(crate) const SRM_OPEN_LOOP_STATUS_READ_PERIOD: Duration = Duration::from_secs(1);
pub(crate) const SRM_OPEN_LOOP_STATUS_READ_NAMES: &[&str] =
    &["v2k_srm_open_loop.state", "v2k_srm_open_loop.result"];
pub(crate) const SRM_OPEN_LOOP_READ_NAMES: &[&str] = &[
    "v2k_srm_open_loop.state",
    "v2k_srm_open_loop.result",
    "v2k_srm_open_loop.dc_v",
    "v2k_srm_open_loop.peak_duty",
    "v2k_srm_open_loop.ticks",
    "v2k_srm_open_loop.eqep_errors",
];

const SRM_OPEN_LOOP_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SrmOpenLoopPhase {
    #[default]
    Idle,
    StartingSystem,
    RequestingOpenLoop,
    Running,
    StoppingSystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SrmOpenLoopHealthLevel {
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SrmOpenLoopHealth {
    pub level: SrmOpenLoopHealthLevel,
    pub label: &'static str,
    pub detail: &'static str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SrmOpenLoopSnapshot {
    pub state: Option<u16>,
    pub result: Option<u16>,
    pub dc_v: Option<f64>,
    pub peak_duty: Option<f64>,
    pub ticks: Option<u32>,
    pub eqep_errors: Option<u32>,
}

impl SrmOpenLoopSnapshot {
    pub fn running(self) -> bool {
        self.state == Some(1)
    }

    pub fn status_health(self) -> Option<SrmOpenLoopHealth> {
        if self.running() {
            return Some(SrmOpenLoopHealth {
                level: SrmOpenLoopHealthLevel::Warning,
                label: "SRM Open-loop Running",
                detail: "Powered SRM ABZ requalification is running.",
            });
        }

        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingSrmOpenLoopCommand {
    pub sequence: Option<u32>,
    sent_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SrmOpenLoopCommandResult {
    Completed { sequence: Option<u32>, result: u16 },
    Failed { message: String },
    Expired { sequence: Option<u32> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SrmOpenLoopGate {
    pub can_run: bool,
    pub reason: Option<&'static str>,
}

pub(crate) struct SrmOpenLoopState {
    pub phase: SrmOpenLoopPhase,
    pub pending: Option<PendingSrmOpenLoopCommand>,
    pub command_sequence: Option<u32>,
    pub last_result: Option<SrmOpenLoopCommandResult>,
    pub next_read: Instant,
    pub show_run_confirmation: bool,
    observed_running: bool,
}

impl SrmOpenLoopState {
    pub fn new() -> Self {
        Self {
            phase: SrmOpenLoopPhase::Idle,
            pending: None,
            command_sequence: None,
            last_result: None,
            next_read: Instant::now(),
            show_run_confirmation: false,
            observed_running: false,
        }
    }

    pub fn active(&self) -> bool {
        self.phase != SrmOpenLoopPhase::Idle
    }

    pub fn begin_workflow(&mut self) {
        self.phase = SrmOpenLoopPhase::StartingSystem;
        self.pending = None;
        self.command_sequence = None;
        self.last_result = None;
        self.next_read = Instant::now();
        self.observed_running = false;
    }

    pub fn begin_open_loop_command(&mut self, now: Instant) {
        self.phase = SrmOpenLoopPhase::RequestingOpenLoop;
        self.pending = Some(PendingSrmOpenLoopCommand {
            sequence: None,
            sent_at: now,
        });
        self.observed_running = false;
    }

    pub fn accept(&mut self, sequence: u32, now: Instant) -> bool {
        let Some(pending) = &mut self.pending else {
            return false;
        };
        pending.sequence = Some(sequence);
        if now.saturating_duration_since(pending.sent_at) >= SRM_OPEN_LOOP_COMMAND_TIMEOUT {
            return false;
        }
        self.command_sequence = Some(sequence);
        self.pending = None;
        self.phase = SrmOpenLoopPhase::Running;
        true
    }

    pub fn complete_when_abz_ready(&mut self) -> Option<SrmOpenLoopCommandResult> {
        if self.phase != SrmOpenLoopPhase::Running {
            return None;
        }
        let result = SrmOpenLoopCommandResult::Completed {
            sequence: self.command_sequence,
            result: 1,
        };
        self.phase = SrmOpenLoopPhase::StoppingSystem;
        self.pending = None;
        self.last_result = Some(result.clone());
        Some(result)
    }

    pub fn fail_if_open_loop_ended_before_abz_ready(
        &mut self,
        snapshot: SrmOpenLoopSnapshot,
    ) -> Option<SrmOpenLoopCommandResult> {
        if !matches!(
            self.phase,
            SrmOpenLoopPhase::RequestingOpenLoop | SrmOpenLoopPhase::Running
        ) {
            return None;
        }
        if snapshot.running() {
            self.observed_running = true;
            return None;
        }
        let result_code = snapshot.result?;
        if result_code == 0 {
            return None;
        }
        if !self.observed_running && matches!(result_code, 1 | 4 | 5) {
            return None;
        }
        let message = if result_code == 1 {
            "SRM Open-loop ABZ ended before ABZ Zeroing became ready".to_owned()
        } else {
            format!(
                "SRM Open-loop ABZ ended {} before ABZ Zeroing became ready",
                srm_open_loop_result_label(Some(result_code))
            )
        };
        let result = SrmOpenLoopCommandResult::Failed { message };
        self.phase = SrmOpenLoopPhase::StoppingSystem;
        self.pending = None;
        self.last_result = Some(result.clone());
        Some(result)
    }

    pub fn expire_pending(&mut self, now: Instant) -> Option<SrmOpenLoopCommandResult> {
        let pending = self.pending?;
        if now.saturating_duration_since(pending.sent_at) < SRM_OPEN_LOOP_COMMAND_TIMEOUT {
            return None;
        }
        let result = SrmOpenLoopCommandResult::Expired {
            sequence: pending.sequence,
        };
        self.phase = SrmOpenLoopPhase::StoppingSystem;
        self.pending = None;
        self.last_result = Some(result.clone());
        Some(result)
    }

    pub fn fail(&mut self, message: String) -> SrmOpenLoopCommandResult {
        let result = SrmOpenLoopCommandResult::Failed { message };
        self.phase = SrmOpenLoopPhase::Idle;
        self.pending = None;
        self.observed_running = false;
        self.last_result = Some(result.clone());
        result
    }

    pub fn fail_and_stop(&mut self, message: String) -> Option<SrmOpenLoopCommandResult> {
        if !self.active() {
            return None;
        }
        let result = SrmOpenLoopCommandResult::Failed { message };
        self.phase = SrmOpenLoopPhase::StoppingSystem;
        self.pending = None;
        self.observed_running = false;
        self.last_result = Some(result.clone());
        Some(result)
    }

    pub fn finish_stop(&mut self) {
        self.phase = SrmOpenLoopPhase::Idle;
        self.pending = None;
        self.command_sequence = None;
        self.observed_running = false;
    }

    pub fn fail_pending(&mut self, message: String) -> Option<SrmOpenLoopCommandResult> {
        if !self.active() {
            return None;
        }
        Some(self.fail(message))
    }

    pub fn reset_session(&mut self) {
        self.phase = SrmOpenLoopPhase::Idle;
        self.pending = None;
        self.command_sequence = None;
        self.next_read = Instant::now();
        self.show_run_confirmation = false;
        self.observed_running = false;
    }
}

impl Default for SrmOpenLoopState {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn srm_open_loop_result_label(value: Option<u16>) -> String {
    match value {
        Some(0) => "NONE".to_owned(),
        Some(1) => "OK".to_owned(),
        Some(2) => "BAD_STATE".to_owned(),
        Some(3) => "BAD_DC".to_owned(),
        Some(4) => "EQEP".to_owned(),
        Some(5) => "CANCELLED".to_owned(),
        Some(other) => format!("RESULT {other}"),
        None => "UNKNOWN".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_name_known_open_loop_values() {
        assert_eq!(srm_open_loop_result_label(Some(0)), "NONE");
        assert_eq!(srm_open_loop_result_label(Some(1)), "OK");
        assert_eq!(srm_open_loop_result_label(Some(2)), "BAD_STATE");
        assert_eq!(srm_open_loop_result_label(Some(3)), "BAD_DC");
        assert_eq!(srm_open_loop_result_label(Some(4)), "EQEP");
        assert_eq!(srm_open_loop_result_label(Some(5)), "CANCELLED");
    }

    #[test]
    fn health_reports_running_and_failure_values() {
        let running = SrmOpenLoopSnapshot {
            state: Some(1),
            result: Some(0),
            ..SrmOpenLoopSnapshot::default()
        };
        assert_eq!(
            running.status_health().expect("running health").level,
            SrmOpenLoopHealthLevel::Warning
        );

        let cancelled = SrmOpenLoopSnapshot {
            state: Some(0),
            result: Some(5),
            ..SrmOpenLoopSnapshot::default()
        };
        assert_eq!(cancelled.status_health(), None);

        let idle = SrmOpenLoopSnapshot {
            state: Some(0),
            result: Some(0),
            ..SrmOpenLoopSnapshot::default()
        };
        assert_eq!(idle.status_health(), None);
    }

    #[test]
    fn command_ack_only_moves_workflow_to_running() {
        let now = Instant::now();
        let mut state = SrmOpenLoopState::new();
        state.begin_workflow();
        state.begin_open_loop_command(now);
        assert!(state.accept(9, now));

        assert_eq!(state.phase, SrmOpenLoopPhase::Running);
        assert_eq!(state.command_sequence, Some(9));
        assert!(state.pending.is_none());
        assert_eq!(state.last_result, None);
    }

    #[test]
    fn abz_ready_completes_workflow_and_requests_stop() {
        let now = Instant::now();
        let mut state = SrmOpenLoopState::new();
        state.begin_workflow();
        state.begin_open_loop_command(now);
        assert!(state.accept(9, now));

        assert!(
            state
                .fail_if_open_loop_ended_before_abz_ready(SrmOpenLoopSnapshot {
                    state: Some(1),
                    result: Some(0),
                    ..SrmOpenLoopSnapshot::default()
                })
                .is_none()
        );

        let completed = state
            .complete_when_abz_ready()
            .expect("ABZ ready completes");
        assert_eq!(
            completed,
            SrmOpenLoopCommandResult::Completed {
                sequence: Some(9),
                result: 1
            }
        );
        assert_eq!(state.phase, SrmOpenLoopPhase::StoppingSystem);
    }

    #[test]
    fn stale_terminal_result_is_ignored_until_running_is_observed() {
        let now = Instant::now();
        let mut state = SrmOpenLoopState::new();
        state.begin_workflow();
        state.begin_open_loop_command(now);
        assert!(state.accept(9, now));

        assert_eq!(
            state.fail_if_open_loop_ended_before_abz_ready(SrmOpenLoopSnapshot {
                state: Some(0),
                result: Some(5),
                ticks: Some(99),
                ..SrmOpenLoopSnapshot::default()
            }),
            None
        );
        assert_eq!(state.phase, SrmOpenLoopPhase::Running);

        assert!(
            state
                .fail_if_open_loop_ended_before_abz_ready(SrmOpenLoopSnapshot {
                    state: Some(1),
                    result: Some(0),
                    ticks: Some(1),
                    ..SrmOpenLoopSnapshot::default()
                })
                .is_none()
        );
        assert!(matches!(
            state.fail_if_open_loop_ended_before_abz_ready(SrmOpenLoopSnapshot {
                state: Some(0),
                result: Some(5),
                ticks: Some(2),
                ..SrmOpenLoopSnapshot::default()
            }),
            Some(SrmOpenLoopCommandResult::Failed { .. })
        ));
    }

    #[test]
    fn open_loop_ok_without_abz_ready_is_not_success() {
        let now = Instant::now();
        let mut state = SrmOpenLoopState::new();
        state.begin_workflow();
        state.begin_open_loop_command(now);
        assert!(state.accept(9, now));
        assert!(
            state
                .fail_if_open_loop_ended_before_abz_ready(SrmOpenLoopSnapshot {
                    state: Some(1),
                    result: Some(0),
                    ticks: Some(1),
                    ..SrmOpenLoopSnapshot::default()
                })
                .is_none()
        );

        assert!(matches!(
            state.fail_if_open_loop_ended_before_abz_ready(SrmOpenLoopSnapshot {
                state: Some(2),
                result: Some(1),
                ticks: Some(2),
                ..SrmOpenLoopSnapshot::default()
            }),
            Some(SrmOpenLoopCommandResult::Failed { .. })
        ));
        assert_eq!(state.phase, SrmOpenLoopPhase::StoppingSystem);
    }
}
