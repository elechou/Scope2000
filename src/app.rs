mod actions;
pub(crate) mod state;

use std::time::{Duration, Instant};

use eframe::egui;

use crate::console::{LogBuffer, LogLevel};
use crate::source::v2k::{V2kSource, transport};
use crate::source::{
    CAP_SYSTEM_CMD, DataSource, ScopeMode, SourceCommand, SourceHandle, SystemCommand, SystemState,
};
use crate::theme;
use crate::variable::InspectorState;
use crate::wave::csv::CsvState;
use crate::wave::data::PlotData;
use crate::wave::viewer_panel::ViewportPanelState;
use crate::wave::{PLOT_MAX_POINTS, WaveState, pane::PaneKind};

use self::state::{AppConfig, HardwareState, UiState, ViewportState};

const WATCH_READ_PERIOD: Duration = Duration::from_secs(1);

pub struct ScopeApp {
    hardware: HardwareState,
    source: SourceHandle,
    inspector: InspectorState,
    viewport: ViewportState,
    wave: WaveState,
    plot_data: PlotData,
    csv: CsvState,
    log: LogBuffer,
    ui: UiState,
    config: AppConfig,
    next_watch_read: Instant,
    workspace_watch_restored: bool,
}

impl ScopeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::setup_fonts_and_icons(&cc.egui_ctx);
        theme::apply_theme(&cc.egui_ctx);
        let csv_gpu_mask_ready = crate::wave::panel::init_csv_button_renderer(cc);

        let mut config = AppConfig::load();
        config.workspace.acquisition.clamp();

        let mut hardware = HardwareState {
            port: config.port.clone(),
            baud: config.baud,
            serial_ports: transport::available_serial_ports(),
            ..HardwareState::default()
        };
        if !hardware.port.is_empty() && !hardware.serial_ports.contains(&hardware.port) {
            hardware.serial_ports.insert(0, hardware.port.clone());
        }

        let mut app = Self {
            hardware,
            source: Box::new(V2kSource).spawn(),
            inspector: InspectorState::default(),
            viewport: ViewportState::new(),
            wave: WaveState {
                settings: config.workspace.acquisition.clone(),
                settings_snapshot: config.workspace.acquisition.clone(),
                ..WaveState::default()
            },
            plot_data: PlotData::new(PLOT_MAX_POINTS),
            csv: CsvState {
                snapshot_dir: config.workspace.csv_export.snapshot_dir.clone(),
                filename_template: config.workspace.csv_export.filename_template.clone(),
                ultra_fast: config.workspace.csv_export.ultra_fast,
                gpu_mask_ready: csv_gpu_mask_ready,
                ..CsvState::default()
            },
            log: LogBuffer::default(),
            ui: UiState::default(),
            config,
            next_watch_read: Instant::now(),
            workspace_watch_restored: false,
        };
        app.restore_workspace_layout();
        app
    }

    fn has_capability(&self, capability: u32) -> bool {
        self.hardware
            .info
            .as_ref()
            .is_some_and(|info| info.has(capability))
    }

    fn device_panel(&mut self, ui: &mut egui::Ui) {
        theme::section_header(ui, "Device");
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            if theme::action_button(ui, "Refresh", theme::WIDGET_BG, true) {
                self.hardware.serial_ports = transport::available_serial_ports();
                if !self.hardware.port.is_empty()
                    && !self.hardware.serial_ports.contains(&self.hardware.port)
                {
                    self.hardware
                        .serial_ports
                        .insert(0, self.hardware.port.clone());
                }
            }
        });

        egui::ComboBox::from_id_salt("serial_port")
            .width(ui.available_width())
            .selected_text(if self.hardware.port.is_empty() {
                "Select serial port"
            } else {
                &self.hardware.port
            })
            .show_ui(ui, |ui| {
                for port in &self.hardware.serial_ports {
                    ui.selectable_value(&mut self.hardware.port, port.clone(), port);
                }
            });
        egui::ComboBox::from_id_salt("baud")
            .width(ui.available_width())
            .selected_text(self.hardware.baud.to_string())
            .show_ui(ui, |ui| {
                for baud in [115_200, 230_400, 460_800, 921_600, 1_500_000] {
                    ui.selectable_value(&mut self.hardware.baud, baud, baud.to_string());
                }
            });

        let button_gap = ui.spacing().item_spacing.x;
        let pair_button_w = ((ui.available_width() - button_gap) / 2.0).max(0.0);
        ui.horizontal(|ui| {
            let can_connect = !self.hardware.connected
                && !self.hardware.connecting
                && !self.hardware.port.is_empty();
            if theme::action_button_w(ui, "Connect", theme::GREEN, can_connect, pair_button_w) {
                self.connect();
            }
            if theme::action_button_w(
                ui,
                "Disconnect",
                theme::RED,
                self.hardware.connected,
                pair_button_w,
            ) {
                self.disconnect_or_warn();
            }
        });

        ui.separator();
        if let Some(info) = &self.hardware.info {
            ui.monospace(format!("firmware {}", info.firmware_name));
            ui.monospace(format!(
                "wire {} contract {}",
                info.protocol_version, info.contract_version
            ));
            ui.monospace(format!("build 0x{:08X}", info.build_hash));
            ui.monospace(format!("tick {} Hz", info.tick_hz));
            ui.monospace(format!(
                "descriptors {}/{}",
                self.inspector.descriptors.len(),
                info.descriptor_count
            ));
            ui.monospace(format!("capabilities 0x{:08X}", info.capabilities));
        } else {
            ui.weak("No Viewer2000 session");
        }

        if let Some(status) = &self.hardware.status {
            ui.separator();
            ui.monospace(format!(
                "state={}({}) fault={} flags=0x{:04X}",
                status.system_state,
                status.system_state.wire_value(),
                status.fault_code,
                status.status_flags
            ));
            ui.monospace(format!(
                "tick={}",
                status.tick
            ));
            ui.monospace(format!(
                "hb={}/{}",
                status.cpu1_heartbeat, status.cpu2_heartbeat
            ));
            ui.monospace(format!(
                "cal seq={} result={} fail={}",
                status.applied_seq, status.calibration_result, status.calibration_fail_index
            ));
            ui.monospace(format!(
                "scope={} flags=0x{:02X}",
                self.hardware.scope_mode_label(),
                status.scope_flags,
            ));
            ui.monospace(format!(
                "cmd ack={} result={}",
                status.command_ack_seq.unwrap_or_default(),
                status.command_result.unwrap_or_default()
            ));
        }

        theme::section_header(ui, "System");
        ui.add_space(4.0);
        let system_state = self
            .hardware
            .status
            .as_ref()
            .map(|status| status.system_state);
        let can_send_system_command =
            self.hardware.connected && self.has_capability(CAP_SYSTEM_CMD);
        let can_start = can_send_system_command && matches!(system_state, Some(SystemState::Idle));
        let can_stop =
            can_send_system_command && matches!(system_state, Some(SystemState::Running));
        let can_clear_fault =
            can_send_system_command && matches!(system_state, Some(SystemState::Fault));
        let button_gap = ui.spacing().item_spacing.x;
        let start_w = 70.0;
        let stop_w = 70.0;
        let clear_w = (ui.available_width() - start_w - stop_w - button_gap * 2.0).max(0.0);
        ui.horizontal(|ui| {
            if theme::action_button_w(ui, "Start", theme::GREEN, can_start, start_w) {
                self.send(SourceCommand::SystemCommand(SystemCommand::Start));
            }
            if theme::action_button_w(ui, "Stop", theme::RED, can_stop, stop_w) {
                self.send(SourceCommand::SystemCommand(SystemCommand::Stop));
            }
            if theme::action_button_w(ui, "Clear Fault", theme::GREEN, can_clear_fault, clear_w) {
                self.send(SourceCommand::SystemCommand(SystemCommand::ClearFault));
            }
        });

        let (state_label, state_color) =
            user_system_state_label(system_state, self.hardware.connected);
        ui.colored_label(state_color, format!("User system: {state_label}"));
    }

    fn render_viewport(&mut self, ui: &mut egui::Ui) {
        if self.inspector.descriptors.is_empty() {
            crate::ui::welcome::show(ui);
            return;
        }

        let mut vp = ViewportPanelState {
            tree: &mut self.viewport.tree,
            blueprint_order: &mut self.viewport.blueprint_order,
            selection: &mut self.viewport.selection,
            hovered_tile: &mut self.viewport.hovered_tile,
            hovered_blueprint_var: &mut self.viewport.hovered_blueprint_var,
            hovered_plot_var: &mut self.viewport.hovered_plot_var,
            drop_hover_panel: &mut self.viewport.drop_hover_panel,
        };
        if let Some(feedback) =
            crate::wave::viewer_panel::show_viewport(ui, &mut vp, &self.plot_data, &self.inspector)
        {
            self.log.push(LogLevel::Warn, feedback.message());
        }
    }

    fn handle_menu_action(&mut self, action: crate::ui::menu_bar::MenuAction) {
        match action {
            crate::ui::menu_bar::MenuAction::SaveWorkspace => self.save_workspace_with_log(),
            crate::ui::menu_bar::MenuAction::ResetLayout => {
                self.viewport.reset_layout();
                self.log.push(LogLevel::Info, "Layout reset".to_owned());
            }
        }
    }
}

