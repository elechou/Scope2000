use std::time::Instant;

use crate::app::ScopeApp;
use crate::app::state::{
    CALIBRATION_READ_NAMES, CALIBRATION_READ_PERIOD, CALIBRATION_STATUS_READ_NAMES,
    CALIBRATION_STATUS_READ_PERIOD, CalibrationGate, CalibrationGateInput, CalibrationSnapshot,
    calibration_gate,
};
use crate::console::LogLevel;
use crate::source::{
    CAL_READ_MAX, CAP_CAL, CAP_CT_ZERO_CAL, CAP_NATIVE_BLOCK, CAP_PRE_TRIGGER, CAP_SCOPE_CAPTURE,
    CAP_SCOPE_STREAM, CAP_SYSTEM_CMD, CalibrationCommand, CatalogCommand, NO_CAPTURE_ACK,
    ParamWrite, ScopeConfig, ScopeMode, SourceCommand, SystemCommand, TriggerEdge, ValueRead,
    VarDescriptor,
};
use crate::wave::{max_record_points_for_binding, pane::PaneKind, scope_channel_limit};

impl ScopeApp {
    pub(in crate::app) fn send(&self, command: SourceCommand) {
        let _ = self.source.commands.send(command);
    }

    fn send_catalog(&self, command: CatalogCommand) {
        let Some(info) = &self.hardware.info else {
            return;
        };
        self.send(SourceCommand::Catalog {
            build_hash: info.build_hash,
            command,
        });
    }

    pub(in crate::app) fn send_system_command(&mut self, command: SystemCommand) {
        if self.hardware.pending_system_command.is_some() {
            self.log.push(
                LogLevel::Warn,
                "A system command is already pending".to_owned(),
            );
            return;
        }
        if !self.hardware.connected {
            self.log.push(LogLevel::Warn, "Not connected".to_owned());
            return;
        }
        if !self.has_capability(CAP_SYSTEM_CMD) {
            self.log.push(
                LogLevel::Warn,
                "SYSTEM_CMD capability is not available".to_owned(),
            );
            return;
        }
        self.hardware.begin_system_command(command);
        self.send(SourceCommand::SystemCommand(command));
    }

    pub(in crate::app) fn current_sensor_calibration_gate(&self) -> CalibrationGate {
        calibration_gate(CalibrationGateInput {
            connected: self.hardware.connected,
            catalog_ready: self.descriptor_catalog_ready,
            has_cal: self.has_capability(CAP_CAL),
            has_ct_zero_cal: self.has_capability(CAP_CT_ZERO_CAL),
            can_write_calibration: self.project_policy().calibration_write,
            system_state: self
                .hardware
                .status
                .as_ref()
                .map(|status| status.system_state),
            system_command_pending: self.hardware.pending_system_command.is_some(),
            calibration_command_pending: self.calibration.pending.is_some(),
            measurement_done_ok: self.calibration_measurement_done_ok(),
        })
    }

    pub(in crate::app) fn send_calibration_command(&mut self, command: CalibrationCommand) {
        let gate = self.current_sensor_calibration_gate();
        let allowed = match command {
            CalibrationCommand::MeasureZero => gate.can_measure,
            CalibrationCommand::CommitToFlash => gate.can_commit,
        };
        if !allowed {
            let reason = gate
                .reason
                .unwrap_or("No passing measurement available to commit");
            self.log.push(LogLevel::Warn, reason.to_owned());
            return;
        }
        self.calibration.begin(command);
        self.send(SourceCommand::CalibrationCommand(command));
        self.log.push(
            LogLevel::Info,
            format!("Calibration command {} sent", command.label()),
        );
    }

    pub(in crate::app) fn connect(&mut self) {
        let Some(endpoint) = self.hardware.endpoint() else {
            self.log
                .push(LogLevel::Warn, "Select a serial port first".to_owned());
            return;
        };
        self.hardware.connecting = true;
        self.workspace = self.snapshot_workspace();
        self.workspace_watch_restored = false;
        self.descriptor_catalog_ready = false;
        self.hardware.info = None;
        self.hardware.status = None;
        self.hardware.performance.clear();
        self.hardware.clear_system_command_state();
        self.send(SourceCommand::Connect(endpoint));
    }

