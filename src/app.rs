mod actions;
pub(crate) mod state;

use std::sync::mpsc;
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

use self::state::{
    AppConfig, HardwareState, ProjectCandidate, ProjectContext, UiState, ViewportState,
    WorkspaceAutosaveState, WorkspaceState, WorkspaceStore,
};

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
    workspace: WorkspaceState,
    project: ProjectContext,
    project_scan: Option<mpsc::Receiver<Vec<ProjectCandidate>>>,
    project_metadata_scan: Option<mpsc::Receiver<Result<state::LocalProject, String>>>,
    project_candidates: Vec<ProjectCandidate>,
    project_index_target: Option<String>,
    pending_rebind: Option<state::LocalProject>,
    pending_delete_project: Option<String>,
    next_watch_read: Instant,
    workspace_watch_restored: bool,
    descriptor_catalog_ready: bool,
    workspace_autosave: WorkspaceAutosaveState,
}

impl ScopeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::setup_fonts_and_icons(&cc.egui_ctx);
        theme::apply_theme(&cc.egui_ctx);
        let csv_gpu_mask_ready = crate::wave::panel::init_csv_button_renderer(cc);

        let config = AppConfig::load();
        let legacy_backup_error = config
            .legacy_workspace
            .as_ref()
            .and_then(|legacy| WorkspaceStore::save_legacy(legacy).err());
        let project = ProjectContext::load(&config);
        let mut workspace = project
            .active_name
            .as_deref()
            .map(WorkspaceStore::load)
            .unwrap_or_default();
        workspace.acquisition.clamp();

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
                settings: workspace.acquisition.clone(),
                settings_snapshot: workspace.acquisition.clone(),
                ..WaveState::default()
            },
            plot_data: PlotData::new(PLOT_MAX_POINTS),
            csv: CsvState {
                snapshot_dir: workspace.csv_export.snapshot_dir.clone(),
                filename_template: workspace.csv_export.filename_template.clone(),
                ultra_fast: workspace.csv_export.ultra_fast,
                gpu_mask_ready: csv_gpu_mask_ready,
                ..CsvState::default()
            },
            log: LogBuffer::default(),
            ui: UiState::default(),
            config,
            workspace,
            project,
            project_scan: None,
            project_metadata_scan: None,
            project_candidates: Vec::new(),
            project_index_target: None,
            pending_rebind: None,
            pending_delete_project: None,
            next_watch_read: Instant::now(),
            workspace_watch_restored: false,
            descriptor_catalog_ready: false,
            workspace_autosave: WorkspaceAutosaveState::new(),
        };
        app.restore_workspace_layout();
        app.reset_workspace_autosave_baseline();
        app.begin_local_project_metadata_refresh();
        if let Some(error) = legacy_backup_error {
            app.log.push(
                LogLevel::Warn,
                format!("Failed to back up the legacy workspace: {error}"),
            );
        }
        app
    }

    fn has_capability(&self, capability: u32) -> bool {
        self.hardware
            .info
            .as_ref()
            .is_some_and(|info| info.has(capability))
    }

    fn system_panel(&mut self, ui: &mut egui::Ui) {
        theme::section_header(ui, "System");
        ui.add_space(4.0);
        let system_state = self
            .hardware
            .status
            .as_ref()
            .map(|status| status.system_state);
        let can_send_system_command =
            self.hardware.connected && self.has_capability(CAP_SYSTEM_CMD);
        let can_start = can_send_system_command
            && self.project_policy().system_start
            && matches!(system_state, Some(SystemState::Idle));
        let can_stop =
            can_send_system_command && matches!(system_state, Some(SystemState::Running));

        if matches!(system_state, Some(SystemState::Fault)) {
            let clear_w = ui.available_width();
            if theme::action_button_w(
                ui,
                "Clear Fault",
                theme::GREEN,
                can_send_system_command,
                clear_w,
            ) {
                self.send(SourceCommand::SystemCommand(SystemCommand::ClearFault));
            }
        } else {
            let button_gap = ui.spacing().item_spacing.x;
            let button_w = ((ui.available_width() - button_gap) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                if theme::action_button_w(ui, "Start", theme::GREEN, can_start, button_w) {
                    self.start_system();
                }
                if theme::action_button_w(ui, "Stop", theme::RED, can_stop, button_w) {
                    self.send(SourceCommand::SystemCommand(SystemCommand::Stop));
                }
            });
        }

        let (state_label, state_color) =
            user_system_state_label(system_state, self.hardware.connected);
        ui.colored_label(state_color, format!("User system: {state_label}"));
        self.performance_panel(ui);
    }

    fn performance_panel(&self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Control Cycle Budget").strong());
        ui.add_space(2.0);

        let sample = self.hardware.performance.sample();
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, 16.0), egui::Sense::hover());
        let painter = ui.painter();

        if let Some(sample) = sample {
            paint_load_track(painter, rect, sample.has_violation());
            let fill_rect = load_fill_rect(rect);
            let budget = sample.cycle_budget as f32;
            let mut used = 0.0_f32;
            paint_load_segment(
                painter,
                fill_rect,
                &mut used,
                sample.adc_at_peak() as f32,
                budget,
                theme::RED,
            );
            paint_load_segment(
                painter,
                fill_rect,
                &mut used,
                sample.control_at_peak as f32,
                budget,
                theme::SELECT_BG,
            );
            paint_load_segment(
                painter,
                fill_rect,
                &mut used,
                sample.scope_at_peak as f32,
                budget,
                theme::YELLOW,
            );
            paint_load_segment(
                painter,
                fill_rect,
                &mut used,
                sample.runtime_at_peak() as f32,
                budget,
                theme::GREEN,
            );
            paint_load_track_stroke(painter, rect, sample.has_violation());

            response.on_hover_ui(|ui| {
                ui.label(egui::RichText::new("Last completed 1 s ISR window").small());
                ui.colored_label(
                    theme::RED,
                    format!("ADC/EOC: {} cycles", sample.adc_at_peak()),
                );
                ui.colored_label(
                    theme::SELECT_BG,
                    format!("Control: {} cycles", sample.control_at_peak),
                );
                ui.colored_label(
                    theme::YELLOW,
                    format!("Scope: {} cycles", sample.scope_at_peak),
                );
                ui.colored_label(
                    theme::GREEN,
                    format!("Runtime: {} cycles", sample.runtime_at_peak()),
                );
                ui.colored_label(
                    theme::TEXT_SUBDUED,
                    format!("Headroom: {} cycles", sample.headroom_at_peak()),
                );
                ui.label(format!("Budget: {} cycles", sample.cycle_budget));
                ui.label(format!("ISR overflows: {}", sample.overflows));
            });

            let text = format!(
                "Peak {:.0}% · Avg {:.0}% · Violations {}",
                sample.peak_percent(),
                sample.average_percent(),
                sample.violations
            );
            ui.label(
                egui::RichText::new(text)
                    .small()
                    .color(if sample.has_violation() {
                        theme::RED
                    } else {
                        theme::TEXT_SUBDUED
                    }),
            );
        } else {
            paint_load_track(painter, rect, false);
            paint_load_track_stroke(painter, rect, false);
            let text = if self.hardware.connected && self.hardware.performance.is_available() {
                "Collecting performance data…"
            } else {
                "Performance data unavailable"
            };
            ui.label(egui::RichText::new(text).small().color(theme::TEXT_SUBDUED));
        }
    }

    fn render_viewport(&mut self, ui: &mut egui::Ui) {
        if self.inspector.descriptors.is_empty() {
            crate::ui::welcome::show(ui);
            return;
        }

        let can_edit_variable_refs = self.project_policy().edit_variable_refs;
        let mut vp = ViewportPanelState {
            tree: &mut self.viewport.tree,
            blueprint_order: &mut self.viewport.blueprint_order,
            selection: &mut self.viewport.selection,
            hovered_tile: &mut self.viewport.hovered_tile,
            hovered_blueprint_var: &mut self.viewport.hovered_blueprint_var,
            hovered_plot_var: &mut self.viewport.hovered_plot_var,
            drop_hover_panel: &mut self.viewport.drop_hover_panel,
        };
        if let Some(feedback) = crate::wave::viewer_panel::show_viewport(
            ui,
            &mut vp,
            &self.plot_data,
            &self.inspector,
            can_edit_variable_refs,
        ) {
            self.log.push(LogLevel::Warn, feedback.message());
        }
    }

    fn handle_menu_action(&mut self, action: crate::ui::menu_bar::MenuAction) {
        match action {
            crate::ui::menu_bar::MenuAction::OpenProject => self.begin_project_index(),
            crate::ui::menu_bar::MenuAction::OpenRecentProject(name) => {
                self.open_recent_project(&name)
            }
            crate::ui::menu_bar::MenuAction::ManageProjects => {
                self.project.show_project_manager = true;
            }
            crate::ui::menu_bar::MenuAction::SaveWorkspace => self.save_workspace_with_log(),
            crate::ui::menu_bar::MenuAction::ResetLayout => {
                self.viewport.reset_layout();
                self.log.push(LogLevel::Info, "Layout reset".to_owned());
            }
        }
    }
}