fn user_system_state_label(state: Option<SystemState>, connected: bool) -> (String, egui::Color32) {
    match state {
        Some(SystemState::Idle) => ("IDLE".to_owned(), theme::TEXT_SUBDUED),
        Some(SystemState::Running) => ("RUNNING".to_owned(), theme::GREEN),
        Some(SystemState::Fault) => ("FAULT".to_owned(), theme::RED),
        Some(SystemState::Init) => ("INIT".to_owned(), theme::YELLOW),
        Some(SystemState::Unknown(value)) => (format!("STATE {value}"), theme::TEXT_SUBDUED),
        None if connected => ("UNKNOWN".to_owned(), theme::TEXT_SUBDUED),
        None => ("DISCONNECTED".to_owned(), theme::TEXT_SUBDUED),
    }
}

impl eframe::App for ScopeApp {
    fn on_exit(&mut self) {
        self.save_workspace();
        if self.hardware.connected {
            self.send(SourceCommand::Disconnect);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_events();
        self.poll_watch_reads();
        if self.hardware.connected {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }

        if std::mem::take(&mut self.ui.apply_panel_sizes) {
            self.apply_saved_panel_sizes(ui.ctx());
        }

        crate::ui::modals::handle_close_guard(ui, &self.hardware, &mut self.ui);
        crate::ui::modals::show_stop_warning(ui, &mut self.ui);
        crate::ui::modals::show_about_window(ui, &mut self.ui);

        if let Some(action) = crate::ui::menu_bar::show(ui, &mut self.ui) {
            self.handle_menu_action(action);
        }
        if let Some(action) =
            crate::ui::status_bar::show(ui, &mut self.hardware, &mut self.ui, &mut self.log)
        {
            use crate::ui::status_bar::StatusBarAction;
            match action {
                StatusBarAction::Connect => self.connect(),
                StatusBarAction::CancelConnect => self.hardware.connecting = false,
                StatusBarAction::Disconnect => self.disconnect_or_warn(),
            }
        }

        theme::pretick_panel_animation(ui.ctx(), "console_panel", self.ui.show_console_panel);
        egui::Panel::bottom("console_panel")
            .resizable(true)
            .default_size(250.0)
            .min_size(80.0)
            .frame(theme::side_panel_frame())
            .show_animated_inside(ui, self.ui.show_console_panel, |ui| {
                crate::console::panel::show(ui, &mut self.log);
            });

        self.viewport.drop_hover_panel = false;
        theme::pretick_panel_animation(ui.ctx(), "device_panel", self.ui.show_device_panel);
        egui::Panel::left("device_panel")
            .resizable(false)
            .exact_size(250.0)
            .show_separator_line(true)
            .frame(theme::side_panel_frame())
            .show_animated_inside(ui, self.ui.show_device_panel, |ui| {
                self.device_panel(ui);
                ui.separator();
                let pinned_changed = crate::variable::panel::show_variable_map(
                    ui,
                    &mut self.inspector,
                    &mut self.ui.source_filter,
                    &mut self.ui.varmap_split,
                );
                if pinned_changed {
                    self.next_watch_read = Instant::now();
                }
            });

        egui::Panel::left("data_panel")
            .resizable(true)
            .default_size(250.0)
            .min_size(250.0)
            .show_separator_line(false)
            .frame(theme::side_panel_frame())
            .show_inside(ui, |ui| {
                let record_limit = self.current_scope_record_limit();
                if let Some(action) = crate::wave::panel::show_wave_section(
                    ui,
                    &mut self.wave,
                    self.hardware.connected,
                    self.hardware.info.as_ref().map(|info| info.tick_hz),
                    &self.viewport.tree.tiles,
                    record_limit,
                ) {
                    use crate::wave::panel::WaveAction;
                    match action {
                        WaveAction::StartStream => self.start_acquisition(ScopeMode::Stream),
                        WaveAction::ArmCapture => {
                            self.start_acquisition(ScopeMode::CaptureArmed);
                        }
                        WaveAction::Stop => self.stop_acquisition(),
                        WaveAction::Restart(mode) => self.restart_acquisition(mode),
                    }
                }

                let has_csv_data = !self.plot_data.series.is_empty();
                let csv_preview = if self.csv.ultra_fast {
                    let lookup = |name: &str| self.inspector.value_by_name(name);
                    Some(
                        crate::app::actions::csv_export::evaluate_template(
                            &self.csv.filename_template,
                            &lookup,
                        )
                        .unwrap_or_else(|error| format!("<{error}>")),
                    )
                } else {
                    None
                };
                self.poll_csv_save();
                if let Some(action) = crate::wave::panel::show_csv_export(
                    ui,
                    &mut self.csv,
                    has_csv_data,
                    csv_preview.as_deref(),
                ) {
                    use crate::wave::panel::CsvAction;
                    match action {
                        CsvAction::QuickSnapshot => self.quick_snapshot(),
                        CsvAction::SaveWithDialog => self.save_csv_with_dialog(),
                    }
                }
                self.show_overwrite_modal(ui);

                let var_out = crate::variable::panel::show_variables(
                    ui,
                    &mut self.inspector,
                    &mut self.viewport.drop_hover_panel,
                );
                if !var_out.to_write.is_empty() {
                    self.write_variables(var_out.to_write);
                }
                if var_out.watch_changed {
                    self.next_watch_read = Instant::now();
                }

                let (add_pane, feedback) = {
                    let mut vp = ViewportPanelState {
                        tree: &mut self.viewport.tree,
                        blueprint_order: &mut self.viewport.blueprint_order,
                        selection: &mut self.viewport.selection,
                        hovered_tile: &mut self.viewport.hovered_tile,
                        hovered_blueprint_var: &mut self.viewport.hovered_blueprint_var,
                        hovered_plot_var: &mut self.viewport.hovered_plot_var,
                        drop_hover_panel: &mut self.viewport.drop_hover_panel,
                    };
                    crate::wave::viewer_panel::show_blueprint(ui, &mut vp)
                };
                if let Some(feedback) = feedback {
                    self.log.push(LogLevel::Warn, feedback.message());
                }
                if let Some(kind) = add_pane {
                    self.add_pane(kind);
                }
            });

        theme::pretick_panel_animation(ui.ctx(), "selection_panel", self.ui.show_selection_panel);
        egui::Panel::right("selection_panel")
            .resizable(true)
            .default_size(250.0)
            .min_size(250.0)
            .show_separator_line(false)
            .frame(theme::side_panel_frame())
            .show_animated_inside(ui, self.ui.show_selection_panel, |ui| {
                let mut vp = ViewportPanelState {
                    tree: &mut self.viewport.tree,
                    blueprint_order: &mut self.viewport.blueprint_order,
                    selection: &mut self.viewport.selection,
                    hovered_tile: &mut self.viewport.hovered_tile,
                    hovered_blueprint_var: &mut self.viewport.hovered_blueprint_var,
                    hovered_plot_var: &mut self.viewport.hovered_plot_var,
                    drop_hover_panel: &mut self.viewport.drop_hover_panel,
                };
                crate::wave::viewer_panel::show_selection_panel(ui, &mut vp);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::BG_BODY))
            .show_inside(ui, |ui| self.render_viewport(ui));

        self.record_panel_sizes(ui.ctx());
    }
}

impl ScopeApp {
    pub(in crate::app) fn next_pane_number(
        tiles: &egui_tiles::Tiles<crate::wave::pane::ViewPane>,
        kind: PaneKind,
    ) -> u32 {
        let prefix = kind.label();
        let mut used = Vec::new();
        for id in tiles.tile_ids() {
            if let Some(egui_tiles::Tile::Pane(pane)) = tiles.get(id)
                && pane.kind == kind
                && let Some(suffix) = pane.name.strip_prefix(prefix)
                && let Ok(number) = suffix.trim().parse::<u32>()
            {
                used.push(number);
            }
        }
        let mut number = 1;
        while used.contains(&number) {
            number += 1;
        }
        number
    }
}
