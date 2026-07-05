mod actions;
pub(crate) mod state;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::console::{LogBuffer, LogLevel};
use crate::source::v2k::{V2kSource, transport};
use crate::source::{
    CAP_SYSTEM_CMD, DataSource, ScopeMode, SourceHandle, SystemCommand, SystemState,
    fault_code_text,
};
use crate::theme;
use crate::variable::InspectorState;
use crate::wave::csv::CsvState;
use crate::wave::data::PlotData;
use crate::wave::viewer_panel::ViewportPanelState;
use crate::wave::{PLOT_MAX_POINTS, WaveState, pane::PaneKind};

use self::state::{
    AbzZeroingState, AppConfig, CalibrationState, HardwareState, ProjectCandidate, ProjectContext,
    UiState, ViewportState, WorkspaceAutosaveState, WorkspaceState, WorkspaceStore,
};

const WATCH_READ_PERIOD: Duration = Duration::from_secs(1);
/// How often the bound CCS project is re-scanned for a fresh build so a
/// recompile is noticed live, without blocking the UI on a directory walk.
pub(in crate::app) const LOCAL_METADATA_REFRESH_PERIOD: Duration = Duration::from_secs(2);

pub struct ScopeApp {
    hardware: HardwareState,
    abz_zeroing: AbzZeroingState,
    calibration: CalibrationState,
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
    project_metadata_scan: Option<mpsc::Receiver<Result<state::LocalBuildScan, String>>>,
    local_report_path: Option<std::path::PathBuf>,
    project_candidates: Vec<ProjectCandidate>,
    project_index_target: Option<String>,
    pending_rebind: Option<state::LocalProject>,
    pending_delete_project: Option<String>,
    next_watch_read: Instant,
    next_metadata_refresh: Instant,
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
            abz_zeroing: AbzZeroingState::new(),
            calibration: CalibrationState::new(),
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
            local_report_path: None,
            project_candidates: Vec::new(),
            project_index_target: None,
            pending_rebind: None,
            pending_delete_project: None,
            next_watch_read: Instant::now(),
            next_metadata_refresh: Instant::now(),
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
        let system_command_pending = self.hardware.pending_system_command.is_some();
        let can_send_system_command = self.hardware.connected
            && self.has_capability(CAP_SYSTEM_CMD)
            && !system_command_pending;
        let can_start = can_send_system_command
            && self.project_policy().system_start
            && matches!(system_state, Some(SystemState::Idle))
            && self.zeroing_start_ready();
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
                self.send_system_command(SystemCommand::ClearFault);
            }
        } else {
            let button_gap = ui.spacing().item_spacing.x;
            let button_w = ((ui.available_width() - button_gap) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                if theme::action_button_w(ui, "Start", theme::GREEN, can_start, button_w) {
                    self.start_system();
                }
                if theme::action_button_w(ui, "Stop", theme::RED, can_stop, button_w) {
                    self.send_system_command(SystemCommand::Stop);
                }
            });
        }

        let (state_label, state_color) =
            user_system_state_label(system_state, self.hardware.connected);
        ui.colored_label(state_color, format!("User system: {state_label}"));
        if let Some(text) = self.hardware.pending_system_command_text() {
            ui.colored_label(theme::YELLOW, text);
        }
        if let Some(text) = self.hardware.last_system_command_text() {
            ui.colored_label(theme::TEXT_SUBDUED, text);
        }
        if let Some(reason) = self.zeroing_start_block_reason()
            && can_send_system_command
            && matches!(system_state, Some(SystemState::Idle))
        {
            ui.colored_label(theme::YELLOW, reason);
        }
        if let Some(status) = &self.hardware.status
            && status.fault_code != 0
        {
            ui.colored_label(
                theme::RED,
                format!(
                    "Fault: {} ({})",
                    fault_code_text(status.fault_code),
                    status.fault_code
                ),
            );
        }
        self.performance_panel(ui);
    }

    fn performance_panel(&self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Control Cycle Budget").strong());
        ui.add_space(2.0);

        let sample = self.hardware.performance.sample();
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, LOAD_BAR_HEIGHT), egui::Sense::hover());
        let hover = response.hover_pos();
        let painter = ui.painter();

        if let Some(sample) = sample {
            let segments = [
                (theme::LOAD_ADC, sample.adc_at_peak() as f32),
                (theme::LOAD_CONTROL, sample.control_at_peak as f32),
                (theme::LOAD_SCOPE, sample.scope_at_peak as f32),
                (theme::LOAD_RUNTIME, sample.runtime_at_peak() as f32),
                (theme::LOAD_HEADROOM, sample.headroom_at_peak() as f32),
            ];
            paint_load_bar(painter, rect, &segments, sample.cycle_budget as f32, hover);

            response.on_hover_ui(|ui| {
                ui.set_min_width(LOAD_TOOLTIP_WIDTH);
                ui.colored_label(
                    theme::LOAD_ADC,
                    format!("ADC/EOC: {} cycles", sample.adc_at_peak()),
                );
                ui.colored_label(
                    theme::LOAD_CONTROL,
                    format!("Control: {} cycles", sample.control_at_peak),
                );
                ui.colored_label(
                    theme::LOAD_SCOPE,
                    format!("Scope: {} cycles", sample.scope_at_peak),
                );
                ui.colored_label(
                    theme::LOAD_RUNTIME,
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
            paint_load_bar(painter, rect, &[(theme::LOAD_HEADROOM, 1.0)], 1.0, None);
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

const LOAD_BAR_HEIGHT: f32 = 8.0;
const LOAD_SEG_GAP: f32 = 2.0;
const LOAD_SEG_ROUND: f32 = 2.0;
/// Floor width for any non-empty block, so sub-1% slivers stay visible and read
/// at a uniform height instead of collapsing into sub-pixel fuzz. Set to 0.0
/// for strictly proportional widths.
const LOAD_SEG_MIN_W: f32 = 2.0;
const LOAD_TOOLTIP_WIDTH: f32 = 220.0;

/// Paint the cycle-budget bar as discrete blocks separated by a uniform gap.
/// Each `(color, cycles)` block is sized proportionally to `total`. Gaps are
/// reserved up front (not carved out of the blocks) so they stay constant, and
/// the corner radius is clamped to each block's own width so thin slivers keep
/// full height instead of rounding down to a dot. The block under the pointer
/// (if any) is brightened.
fn paint_load_bar(
    painter: &egui::Painter,
    rect: egui::Rect,
    segments: &[(egui::Color32, f32)],
    total: f32,
    hover: Option<egui::Pos2>,
) {
    if total <= 0.0 {
        return;
    }
    let visible = segments.iter().filter(|(_, cycles)| *cycles > 0.0).count();
    if visible == 0 {
        return;
    }

    // Reserve the gaps before distributing pixels to fills, so every gap is
    // exactly LOAD_SEG_GAP regardless of how thin its neighbours are.
    let fill_budget = (rect.width() - LOAD_SEG_GAP * (visible - 1) as f32).max(0.0);
    let half_height = rect.height() * 0.5;
    let right = rect.right();

    let mut x = rect.left();
    for (color, cycles) in segments {
        if *cycles <= 0.0 || x >= right {
            continue;
        }
        let w = (fill_budget * cycles / total).max(LOAD_SEG_MIN_W);
        let seg_right = (x + w).min(right);
        if seg_right > x {
            // Radius never exceeds half the block's own width (or height), so a
            // 2px sliver stays a full-height bar rather than a rounded pill.
            let radius = LOAD_SEG_ROUND
                .min((seg_right - x) * 0.5)
                .min(half_height)
                .round() as u8;
            let seg = egui::Rect::from_min_max(
                egui::pos2(x, rect.top()),
                egui::pos2(seg_right, rect.bottom()),
            );
            let hot = hover.is_some_and(|p| seg.contains(p));
            let fill = if hot {
                color.gamma_multiply(1.15)
            } else {
                *color
            };
            painter.rect_filled(seg, egui::CornerRadius::same(radius), fill);
        }
        x = seg_right + LOAD_SEG_GAP;
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
        self.source.shutdown();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_events();
        self.poll_watch_reads();
        self.poll_current_sensor_calibration_reads();
        self.poll_abz_zeroing_reads();
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
        self.maybe_refresh_local_project_metadata();
        if self.project_metadata_scan.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        } else if self.project.local.is_some() {
            // Keep the timer ticking while idle so a CCS recompile is detected
            // even when no firmware is driving 16 ms repaints.
            ui.ctx()
                .request_repaint_after(LOCAL_METADATA_REFRESH_PERIOD);
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
        let calibration_snapshot = self.current_sensor_calibration_snapshot();
        let dc_voltage_snapshot = self.dc_voltage_snapshot();
        let abz_zeroing_snapshot = self.abz_zeroing_snapshot();
        if let Some(action) = crate::ui::status_bar::show(
            ui,
            &mut self.hardware,
            &mut self.ui,
            &mut self.log,
            dc_voltage_snapshot,
            calibration_snapshot,
            abz_zeroing_snapshot,
        ) {
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
            calibration_snapshot,
            abz_zeroing_snapshot,
            &mut self.ui,
        );
        crate::ui::abz_zeroing::show(ui, &mut self.ui, abz_zeroing_snapshot);
        let calibration_gate = self.current_sensor_calibration_gate();
        if let Some(action) = crate::ui::current_sensor_calibration::show(
            ui,
            &mut self.ui,
            &mut self.calibration,
            calibration_gate,
            calibration_snapshot,
            &self.inspector,
        ) {
            use crate::ui::current_sensor_calibration::CurrentSensorCalibrationAction;
            match action {
                CurrentSensorCalibrationAction::MeasureZero => {
                    self.send_calibration_command(crate::source::CalibrationCommand::MeasureZero);
                }
                CurrentSensorCalibrationAction::CommitToFlash => {
                    self.send_calibration_command(crate::source::CalibrationCommand::CommitToFlash);
                }
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
                let system_var_names = self.inspector.system_var_names();
                if let Some(action) = crate::wave::panel::show_wave_section(
                    ui,
                    &mut self.wave,
                    self.hardware.connected,
                    self.hardware.info.as_ref().map(|info| info.tick_hz),
                    &self.viewport.tree.tiles,
                    &system_var_names,
                    record_limit,
                    crate::wave::panel::WavePermissions {
                        can_start: project_policy.wave_start,
                        can_edit_variable_refs: project_policy.edit_variable_refs,
                    },
                ) {
                    use crate::wave::panel::WaveAction;
                    match action {
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
                        &self.inspector,
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
                    &self.inspector,
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
