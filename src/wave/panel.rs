use eframe::egui;

use crate::source::{ScopeMode, TriggerEdge};
use crate::theme;
use crate::variable::InspectorState;

use super::csv::CsvState;
use super::{
    DEFAULT_TICK_HZ, MAX_RECORD_POINTS_ABSOLUTE, MIN_PRESCALER, MIN_RECORD_POINTS, WaveState,
    effective_tick_hz, format_record_duration, nearest_sampling_prescaler,
    sampling_prescaler_steps,
};

// ---------------------------------------------------------------------------
// Wave section
// ---------------------------------------------------------------------------

const TRIGGER_SOURCE_SELECTED_MAX_CHARS: usize = 20;
const TRIGGER_SOURCE_BUTTON_WIDTH: f32 = 160.0;
const SYSTEM_TRIGGER_SOURCE_PREFIX: &str = "SYS ";
const GLYPH_SETTINGS: &str = "\u{eb52}";
const WAVE_CONTROL_SETTINGS_POPUP_WIDTH: f32 = 280.0;
const SETTINGS_POPUP_GAP: f32 = 10.0;

/// Actions produced by the wave panel that the caller must handle.
pub enum WaveAction {
    ArmCapture,
    Stop,
    Restart(ScopeMode),
}

#[derive(Debug, Clone, Copy)]
pub struct WavePermissions {
    pub can_start: bool,
    pub can_edit_variable_refs: bool,
}

fn is_system_var_name(name: &str, inspector: &InspectorState) -> bool {
    inspector.is_system_variable_name(name)
}

fn truncate_from_start(name: &str, max_chars: usize) -> String {
    let char_count = name.chars().count();
    if char_count <= max_chars {
        return name.to_owned();
    }
    let marker = "...";
    let marker_chars = marker.chars().count();
    if max_chars <= marker_chars {
        return name.chars().skip(char_count - max_chars).collect();
    }
    let tail_chars = max_chars - marker_chars;
    let tail: String = name.chars().skip(char_count - tail_chars).collect();
    format!("{marker}{tail}")
}

fn trigger_source_selected_label(name: &str, inspector: &InspectorState) -> String {
    if is_system_var_name(name, inspector) {
        let name_max_chars =
            TRIGGER_SOURCE_SELECTED_MAX_CHARS - SYSTEM_TRIGGER_SOURCE_PREFIX.chars().count();
        format!(
            "{SYSTEM_TRIGGER_SOURCE_PREFIX}{}",
            truncate_from_start(name, name_max_chars)
        )
    } else {
        truncate_from_start(name, TRIGGER_SOURCE_SELECTED_MAX_CHARS)
    }
}

fn trigger_source_width(ui: &egui::Ui, name: &str, inspector: &InspectorState) -> f32 {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let text_width = ui
        .painter()
        .layout_no_wrap(name.to_owned(), font_id, theme::TEXT_DEFAULT)
        .size()
        .x;
    if is_system_var_name(name, inspector) {
        text_width + theme::system_variable_badge_size().x + theme::SYSTEM_VARIABLE_BADGE_GAP
    } else {
        text_width
    }
}

fn trigger_source_popup_width(
    ui: &egui::Ui,
    pane_vars: &[String],
    inspector: &InspectorState,
) -> f32 {
    const MIN_WIDTH: f32 = 90.0;
    const OUTER_MARGIN: f32 = 24.0;

    let auto_width = trigger_source_width(ui, "Auto", inspector);
    let var_width = pane_vars
        .iter()
        .map(|name| trigger_source_width(ui, name, inspector))
        .fold(auto_width, f32::max);
    let padded = var_width + ui.spacing().button_padding.x * 2.0 + 24.0;
    let content_max = (ui.ctx().content_rect().width() - OUTER_MARGIN).max(MIN_WIDTH);
    padded.clamp(MIN_WIDTH, content_max)
}

fn selectable_trigger_source(
    ui: &mut egui::Ui,
    selected: bool,
    name: &str,
    inspector: &InspectorState,
) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        let bg_rect = rect.expand(visuals.expansion);
        if selected || response.hovered() {
            ui.painter()
                .rect_filled(bg_rect, visuals.corner_radius, visuals.weak_bg_fill);
        }

        let is_system_variable = is_system_var_name(name, inspector);
        let mut text_x = rect.left() + ui.spacing().button_padding.x;
        if is_system_variable {
            text_x +=
                theme::paint_system_variable_badge(ui, egui::pos2(text_x, rect.center().y), 1.0);
        }
        ui.painter().text(
            egui::pos2(text_x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::TextStyle::Button.resolve(ui.style()),
            if selected {
                theme::TEXT_STRONG
            } else {
                theme::TEXT_DEFAULT
            },
        );
    }
    response
}