    pub(in crate::app) fn disconnect_or_warn(&mut self) {
        if self.hardware.is_running() {
            self.ui.stop_warning_action = Some("Disconnect");
        } else {
            self.send(SourceCommand::Disconnect);
        }
    }

    pub(in crate::app) fn poll_watch_reads(&mut self) {
        if !self.hardware.connected || !self.has_capability(CAP_CAL) {
            return;
        }
        let now = Instant::now();
        if now >= self.next_watch_read {
            for reads in self.inspector.read_batches() {
                self.send_catalog(CatalogCommand::ReadValues(reads));
            }
            self.next_watch_read = now + super::super::WATCH_READ_PERIOD;
        }
    }

    pub(in crate::app) fn poll_current_sensor_calibration_reads(&mut self) {
        if !self.hardware.connected
            || !self.descriptor_catalog_ready
            || !self.has_capability(CAP_CAL)
            || !self.has_capability(CAP_CT_ZERO_CAL)
        {
            return;
        }
        let now = Instant::now();
        if now < self.calibration.next_read {
            return;
        }

        let names = if self.ui.show_current_sensor_calibration {
            CALIBRATION_READ_NAMES
        } else {
            CALIBRATION_STATUS_READ_NAMES
        };
        let reads: Vec<ValueRead> = names
            .iter()
            .filter_map(|name| {
                let descriptor_index = self.inspector.index_by_name(name)?;
                let descriptor = self.inspector.descriptors.get(descriptor_index)?;
                Some(ValueRead {
                    descriptor_index,
                    var: descriptor.var,
                })
            })
            .collect();
        if !reads.is_empty() {
            self.send_catalog(CatalogCommand::ReadValues(reads));
        }
        let period = if self.ui.show_current_sensor_calibration {
            CALIBRATION_READ_PERIOD
        } else {
            CALIBRATION_STATUS_READ_PERIOD
        };
        self.calibration.next_read = now + period;
    }

    pub(in crate::app) fn current_sensor_calibration_snapshot(&self) -> CalibrationSnapshot {
        CalibrationSnapshot {
            state: self.calibration_value_u16("v2k_cal.state"),
            result: self.calibration_value_u16("v2k_cal.result"),
            applied_source: self.calibration_value_u16("v2k_cal.applied_src"),
            store_valid: self.calibration_value_u16("v2k_cal.store_valid"),
            store_result: self.calibration_value_u16("v2k_cal.store_result"),
            store_sequence: self.calibration_value_u32("v2k_cal.store_seq"),
        }
    }

    pub(in crate::app) fn write_variables(&mut self, writes: Vec<(usize, f64)>) {
        if !self.project_policy().calibration_write {
            self.log.push(
                LogLevel::Warn,
                "Variable write blocked by project safety state".to_owned(),
            );
            return;
        }
        if !self.hardware.connected || !self.has_capability(CAP_CAL) {
            self.log
                .push(LogLevel::Warn, "CAL capability is not available".to_owned());
            return;
        }
        let read_values: Vec<ValueRead> = writes
            .iter()
            .filter_map(|(index, _)| {
                self.inspector
                    .descriptors
                    .get(*index)
                    .map(|descriptor| ValueRead {
                        descriptor_index: *index,
                        var: descriptor.var,
                    })
            })
            .collect();
        let param_writes: Vec<ParamWrite> = writes
            .iter()
            .filter_map(|(index, value)| self.inspector.param_write_for(*index, *value))
            .collect();
        if param_writes.is_empty() {
            return;
        }
        if param_writes.len() > 16 {
            self.log.push(
                LogLevel::Error,
                format!(
                    "Parameter commit rejected: {} writes exceed the native batch limit of 16",
                    param_writes.len()
                ),
            );
            return;
        }
        self.send_catalog(CatalogCommand::WriteParams(param_writes));
        self.send_catalog(CatalogCommand::CommitParams);
        for reads in read_values.chunks(CAL_READ_MAX) {
            self.send_catalog(CatalogCommand::ReadValues(reads.to_vec()));
        }
    }