fn paint_load_track(painter: &egui::Painter, rect: egui::Rect, warning: bool) {
    let radius = egui::CornerRadius::same(8);
    painter.rect_filled(rect, radius, theme::WIDGET_BG);
    let fill_rect = load_fill_rect(rect);
    painter.rect_filled(fill_rect, egui::CornerRadius::same(6), theme::WIDGET_ACTIVE);
    let sheen = egui::Rect::from_min_max(
        fill_rect.min,
        egui::pos2(
            fill_rect.right(),
            fill_rect.top() + fill_rect.height() * 0.42,
        ),
    );
    painter.rect_filled(
        sheen,
        egui::CornerRadius {
            nw: 6,
            ne: 6,
            sw: 0,
            se: 0,
        },
        egui::Color32::from_white_alpha(if warning { 18 } else { 10 }),
    );
}

fn paint_load_track_stroke(painter: &egui::Painter, rect: egui::Rect, warning: bool) {
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        egui::Stroke::new(
            1.0,
            if warning {
                theme::RED
            } else {
                theme::SEPARATOR
            },
        ),
        egui::StrokeKind::Inside,
    );
}

fn load_fill_rect(rect: egui::Rect) -> egui::Rect {
    rect.shrink2(egui::vec2(2.0, 2.0))
}