pub fn show_wave_section(
    ui: &mut egui::Ui,
    wave: &mut WaveState,
    connected: bool,
    tick_hz: Option<u32>,
    tiles: &egui_tiles::Tiles<crate::wave::pane::ViewPane>,
    inspector: &InspectorState,
    record_max_points: Option<u16>,
    permissions: WavePermissions,
) -> Option<WaveAction> {
    let mut action = None;
    let tick_hz = effective_tick_hz(tick_hz.unwrap_or(DEFAULT_TICK_HZ));
    let WavePermissions {
        can_start,
        can_edit_variable_refs,
    } = permissions;

    theme::section_header(ui, "Wave");
    ui.add_space(4.0);

    let gear_rect = ui
        .horizontal(|ui| {
            let spacing = ui.spacing().item_spacing.x;
            let gear_w = theme::ACTION_BUTTON_HEIGHT + 4.0;
            let button_w = ((ui.available_width() - gear_w - spacing * 2.0) / 2.0).max(0.0);
            if theme::action_button_w(
                ui,
                "Capture",
                theme::SELECT_BG,
                connected && can_start && !wave.active,
                button_w,
            ) {
                action = Some(WaveAction::ArmCapture);
            }
            if theme::action_button_w(
                ui,
                "Stop",
                theme::RED,
                wave.active || wave.restart_pending.is_some(),
                button_w,
            ) {
                action = Some(WaveAction::Stop);
            }

            let resp =
                theme::action_button_response_w(ui, GLYPH_SETTINGS, theme::WIDGET_BG, true, gear_w)
                    .on_hover_text("Wave control settings");
            if resp.clicked() {
                wave.show_control_settings = !wave.show_control_settings;
            }
            resp.rect
        })
        .inner;
    show_wave_control_settings_popup(ui, wave, gear_rect);
    if wave.active {
        let mode = if wave.settings_snapshot.trigger_source.is_none()
            && matches!(
                wave.mode,
                ScopeMode::CaptureArmed | ScopeMode::CapturePost | ScopeMode::CaptureFrozen
            ) {
            "auto capture"
        } else {
            mode_label(wave.mode)
        };
        ui.colored_label(
            theme::GREEN,
            format!("{} channels, {mode}", wave.binding.len()),
        );
    } else {
        ui.colored_label(egui::Color32::GRAY, "Wave stopped");
    }

    ui.add_space(2.0);
    ui.separator();

    let mut any_dragging = false;

    show_sampling_controls(ui, wave, tick_hz, record_max_points, &mut any_dragging);

    ui.add_space(2.0);
    ui.separator();

    let pane_vars = super::collect_time_series_vars(tiles);

    ui.add_enabled_ui(can_edit_variable_refs, |ui| {
        ui.horizontal(|ui| {
            ui.label("Trigger");
            let source_popup_width = trigger_source_popup_width(ui, &pane_vars, inspector);
            let ch_label: egui::WidgetText = match &wave.settings.trigger_source {
                None => "Auto".into(),
                Some(name) => trigger_source_selected_label(name, inspector).into(),
            };
            egui::ComboBox::from_id_salt("trg_ch")
                .width(TRIGGER_SOURCE_BUTTON_WIDTH)
                .selected_text(ch_label)
                .truncate()
                .show_ui(ui, |ui| {
                    ui.set_min_width(source_popup_width);
                    ui.selectable_value(&mut wave.settings.trigger_source, None, "Auto")
                        .on_hover_text(
                            "Capture the next complete frame without a trigger threshold",
                        );
                    for name in &pane_vars {
                        let selected = wave.settings.trigger_source.as_ref() == Some(name);
                        let mut response = selectable_trigger_source(ui, selected, name, inspector);
                        if response.clicked() && !selected {
                            wave.settings.trigger_source = Some(name.clone());
                            response.mark_changed();
                            ui.close();
                        }
                    }
                });
            ui.add_enabled_ui(wave.settings.trigger_source.is_some(), |ui| {
                egui::ComboBox::from_id_salt("trg_edge")
                    .width(45.0)
                    .selected_text(trigger_edge_label(wave.settings.trigger_edge))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut wave.settings.trigger_edge,
                            TriggerEdge::Rise,
                            "Rise",
                        );
                        ui.selectable_value(
                            &mut wave.settings.trigger_edge,
                            TriggerEdge::Fall,
                            "Fall",
                        );
                    });
            });
        })
    });

    ui.horizontal(|ui| {
        let edge_trigger_enabled = wave.settings.trigger_source.is_some();
        ui.label("Level");
        let edit_id = ui.make_persistent_id("wave_trigger_level_edit");
        let mut value = ui
            .data_mut(|data| data.get_temp::<f32>(edit_id))
            .unwrap_or(wave.settings.trigger_level);
        let response = ui.add_enabled(
            edge_trigger_enabled,
            egui::DragValue::new(&mut value)
                .speed(0.1)
                .update_while_editing(false),
        );
        commit_deferred_edit(
            ui,
            edit_id,
            value,
            &mut wave.settings.trigger_level,
            &response,
        );
        any_dragging |= response.dragged();

        ui.label("Hyst");
        let edit_id = ui.make_persistent_id("wave_trigger_hysteresis_edit");
        let mut value = ui
            .data_mut(|data| data.get_temp::<f32>(edit_id))
            .unwrap_or(wave.settings.trigger_hysteresis);
        let response = ui.add_enabled(
            edge_trigger_enabled,
            egui::DragValue::new(&mut value)
                .range(0.0..=f32::MAX)
                .speed(0.01)
                .update_while_editing(false),
        );
        commit_deferred_edit(
            ui,
            edit_id,
            value,
            &mut wave.settings.trigger_hysteresis,
            &response,
        );
        any_dragging |= response.dragged();

        ui.label("Pre");
        let edit_id = ui.make_persistent_id("wave_pre_trigger_percent_edit");
        let mut value = ui
            .data_mut(|data| data.get_temp::<u8>(edit_id))
            .unwrap_or(wave.settings.pre_trigger_percent);
        let response = ui.add(
            egui::DragValue::new(&mut value)
                .range(0..=100)
                .speed(1.0)
                .update_while_editing(false)
                .suffix("%"),
        );
        commit_deferred_edit(
            ui,
            edit_id,
            value,
            &mut wave.settings.pre_trigger_percent,
            &response,
        );
        any_dragging |= response.dragged();
    });

    wave.settings.clamp();

    if can_start && wave.active && action.is_none() && !any_dragging {
        let settings_changed = settings_changed_for_mode(
            wave.mode,
            &wave.settings,
            &wave.settings_snapshot,
            normalized_record_max_points(record_max_points),
        );
        let channels_changed = pane_vars != wave.pane_vars_snapshot;
        if settings_changed || channels_changed {
            action = Some(WaveAction::Restart(restart_entry_mode(wave.mode)));
        }
    }

    action
}

