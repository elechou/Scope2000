use std::time::Instant;

use crate::app::ScopeApp;
use crate::app::state::{
    ABZ_ZEROING_READ_NAMES, ABZ_ZEROING_READ_PERIOD, ABZ_ZEROING_STATUS_READ_NAMES,
    ABZ_ZEROING_STATUS_READ_PERIOD, AbzZeroingSnapshot, CALIBRATION_READ_NAMES,
    CALIBRATION_READ_PERIOD, CALIBRATION_STATUS_READ_NAMES, CALIBRATION_STATUS_READ_PERIOD,
    CalibrationGate, CalibrationGateInput, CalibrationSnapshot, DcVoltageSnapshot,
    calibration_gate,
};
use crate::console::LogLevel;
use crate::source::{
    CAL_READ_MAX, CAP_ABZ_ZEROING, CAP_CAL, CAP_CAPTURE_FORCE, CAP_CT_ZERO_CAL, CAP_NATIVE_BLOCK,
    CAP_PRE_TRIGGER, CAP_SCOPE_CAPTURE, CAP_SCOPE_STREAM, CAP_SYSTEM_CMD, CalibrationCommand,
    CatalogCommand, DAQ_FLAG_TRIGGER_DISABLED, NO_CAPTURE_ACK, ParamWrite, ScopeConfig, ScopeMode,
    SourceCommand, SystemCommand, TriggerEdge, ValueRead, VarDescriptor,
};
use crate::wave::{max_record_points_for_binding, pane::PaneKind, scope_channel_limit};

impl ScopeApp {
    pub(in crate::app) fn send(&self, command: SourceCommand) {
        let _ = self.source.commands.send(command);
    }