    pub(in crate::app) fn start_acquisition(&mut self, mode: ScopeMode) {
        if !self.project_policy().wave_start {
            self.log.push(
                LogLevel::Warn,
                "Wave start blocked by project safety state".to_owned(),
            );
            return;
        }
        if !self.hardware.connected {
            self.log.push(LogLevel::Warn, "Not connected".to_owned());
            return;
        }
        if !self.has_capability(CAP_NATIVE_BLOCK) {
            self.log.push(
                LogLevel::Warn,
                "NATIVE_BLOCK capability is not available".to_owned(),
            );
            return;
        }
        if mode == ScopeMode::Stream && !self.has_capability(CAP_SCOPE_STREAM) {
            self.log.push(
                LogLevel::Warn,
                "SCOPE_STREAM capability is not available".to_owned(),
            );
            return;
        }
        if mode == ScopeMode::CaptureArmed && !self.has_capability(CAP_SCOPE_CAPTURE) {
            self.log.push(
                LogLevel::Warn,
                "SCOPE_CAPTURE capability is not available".to_owned(),
            );
            return;
        }
        if mode == ScopeMode::CaptureArmed && !self.has_capability(CAP_PRE_TRIGGER) {
            self.log.push(
                LogLevel::Warn,
                "Device does not declare PRE_TRIGGER capability".to_owned(),
            );
        }

        self.wave.settings.clamp();
        let pane_vars = self.collect_time_series_vars();
        let binding = self.resolve_scope_binding(&pane_vars);
        if binding.is_empty() {
            self.log.push(
                LogLevel::Warn,
                "Add scope-capable variables to a Time Series pane first".to_owned(),
            );
            return;
        }
        if let Some(trigger_source) = &self.wave.settings.trigger_source
            && !binding
                .iter()
                .any(|descriptor| &descriptor.name == trigger_source)
        {
            self.log.push(
                LogLevel::Warn,
                format!("Trigger source is not in the active scope binding: {trigger_source}"),
            );
            return;
        }
        let settings_snapshot = self.scope_effective_settings(&binding);
        if mode == ScopeMode::CaptureArmed
            && settings_snapshot.record_points != self.wave.settings.record_points
        {
            self.log.push(
                LogLevel::Warn,
                format!(
                    "Wave record fallback: requested {} pts, using {} pts for {} channel(s)",
                    self.wave.settings.record_points,
                    settings_snapshot.record_points,
                    binding.len()
                ),
            );
        }

        self.plot_data.clear();
        self.wave.pending_binding = binding.clone();
        self.wave.settings_snapshot = settings_snapshot;
        self.wave.pane_vars_snapshot = pane_vars;

        self.send_catalog(CatalogCommand::ConfigureScope(ScopeConfig {
            mode: ScopeMode::Off,
            trigger_slot: 0,
            trigger_level: 0.0,
            trigger_hysteresis: 0.0,
            trigger_edge: TriggerEdge::Rise,
            pre_trigger_percent: 0,
            prescaler: self.wave.settings.prescaler,
            record_points: 0,
            ack_capture_id: NO_CAPTURE_ACK,
            flags: 0,
        }));
        self.send_catalog(CatalogCommand::BindChannels {
            channels: binding.iter().map(|descriptor| descriptor.var).collect(),
        });
        self.send_catalog(CatalogCommand::ConfigureScope(
            self.scope_config(mode, &binding),
        ));
        self.log.push(
            LogLevel::Info,
            format!(
                "Wave start: {}, {} channel(s)",
                mode_name(mode),
                binding.len()
            ),
        );
    }

    pub(in crate::app) fn stop_acquisition(&mut self) {
        self.send_catalog(CatalogCommand::ConfigureScope(ScopeConfig {
            mode: ScopeMode::Off,
            trigger_slot: 0,
            trigger_level: 0.0,
            trigger_hysteresis: 0.0,
            trigger_edge: TriggerEdge::Rise,
            pre_trigger_percent: 0,
            prescaler: self.wave.settings.prescaler,
            record_points: 0,
            ack_capture_id: NO_CAPTURE_ACK,
            flags: 0,
        }));
        self.wave.active = false;
        self.wave.restart_pending = None;
    }

    pub(in crate::app) fn restart_acquisition(&mut self, mode: ScopeMode) {
        if !self.project_policy().wave_start {
            self.log.push(
                LogLevel::Warn,
                "Wave restart blocked by project safety state".to_owned(),
            );
            return;
        }
        self.stop_acquisition();
        self.wave.restart_pending = Some(mode);
    }