fn show_wave_control_settings_popup(ui: &egui::Ui, wave: &mut WaveState, anchor: egui::Rect) {
    if !wave.show_control_settings {
        return;
    }

    let popup_id = egui::Id::new("wave_control_settings_popup");
    let popup_width = WAVE_CONTROL_SETTINGS_POPUP_WIDTH;
    let pos = settings_popup_pos(ui, anchor, popup_width);

    let resp = egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .pivot(egui::Align2::LEFT_TOP)
        .default_width(popup_width)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .shadow(settings_popup_shadow())
                .show(ui, |ui| {
                    ui.set_width(popup_width);

                    theme::modal_title(ui, "Wave Control Settings");
                    ui.add_space(8.0);
                    ui.checkbox(
                        &mut wave.control.capture_on_system_start,
                        "Capture follows System Start",
                    );
                    ui.add_space(4.0);
                    ui.checkbox(
                        &mut wave.control.stop_on_system_stop,
                        "Stop follows System Stop",
                    );
                });
        });

    if settings_popup_clicked_outside(ui, resp.response.rect, anchor) {
        wave.show_control_settings = false;
    }
}

fn settings_popup_pos(ui: &egui::Ui, anchor: egui::Rect, width: f32) -> egui::Pos2 {
    let content_rect = ui.ctx().content_rect();
    let right_x = anchor.right() + SETTINGS_POPUP_GAP;
    let left_x = anchor.left() - SETTINGS_POPUP_GAP - width;
    let popup_x = if right_x + width <= content_rect.right() || left_x < content_rect.left() {
        right_x
    } else {
        left_x
    }
    .clamp(
        content_rect.left(),
        (content_rect.right() - width).max(content_rect.left()),
    );
    egui::pos2(popup_x, anchor.top().max(content_rect.top()))
}