    pub(in crate::app) fn send_catalog(&self, command: CatalogCommand) {
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
        self.abz_zeroing.reset_session();
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
        if !self.watch_read_pending && !self.ui.varmap_continuous_refresh {
            return;
        }
        let now = Instant::now();
        if now >= self.next_watch_read {
            for reads in self.inspector.read_batches() {
                self.send_catalog(CatalogCommand::ReadValues(reads));
            }
            let voltage_reads = self.dc_voltage_reads();
            if !voltage_reads.is_empty() {
                self.send_catalog(CatalogCommand::ReadValues(voltage_reads));
            }
            self.watch_read_pending = false;
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

    pub(in crate::app) fn poll_abz_zeroing_reads(&mut self) {
        if !self.hardware.connected
            || !self.descriptor_catalog_ready
            || !self.has_capability(CAP_CAL)
            || !self.has_capability(CAP_ABZ_ZEROING)
        {
            return;
        }
        if !self.ui.show_abz_zeroing && self.catalog_value_u16("v2k_abz_zeroing.ready") == Some(1) {
            return;
        }

        let now = Instant::now();
        if now < self.abz_zeroing.next_read {
            return;
        }

        let names = if self.ui.show_abz_zeroing {
            ABZ_ZEROING_READ_NAMES
        } else {
            ABZ_ZEROING_STATUS_READ_NAMES
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
        let period = if self.ui.show_abz_zeroing {
            ABZ_ZEROING_READ_PERIOD
        } else {
            ABZ_ZEROING_STATUS_READ_PERIOD
        };
        self.abz_zeroing.next_read = now + period;
    }

    pub(in crate::app) fn current_sensor_calibration_snapshot(&self) -> CalibrationSnapshot {
        CalibrationSnapshot {
            state: self.catalog_value_u16("v2k_cal.state"),
            result: self.catalog_value_u16("v2k_cal.result"),
            applied_source: self.catalog_value_u16("v2k_cal.applied_src"),
            store_valid: self.catalog_value_u16("v2k_cal.store_valid"),
            store_result: self.catalog_value_u16("v2k_cal.store_result"),
            store_sequence: self.catalog_value_u32("v2k_cal.store_seq"),
        }
    }

    pub(in crate::app) fn dc_voltage_snapshot(&self) -> DcVoltageSnapshot {
        DcVoltageSnapshot {
            dc1: self.catalog_value_dc_voltage(1),
            dc2: self.catalog_value_dc_voltage(2),
        }
    }

    pub(in crate::app) fn abz_zeroing_snapshot(&self) -> Option<AbzZeroingSnapshot> {
        self.has_capability(CAP_ABZ_ZEROING)
            .then(|| AbzZeroingSnapshot {
                ready: self.catalog_value_u16("v2k_abz_zeroing.ready"),
                state: self.catalog_value_u16("v2k_abz_zeroing.state"),
                result: self.catalog_value_u16("v2k_abz_zeroing.result"),
                block_reason: self.catalog_value_u16("v2k_abz_zeroing.block_reason"),
                attempt_sequence: self.catalog_value_u32("v2k_abz_zeroing.attempt_seq"),
                eqep2_raw_count: self.catalog_value_u16("v2k_abz.eqep2.raw_count"),
                eqep2_index_count: self.catalog_value_u32("v2k_abz.eqep2.index_count"),
                eqep2_index_latch: self.catalog_value_u16("v2k_abz.eqep2.index_latch"),
                eqep2_index_event: self.catalog_value_u16("v2k_abz.eqep2.index_event"),
                eqep2_dir_change: self.catalog_value_u16("v2k_abz.eqep2.dir_change"),
                eqep2_status: self.catalog_value_u16("v2k_abz.eqep2.status"),
                eqep2_error_flags: self.catalog_value_u32("v2k_abz.eqep2.error_flags"),
                npe_z_good: self.catalog_value_u16("v2k_abz_zeroing.npe.z_good"),
                npe_z_seen: self.catalog_value_u32("v2k_abz_zeroing.npe.z_seen"),
                npe_z_rejects: self.catalog_value_u32("v2k_abz_zeroing.npe.z_rejects"),
                npe_first_latch: self.catalog_value_u16("v2k_abz_zeroing.npe.first_latch"),
                npe_last_latch: self.catalog_value_u16("v2k_abz_zeroing.npe.last_latch"),
                npe_last_reject_latch: self
                    .catalog_value_u16("v2k_abz_zeroing.npe.last_reject_latch"),
                npe_dir_changes: self.catalog_value_u32("v2k_abz_zeroing.npe.dir_changes"),
                npe_dir_resets: self.catalog_value_u32("v2k_abz_zeroing.npe.dir_resets"),
                npe_error_resets: self.catalog_value_u32("v2k_abz_zeroing.npe.error_resets"),
                npe_last_error_flags: self
                    .catalog_value_u32("v2k_abz_zeroing.npe.last_error_flags"),
            })
    }

    pub(in crate::app) fn zeroing_start_ready(&self) -> bool {
        self.current_zeroing_start_ready() && self.abz_zeroing_start_ready()
    }

    pub(in crate::app) fn zeroing_start_block_reason(&self) -> Option<&'static str> {
        let current_ready = self.current_zeroing_start_ready();
        let abz_ready = self.abz_zeroing_start_ready();
        match (current_ready, abz_ready) {
            (false, false) => Some("Start requires Current Zeroing and ABZ Zeroing ready"),
            (false, true) => Some("Start requires Current Zeroing ready"),
            (true, false) => Some("Start requires ABZ Zeroing ready"),
            (true, true) => None,
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
        if mode == ScopeMode::CaptureArmed
            && self.wave.settings.trigger_source.is_none()
            && !self.has_capability(CAP_CAPTURE_FORCE)
        {
            self.log.push(
                LogLevel::Warn,
                "CAPTURE_FORCE capability is not available".to_owned(),
            );
            return;
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

        let start_config = self.scope_config(mode, &binding);
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
        self.send_catalog(CatalogCommand::ConfigureScope(start_config));
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
        let auto_capture =
            mode == ScopeMode::CaptureArmed && self.wave.settings.trigger_source.is_none();
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
            flags: if auto_capture {
                DAQ_FLAG_TRIGGER_DISABLED
            } else {
                0
            },
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

    fn current_zeroing_start_ready(&self) -> bool {
        if !self.has_capability(CAP_CAL) || !self.has_capability(CAP_CT_ZERO_CAL) {
            return true;
        }
        self.current_sensor_calibration_snapshot().start_ready()
    }

    fn abz_zeroing_start_ready(&self) -> bool {
        if !self.has_capability(CAP_ABZ_ZEROING) {
            return true;
        }
        self.abz_zeroing_snapshot()
            .is_some_and(AbzZeroingSnapshot::start_ready)
    }

    fn catalog_value_dc_voltage(&self, channel: u8) -> Option<f64> {
        let index = self.dc_voltage_descriptor_index(channel)?;
        self.inspector
            .values
            .get(index)
            .copied()
            .flatten()
            .filter(|value| value.is_finite())
    }

    fn dc_voltage_reads(&self) -> Vec<ValueRead> {
        let mut indexes = Vec::new();
        for channel in [1, 2] {
            if let Some(index) = self.dc_voltage_descriptor_index(channel)
                && !indexes.contains(&index)
            {
                indexes.push(index);
            }
        }

        indexes
            .into_iter()
            .filter_map(|descriptor_index| {
                self.inspector
                    .descriptors
                    .get(descriptor_index)
                    .map(|descriptor| ValueRead {
                        descriptor_index,
                        var: descriptor.var,
                    })
            })
            .collect()
    }

    fn dc_voltage_descriptor_index(&self, channel: u8) -> Option<usize> {
        for name in dc_voltage_candidate_names(channel) {
            if let Some(index) = self.inspector.index_by_name(name) {
                return Some(index);
            }
        }

        None
    }

    fn catalog_value_u16(&self, name: &str) -> Option<u16> {
        self.inspector
            .value_by_name(name)
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value as u16)
    }

    fn catalog_value_u32(&self, name: &str) -> Option<u32> {
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

const DC1_VOLTAGE_NAMES: &[&str] = &["v2k_adc.voltage.vt1"];

const DC2_VOLTAGE_NAMES: &[&str] = &["v2k_adc.voltage.vt2"];

fn dc_voltage_candidate_names(channel: u8) -> &'static [&'static str] {
    match channel {
        1 => DC1_VOLTAGE_NAMES,
        2 => DC2_VOLTAGE_NAMES,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread, time::Instant};

    use eframe::egui;

    use super::*;
    use crate::app::ScopeApp;
    use crate::app::state::{
        AbzZeroingState, AppConfig, CalibrationState, PROJECT_MANAGER_SPLIT_DEFAULT,
        ProjectContext, UNTITLED_PROJECT, UiState, UnresolvedRefs, WorkspaceAutosaveState,
        WorkspaceState,
    };
    use crate::source::{
        CAP_CAL, CAP_NATIVE_BLOCK, CAP_PRE_TRIGGER, CAP_SCOPE_CAPTURE, DeviceInfo,
        MCU_MODEL_F28379D, SourceCommand, SourceEvent, SourceHandle, VarRef, VarType,
    };
    use crate::variable::InspectorState;
    use crate::wave::csv::CsvState;
    use crate::wave::data::PlotData;
    use crate::wave::pane::PaneKind;
    use crate::wave::{AcquisitionSettings, PLOT_MAX_POINTS, WaveState};

    struct TestHarness {
        app: ScopeApp,
        commands: mpsc::Receiver<SourceCommand>,
        events: mpsc::Sender<SourceEvent>,
    }

    fn descriptor(name: &str) -> VarDescriptor {
        VarDescriptor {
            name: name.to_owned(),
            var: VarRef {
                addr: 0x1000,
                ty: VarType::F32,
            },
            kind: 0x0002,
            prescaler: 1,
        }
    }

    fn device_info() -> DeviceInfo {
        DeviceInfo {
            protocol_version: 10,
            contract_version: 17,
            build_hash: 0x1234_5678,
            descriptor_count: 1,
            firmware_name: "viewer2000-test".to_owned(),
            tick_hz: 20_000,
            capabilities: CAP_CAL
                | CAP_SCOPE_CAPTURE
                | CAP_PRE_TRIGGER
                | CAP_NATIVE_BLOCK
                | CAP_CAPTURE_FORCE,
            project_name: UNTITLED_PROJECT.to_owned(),
            build_time_utc: 0,
            mcu_model: MCU_MODEL_F28379D,
            scope_max_ch: 16,
            scope_block_ticks: 10,
            scope_ring_words: 0xDFF8,
        }
    }

    fn test_harness(trigger_source: Option<&str>) -> TestHarness {
        let (command_tx, commands) = mpsc::channel();
        let (events, event_rx) = mpsc::channel();
        let source = SourceHandle::new(command_tx, event_rx, thread::spawn(|| {}));
        let mut inspector = InspectorState::default();
        inspector.set_descriptors(vec![descriptor("signal")]);

        let mut viewport = crate::app::state::ViewportState::new();
        let tile_ids: Vec<_> = viewport.tree.tiles.tile_ids().collect();
        for tile_id in tile_ids {
            let Some(egui_tiles::Tile::Pane(pane)) = viewport.tree.tiles.get_mut(tile_id) else {
                continue;
            };
            if pane.kind == PaneKind::TimeSeries {
                pane.add_series("signal".to_owned(), egui::Color32::WHITE);
                break;
            }
        }

        let settings = AcquisitionSettings {
            trigger_source: trigger_source.map(str::to_owned),
            ..AcquisitionSettings::default()
        };
        let now = Instant::now();
        let app = ScopeApp {
            hardware: crate::app::state::HardwareState {
                connected: true,
                info: Some(device_info()),
                ..crate::app::state::HardwareState::default()
            },
            abz_zeroing: AbzZeroingState::new(),
            calibration: CalibrationState::new(),
            source,
            inspector,
            viewport,
            wave: WaveState {
                settings: settings.clone(),
                settings_snapshot: settings,
                ..WaveState::default()
            },
            plot_data: PlotData::new(PLOT_MAX_POINTS),
            csv: CsvState::default(),
            log: Default::default(),
            ui: UiState::default(),
            config: AppConfig::default(),
            workspace: WorkspaceState::default(),
            project: ProjectContext {
                registry: Default::default(),
                active_name: None,
                local: None,
                unresolved: UnresolvedRefs::default(),
                show_missing: false,
                show_migration: false,
                show_project_manager: false,
                project_search: String::new(),
                project_manager_split: PROJECT_MANAGER_SPLIT_DEFAULT,
            },
            project_scan: None,
            project_metadata_scan: None,
            local_report_path: None,
            project_candidates: Vec::new(),
            project_index_target: None,
            pending_rebind: None,
            pending_delete_project: None,
            next_watch_read: now,
            watch_read_pending: false,
            next_metadata_refresh: now,
            workspace_watch_restored: false,
            descriptor_catalog_ready: true,
            workspace_autosave: WorkspaceAutosaveState::new(),
        };

        TestHarness {
            app,
            commands,
            events,
        }
    }

    fn drain_catalog_commands(commands: &mpsc::Receiver<SourceCommand>) -> Vec<CatalogCommand> {
        commands
            .try_iter()
            .filter_map(|command| match command {
                SourceCommand::Catalog { command, .. } => Some(command),
                SourceCommand::Shutdown => None,
                other => panic!("unexpected command: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn auto_capture_start_does_not_read_current_value_and_forces_each_armed_generation() {
        let mut harness = test_harness(None);
        harness.app.start_acquisition(ScopeMode::CaptureArmed);

        let commands = drain_catalog_commands(&harness.commands);
        assert_eq!(commands.len(), 3);
        assert!(
            commands
                .iter()
                .all(|command| { !matches!(command, CatalogCommand::ReadValues(_)) })
        );
        assert!(matches!(
            &commands[0],
            CatalogCommand::ConfigureScope(config)
                if config.mode == ScopeMode::Off && config.flags == 0
        ));
        assert!(
            matches!(&commands[1], CatalogCommand::BindChannels { channels } if channels.len() == 1)
        );
        assert!(matches!(
            &commands[2],
            CatalogCommand::ConfigureScope(config)
                if config.mode == ScopeMode::CaptureArmed
                    && config.flags == DAQ_FLAG_TRIGGER_DISABLED
        ));

        harness
            .events
            .send(SourceEvent::ChannelsBound { bind_sequence: 1 })
            .unwrap();
        harness
            .events
            .send(SourceEvent::ScopeConfigured {
                mode: ScopeMode::CaptureArmed,
            })
            .unwrap();
        harness.app.poll_events();
        let commands = drain_catalog_commands(&harness.commands);
        assert_eq!(commands.len(), 1);
        assert!(matches!(&commands[0], CatalogCommand::ForceCapture));

        harness.app.rearm_capture(42);
        let commands = drain_catalog_commands(&harness.commands);
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            CatalogCommand::ConfigureScope(config)
                if config.mode == ScopeMode::CaptureArmed
                    && config.ack_capture_id == 42
                    && config.flags == DAQ_FLAG_TRIGGER_DISABLED
        ));

        harness
            .events
            .send(SourceEvent::ScopeConfigured {
                mode: ScopeMode::CaptureArmed,
            })
            .unwrap();
        harness.app.poll_events();
        let commands = drain_catalog_commands(&harness.commands);
        assert_eq!(commands.len(), 1);
        assert!(matches!(&commands[0], CatalogCommand::ForceCapture));
    }

    #[test]
    fn explicit_trigger_capture_keeps_threshold_controls_and_does_not_force() {
        let mut harness = test_harness(Some("signal"));
        harness.app.start_acquisition(ScopeMode::CaptureArmed);

        let commands = drain_catalog_commands(&harness.commands);
        assert_eq!(commands.len(), 3);
        assert!(matches!(
            &commands[2],
            CatalogCommand::ConfigureScope(config)
                if config.mode == ScopeMode::CaptureArmed
                    && config.trigger_slot == 0
                    && config.flags == 0
        ));

        harness
            .events
            .send(SourceEvent::ScopeConfigured {
                mode: ScopeMode::CaptureArmed,
            })
            .unwrap();
        harness.app.poll_events();
        assert!(drain_catalog_commands(&harness.commands).is_empty());
    }
}
