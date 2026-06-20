use std::time::Instant;

use crate::app::ScopeApp;
use crate::console::LogLevel;
use crate::source::{
    CAP_CAL, CAP_NATIVE_BLOCK, CAP_PRE_TRIGGER, CAP_SCOPE_CAPTURE, CAP_SCOPE_STREAM,
    CatalogCommand, ParamWrite, ScopeConfig, ScopeMode, SourceCommand, TriggerEdge, VarDescriptor,
};
use crate::wave::{max_record_points_for_binding, pane::PaneKind};

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
        let param_writes: Vec<ParamWrite> = writes
            .into_iter()
            .filter_map(|(index, value)| self.inspector.param_write_for(index, value))
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
        self.wave
            .settings
            .clamp_record_points(max_record_points_for_binding(&binding));

        self.plot_data.clear();
        self.wave.capture_frame_blocks.clear();
        self.wave.pending_binding = binding.clone();
        self.wave.settings_snapshot = self.wave.settings.clone();
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

    pub(in crate::app) fn rearm_capture(&mut self) {
        if !self.project_policy().wave_start
            || !self.hardware.connected
            || !self.wave.active
            || self.wave.restart_pending.is_some()
            || self.wave.binding.is_empty()
        {
            return;
        }
        self.wave.capture_frame_blocks.clear();
        let max_record_points = max_record_points_for_binding(&self.wave.binding);
        self.wave.settings.clamp_record_points(max_record_points);
        self.wave.settings_snapshot = self.wave.settings.clone();
        self.send_catalog(CatalogCommand::ConfigureScope(
            self.scope_config(ScopeMode::CaptureArmed, &self.wave.binding),
        ));
        self.log
            .push(LogLevel::Debug, "Capture re-armed".to_owned());
    }

    fn scope_config(&self, mode: ScopeMode, binding: &[VarDescriptor]) -> ScopeConfig {
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
            trigger_level: self.wave.settings.trigger_level,
            trigger_hysteresis: self.wave.settings.trigger_hysteresis,
            trigger_edge: self.wave.settings.trigger_edge,
            pre_trigger_percent: self.wave.settings.pre_trigger_percent,
            prescaler: self.wave.settings.prescaler,
            record_points: if mode == ScopeMode::CaptureArmed {
                self.wave.settings.record_points
            } else {
                0
            },
        }
    }

    pub(in crate::app) fn current_scope_record_limit(&self) -> Option<u16> {
        let pane_vars = self.collect_time_series_vars();
        let binding = self.resolve_scope_binding(&pane_vars);
        max_record_points_for_binding(&binding)
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
            .take(8)
            .cloned()
            .collect()
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