    pub(in crate::app) fn rearm_capture(&mut self, capture_id: u16) {
        if !self.project_policy().wave_start
            || !self.hardware.connected
            || !self.wave.active
            || self.wave.restart_pending.is_some()
            || self.wave.binding.is_empty()
        {
            return;
        }
        self.wave.settings_snapshot = self.scope_effective_settings(&self.wave.binding);
        let mut config = self.scope_config(ScopeMode::CaptureArmed, &self.wave.binding);
        config.ack_capture_id = capture_id;
        self.send_catalog(CatalogCommand::ConfigureScope(config));
        self.log
            .push(LogLevel::Debug, "Capture re-armed".to_owned());
    }

    fn scope_config(&self, mode: ScopeMode, binding: &[VarDescriptor]) -> ScopeConfig {
        let settings = self.scope_effective_settings(binding);
        let trigger_slot = self
            .wave
            .settings
            .trigger_source
            .as_ref()
            .and_then(|name| {
                binding
                    .iter()
                    .position(|descriptor| &descriptor.name == name)
                    .map(|index| index as u16)
            })
            .unwrap_or(0);
        ScopeConfig {
            mode,
            trigger_slot,
            trigger_level: settings.trigger_level,
            trigger_hysteresis: settings.trigger_hysteresis,
            trigger_edge: settings.trigger_edge,
            pre_trigger_percent: settings.pre_trigger_percent,
            prescaler: settings.prescaler,
            record_points: if mode == ScopeMode::CaptureArmed {
                settings.record_points
            } else {
                0
            },
            ack_capture_id: NO_CAPTURE_ACK,
            flags: 0,
        }
    }

    fn scope_effective_settings(
        &self,
        binding: &[VarDescriptor],
    ) -> crate::wave::AcquisitionSettings {
        self.wave
            .settings
            .with_record_point_fallback(self.max_record_points_for_scope_binding(binding))
    }

    pub(in crate::app) fn current_scope_record_limit(&self) -> Option<u16> {
        let pane_vars = self.collect_time_series_vars();
        let binding = self.resolve_scope_binding(&pane_vars);
        self.max_record_points_for_scope_binding(&binding)
    }

    fn collect_time_series_vars(&self) -> Vec<String> {
        let mut names = Vec::new();
        for id in self.viewport.tree.tiles.tile_ids() {
            let Some(egui_tiles::Tile::Pane(pane)) = self.viewport.tree.tiles.get(id) else {
                continue;
            };
            if pane.kind != PaneKind::TimeSeries {
                continue;
            }
            for series in &pane.series {
                if !names.contains(&series.var_name) {
                    names.push(series.var_name.clone());
                }
            }
        }
        names
    }

    fn resolve_scope_binding(&self, names: &[String]) -> Vec<VarDescriptor> {
        names
            .iter()
            .filter_map(|name| self.inspector.descriptor_by_name(name))
            .filter(|descriptor| descriptor.is_scope())
            .take(scope_channel_limit(self.hardware.info.as_ref()))
            .cloned()
            .collect()
    }

    fn max_record_points_for_scope_binding(&self, binding: &[VarDescriptor]) -> Option<u16> {
        self.hardware
            .info
            .as_ref()
            .and_then(|info| max_record_points_for_binding(binding, info))
    }

    fn calibration_measurement_done_ok(&self) -> bool {
        let state = self
            .inspector
            .value_by_name("v2k_cal.state")
            .map(|value| value as u16);
        let result = self
            .inspector
            .value_by_name("v2k_cal.result")
            .map(|value| value as u16);
        state == Some(2) && result == Some(1)
    }

    fn calibration_value_u16(&self, name: &str) -> Option<u16> {
        self.inspector
            .value_by_name(name)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value as u16)
    }

    fn calibration_value_u32(&self, name: &str) -> Option<u32> {
        self.inspector
            .value_by_name(name)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value as u32)
    }
}

fn mode_name(mode: ScopeMode) -> &'static str {
    match mode {
        ScopeMode::Off => "off",
        ScopeMode::Stream => "stream",
        ScopeMode::CaptureArmed => "capture",
        ScopeMode::CapturePost => "capture post",
        ScopeMode::CaptureFrozen => "capture frozen",
        ScopeMode::Unknown(_) => "unknown",
    }
}