fn settings_popup_shadow() -> egui::Shadow {
    egui::Shadow {
        blur: 40,
        offset: [0, 10],
        spread: 0,
        color: egui::Color32::from_black_alpha(192),
    }
}

fn settings_popup_clicked_outside(
    ui: &egui::Ui,
    popup_rect: egui::Rect,
    anchor: egui::Rect,
) -> bool {
    ui.ctx().input(|input| input.pointer.any_pressed())
        && !popup_rect.contains(
            ui.ctx()
                .input(|input| input.pointer.interact_pos().unwrap_or_default()),
        )
        && !anchor.contains(
            ui.ctx()
                .input(|input| input.pointer.interact_pos().unwrap_or_default()),
        )
}

fn commit_deferred_edit<T>(
    ui: &mut egui::Ui,
    edit_id: egui::Id,
    edited_value: T,
    committed_value: &mut T,
    response: &egui::Response,
) where
    T: Copy + Send + Sync + 'static,
{
    if response.changed() {
        ui.data_mut(|data| data.insert_temp(edit_id, edited_value));
    }
    if response.drag_stopped() || (response.changed() && !response.dragged()) {
        *committed_value = edited_value;
        ui.data_mut(|data| data.remove::<T>(edit_id));
    }
}

fn settings_changed_for_mode(
    mode: ScopeMode,
    settings: &super::AcquisitionSettings,
    snapshot: &super::AcquisitionSettings,
    record_max_points: Option<u16>,
) -> bool {
    let mut settings = settings.with_record_point_fallback(record_max_points);
    let mut snapshot = snapshot.clone();
    snapshot.clamp();
    if mode == ScopeMode::Stream {
        settings.record_points = 0;
        snapshot.record_points = 0;
    }
    settings != snapshot
}

fn restart_entry_mode(mode: ScopeMode) -> ScopeMode {
    match mode {
        ScopeMode::Stream => ScopeMode::Stream,
        ScopeMode::CaptureArmed | ScopeMode::CapturePost | ScopeMode::CaptureFrozen => {
            ScopeMode::CaptureArmed
        }
        ScopeMode::Off | ScopeMode::Unknown(_) => mode,
    }
}

fn show_sampling_controls(
    ui: &mut egui::Ui,
    wave: &mut WaveState,
    tick_hz: u32,
    record_max_points: Option<u16>,
    any_dragging: &mut bool,
) {
    let record_max_points = normalized_record_max_points(record_max_points);
    ui.horizontal(|ui| {
        ui.label("Sampling");
        let steps = sampling_steps_for_ui(tick_hz, wave.settings.prescaler);
        let current_idx = steps
            .iter()
            .position(|&prescaler| prescaler == wave.settings.prescaler)
            .unwrap_or(0);
        let edit_id = ui.make_persistent_id("wave_sampling_step_edit");
        let max_step_idx = steps.len().saturating_sub(1) as i32;
        let mut step_idx = ui
            .data_mut(|data| data.get_temp::<i32>(edit_id))
            .unwrap_or(current_idx as i32)
            .clamp(0, max_step_idx);
        let format_steps = steps.clone();
        let parse_steps = steps.clone();
        let response = ui.add(
            egui::DragValue::new(&mut step_idx)
                .range(0..=max_step_idx)
                .speed(0.08)
                .custom_formatter(move |value, _| {
                    let idx = sampling_step_index(value, format_steps.len());
                    format_sampling_interval_us(prescaler_interval_us(format_steps[idx], tick_hz))
                })
                .custom_parser(move |text| {
                    parse_sampling_interval_us(text).map(|interval_us| {
                        let prescaler =
                            nearest_sampling_prescaler(tick_hz, interval_us, &parse_steps);
                        parse_steps
                            .iter()
                            .position(|&step| step == prescaler)
                            .unwrap_or(0) as f64
                    })
                })
                .update_while_editing(false),
        );
        if response.changed() {
            ui.data_mut(|data| data.insert_temp(edit_id, step_idx));
        }
        if response.drag_stopped() || (response.changed() && !response.dragged()) {
            let idx = sampling_step_index(f64::from(step_idx), steps.len());
            wave.settings.prescaler = steps[idx];
            ui.data_mut(|data| data.remove::<i32>(edit_id));
        }
        *any_dragging |= response.dragged();
        let preview_idx = sampling_step_index(f64::from(step_idx), steps.len());
        let mut preview_settings = wave.settings.clone();
        preview_settings.prescaler = steps[preview_idx];
        ui.weak(format_rate(preview_settings.sample_rate_hz(tick_hz)));
    });
    ui.horizontal(|ui| {
        ui.label("Record");
        let response = ui.add(
            egui::DragValue::new(&mut wave.settings.record_points)
                .range(MIN_RECORD_POINTS..=MAX_RECORD_POINTS_ABSOLUTE)
                .speed(10.0)
                .update_while_editing(false)
                .suffix(" pts"),
        );
        *any_dragging |= response.dragged();
        let effective_settings = wave.settings.with_record_point_fallback(record_max_points);
        ui.weak(format_record_duration(
            effective_settings.record_duration_seconds(tick_hz),
        ));
    });
    if let Some(record_max_points) = record_max_points
        && wave.settings.record_points > record_max_points
    {
        ui.colored_label(
            theme::YELLOW,
            format!(
                "Record fallback: requested {} pts; using {record_max_points} pts for current channels",
                wave.settings.record_points
            ),
        );
    }
}