fn paint_load_segment(
    painter: &egui::Painter,
    rect: egui::Rect,
    used: &mut f32,
    cycles: f32,
    budget: f32,
    color: egui::Color32,
) {
    let start = (*used / budget).clamp(0.0, 1.0);
    *used += cycles;
    let end = (*used / budget).clamp(0.0, 1.0);
    if end <= start {
        return;
    }
    let segment = egui::Rect::from_min_max(
        egui::pos2(rect.left() + rect.width() * start, rect.top()),
        egui::pos2(rect.left() + rect.width() * end, rect.bottom()),
    );
    painter.rect_filled(segment, segment_rounding(start, end, rect.height()), color);
}

fn segment_rounding(start: f32, end: f32, height: f32) -> egui::CornerRadius {
    let radius = ((height * 0.5).round() as u8).max(1);
    egui::CornerRadius {
        nw: if start <= 0.0 { radius } else { 0 },
        sw: if start <= 0.0 { radius } else { 0 },
        ne: if end >= 1.0 { radius } else { 0 },
        se: if end >= 1.0 { radius } else { 0 },
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
        if crate::ui::modals::show_connection_settings(ui, &mut self.hardware, &mut self.ui) {
            self.connect();
        }
        crate::ui::modals::show_about_window(ui, &mut self.ui);
        self.poll_project_scan();
        self.poll_local_project_metadata();
        if self.project_metadata_scan.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        self.show_project_modals(ui);

        let recent_projects: Vec<String> = self
            .project
            .registry
            .recent_names()
            .map(str::to_owned)
            .collect();
        if let Some(action) = crate::ui::menu_bar::show(
            ui,
            &mut self.ui,
            self.hardware.can_configure_connection(),
            &recent_projects,
        ) {
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
        crate::ui::modals::show_device_info_window(
            ui,
            &self.hardware,
            self.inspector.descriptors.len(),
            &mut self.ui,
        );

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
        theme::pretick_panel_animation(ui.ctx(), "system_panel", self.ui.show_system_panel);
        egui::Panel::left("system_panel")
            .resizable(false)
            .exact_size(250.0)
            .show_separator_line(true)
            .frame(theme::side_panel_frame())
            .show_animated_inside(ui, self.ui.show_system_panel, |ui| {
                self.project_panel(ui);
                ui.separator();
                self.system_panel(ui);
                ui.separator();
                let can_edit_refs = self.project_policy().edit_variable_refs;
                let pinned_changed = ui
                    .add_enabled_ui(can_edit_refs, |ui| {
                        crate::variable::panel::show_variable_map(
                            ui,
                            &mut self.inspector,
                            &mut self.ui.source_filter,
                            &mut self.ui.varmap_split,
                        )
                    })
                    .inner;
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
                let project_policy = self.project_policy();
                let record_limit = self.current_scope_record_limit();
                if let Some(action) = crate::wave::panel::show_wave_section(
                    ui,
                    &mut self.wave,
                    self.hardware.connected,
                    self.hardware.info.as_ref().map(|info| info.tick_hz),
                    &self.viewport.tree.tiles,
                    record_limit,
                    crate::wave::panel::WavePermissions {
                        can_start: project_policy.wave_start,
                        can_edit_variable_refs: project_policy.edit_variable_refs,
                    },
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
                    project_policy.calibration_write,
                    project_policy.edit_variable_refs,
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
                    crate::wave::viewer_panel::show_blueprint(
                        ui,
                        &mut vp,
                        project_policy.edit_variable_refs,
                    )
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
                let can_edit_variable_refs = self.project_policy().edit_variable_refs;
                let mut vp = ViewportPanelState {
                    tree: &mut self.viewport.tree,
                    blueprint_order: &mut self.viewport.blueprint_order,
                    selection: &mut self.viewport.selection,
                    hovered_tile: &mut self.viewport.hovered_tile,
                    hovered_blueprint_var: &mut self.viewport.hovered_blueprint_var,
                    hovered_plot_var: &mut self.viewport.hovered_plot_var,
                    drop_hover_panel: &mut self.viewport.drop_hover_panel,
                };
                crate::wave::viewer_panel::show_selection_panel(
                    ui,
                    &mut vp,
                    can_edit_variable_refs,
                );
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(theme::BG_BODY))
            .show_inside(ui, |ui| self.render_viewport(ui));

        self.record_panel_sizes(ui.ctx());
        self.poll_workspace_autosave(ui.ctx());
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