fn normalized_record_max_points(record_max_points: Option<u16>) -> Option<u16> {
    record_max_points.map(|value| value.clamp(MIN_RECORD_POINTS, MAX_RECORD_POINTS_ABSOLUTE))
}

fn sampling_steps_for_ui(tick_hz: u32, current_prescaler: u16) -> Vec<u16> {
    let mut steps = sampling_prescaler_steps(tick_hz);
    let current_prescaler = current_prescaler.clamp(MIN_PRESCALER, super::MAX_PRESCALER);
    if !steps.contains(&current_prescaler) {
        steps.push(current_prescaler);
        steps.sort_unstable();
    }
    steps
}

fn sampling_step_index(value: f64, len: usize) -> usize {
    (value.round() as isize).clamp(0, len.saturating_sub(1) as isize) as usize
}

fn prescaler_interval_us(prescaler: u16, tick_hz: u32) -> f64 {
    f64::from(prescaler.max(1)) * 1_000_000.0 / f64::from(effective_tick_hz(tick_hz))
}

fn format_sampling_interval_us(interval_us: f64) -> String {
    if interval_us >= 1_000_000.0 {
        format_compact_time(interval_us / 1_000_000.0, "s")
    } else if interval_us >= 1_000.0 {
        format_compact_time(interval_us / 1_000.0, "ms")
    } else {
        format_compact_time(interval_us, "us")
    }
}

fn format_compact_time(value: f64, unit: &str) -> String {
    let decimals = if value >= 100.0 || value.fract() == 0.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        3
    };
    let mut text = format!("{value:.decimals$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    format!("{text} {unit}")
}

fn parse_sampling_interval_us(text: &str) -> Option<f64> {
    let text = text.trim().to_ascii_lowercase();
    let (number, multiplier) = if let Some(value) = text.strip_suffix("ms") {
        (value.trim(), 1_000.0)
    } else if let Some(value) = text.strip_suffix("us") {
        (value.trim(), 1.0)
    } else if let Some(value) = text.strip_suffix('s') {
        (value.trim(), 1_000_000.0)
    } else {
        (text.as_str(), 1.0)
    };
    number.parse::<f64>().ok().map(|value| value * multiplier)
}

fn format_rate(hz: f64) -> String {
    if hz >= 1_000_000.0 {
        format!("{:.3} MHz", hz / 1_000_000.0)
    } else if hz >= 1_000.0 {
        format!("{:.3} kHz", hz / 1_000.0)
    } else {
        format!("{:.3} Hz", hz)
    }
}

fn mode_label(mode: ScopeMode) -> &'static str {
    match mode {
        ScopeMode::Off => "off",
        ScopeMode::Stream => "stream",
        ScopeMode::CaptureArmed => "capture",
        ScopeMode::CapturePost => "capture post",
        ScopeMode::CaptureFrozen => "capture frozen",
        ScopeMode::Unknown(_) => "unknown",
    }
}

fn trigger_edge_label(edge: TriggerEdge) -> &'static str {
    match edge {
        TriggerEdge::Rise => "Rise",
        TriggerEdge::Fall => "Fall",
    }
}

// ---------------------------------------------------------------------------
// CSV Export section
// ---------------------------------------------------------------------------

const CSV_SETTINGS_POPUP_WIDTH: f32 = 480.0;

/// Actions produced by the CSV export panel.
pub enum CsvAction {
    QuickSnapshot,
    SaveWithDialog,
}

pub fn show_csv_export(
    ui: &mut egui::Ui,
    csv: &mut CsvState,
    has_data: bool,
    filename_preview: Option<&str>,
) -> Option<CsvAction> {
    let mut action = None;

    theme::section_header(ui, "CSV Export");
    ui.add_space(4.0);

    let saving = csv.save_rx.is_some()
        || csv.pending_screenshot_path.is_some()
        || csv.screenshot_save_rx.is_some();

    // ---- Button row: [Save Data / Ultra Fast Snapshot (wide)] [gear (icon)] ----
    let gear_rect = ui
        .horizontal(|ui| {
            let spacing = ui.spacing().item_spacing.x;
            let gear_w = theme::ACTION_BUTTON_HEIGHT + 4.0;
            let save_w = ui.available_width() - gear_w - spacing;

            if csv.ultra_fast {
                if theme::action_button_w(
                    ui,
                    "Ultra Fast Snapshot",
                    theme::SELECT_BG,
                    has_data && !saving,
                    save_w,
                ) {
                    action = Some(CsvAction::QuickSnapshot);
                }
            } else if theme::action_button_w(
                ui,
                "Save Data",
                theme::SELECT_BG,
                has_data && !saving,
                save_w,
            ) {
                action = Some(CsvAction::SaveWithDialog);
            }

            // Settings gear button
            let resp =
                theme::action_button_response_w(ui, GLYPH_SETTINGS, theme::WIDGET_BG, true, gear_w);
            if resp.clicked() {
                csv.show_settings = !csv.show_settings;
            }
            resp.rect
        })
        .inner;

    // ---- Filename preview (only in ultra-fast mode) ----
    if csv.ultra_fast
        && let Some(preview) = filename_preview
    {
        ui.label(egui::RichText::new(format!("{preview}.csv")).color(theme::TEXT_SUBDUED));
    }

    // ---- Settings popup + Overwrite modal ----
    show_csv_settings_popup(ui, csv, gear_rect);

    action
}

fn show_csv_settings_popup(ui: &egui::Ui, csv: &mut CsvState, anchor: egui::Rect) {
    if !csv.show_settings {
        return;
    }

    let popup_id = egui::Id::new("csv_settings_popup");
    let pos = settings_popup_pos(ui, anchor, CSV_SETTINGS_POPUP_WIDTH);

    let resp = egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .pivot(egui::Align2::LEFT_TOP)
        .default_width(CSV_SETTINGS_POPUP_WIDTH)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .shadow(settings_popup_shadow())
                .show(ui, |ui| {
                    ui.set_width(CSV_SETTINGS_POPUP_WIDTH);

                    theme::modal_title(ui, "CSV Export Settings");
                    ui.add_space(8.0);

                    ui.checkbox(&mut csv.save_with_screenshot, "Save with screenshot");
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.checkbox(&mut csv.ultra_fast, "Ultra Fast Snapshot");
                    ui.add_space(4.0);

                    let editable = csv.ultra_fast;

                    // Directory
                    ui.horizontal(|ui| {
                        ui.label("Dir ");
                        ui.add_enabled(
                            editable,
                            egui::TextEdit::singleline(&mut csv.snapshot_dir)
                                .desired_width(ui.available_width() - 30.0)
                                .hint_text("output directory"),
                        );
                        ui.add_enabled_ui(editable, |ui| {
                            if ui.small_button("...").clicked()
                                && let Some(dir) = rfd::FileDialog::new().pick_folder()
                            {
                                csv.snapshot_dir = dir.to_string_lossy().to_string();
                            }
                        });
                    });

                    // Filename template
                    ui.horizontal(|ui| {
                        ui.label("File");
                        ui.add_enabled(
                            editable,
                            egui::TextEdit::singleline(&mut csv.filename_template)
                                .desired_width(ui.available_width())
                                .hint_text("wave_{$DateTime}"),
                        );
                    });

                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(
                            "Tokens: {$Date} {$Time} {$DateTime} {$var} {$var:.2f}",
                        )
                        .color(theme::TEXT_SUBDUED),
                    );
                });
        });

    // Close when clicking outside the popup (but not on the gear button itself)
    if settings_popup_clicked_outside(ui, resp.response.rect, anchor) {
        csv.show_settings = false;
    }
}
