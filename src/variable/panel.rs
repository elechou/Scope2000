use std::cell::Cell;

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::theme;
use crate::wave::dnd::{self, DragSource, DragSurface, DropAction, VarDragPayload};

use super::{DescriptorEntry, InspectorState, WatchEntry};

const ICON_ARROW_LEFT: &str = "\u{f060}";
const ICON_REFRESH: &str = "\u{f021}";
const ICON_CONTINUOUS_REFRESH: &str = "\u{f01e}";
const ICON_TRASH: &str = "\u{f1f8}";
const DISPLAY_NAME_MAX_CHARS: usize = 24;
const SYSTEM_DISPLAY_NAME_MAX_CHARS: usize = 20;
const VALUE_SIGNIFICANT_DIGITS: usize = 6;
const VARMAP_TRASH_HEIGHT: f32 = 44.0;
const VARMAP_TRASH_MARGIN: f32 = 6.0;
const VAR_CTRL_ARROW_W: f32 = 24.0;
const VAR_CTRL_MIN_TEXT_COL_W: f32 = 40.0;

fn display_name(name: &str, is_system_variable: bool) -> String {
    let max_chars = if is_system_variable {
        SYSTEM_DISPLAY_NAME_MAX_CHARS
    } else {
        DISPLAY_NAME_MAX_CHARS
    };
    truncate_from_start(name, max_chars)
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

fn text_width(ui: &egui::Ui, text: &str, font_id: &egui::FontId, color: egui::Color32) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font_id.clone(), color)
        .size()
        .x
}

fn truncate_from_start_to_width(
    ui: &egui::Ui,
    text: &str,
    font_id: &egui::FontId,
    color: egui::Color32,
    max_width: f32,
) -> String {
    if max_width <= 0.0 || text.is_empty() {
        return String::new();
    }
    if text_width(ui, text, font_id, color) <= max_width {
        return text.to_owned();
    }

    let mut char_starts: Vec<usize> = text.char_indices().map(|(idx, _)| idx).collect();
    char_starts.push(text.len());
    let char_count = char_starts.len().saturating_sub(1);
    let tail_from = |tail_chars: usize| -> &str {
        let start_idx = char_count.saturating_sub(tail_chars);
        &text[char_starts[start_idx]..]
    };

    let marker = "...";
    if text_width(ui, marker, font_id, color) <= max_width {
        let mut low = 0;
        let mut high = char_count;
        while low < high {
            let mid = (low + high).div_ceil(2);
            let candidate = format!("{marker}{}", tail_from(mid));
            if text_width(ui, &candidate, font_id, color) <= max_width {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        format!("{marker}{}", tail_from(low))
    } else {
        let mut low = 0;
        let mut high = char_count;
        while low < high {
            let mid = (low + high).div_ceil(2);
            let tail = tail_from(mid);
            if text_width(ui, tail, font_id, color) <= max_width {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        tail_from(low).to_owned()
    }
}

fn variable_name_table_cell(
    ui: &mut egui::Ui,
    name: &str,
    is_system_variable: bool,
    color: egui::Color32,
    row_alpha: f32,
) -> bool {
    let available = ui.available_size_before_wrap();
    let height = if available.y.is_finite() && available.y > 0.0 {
        available.y
    } else {
        ui.text_style_height(&egui::TextStyle::Body)
    };
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(available.x.max(0.0), height),
        egui::Sense::hover(),
    );

    let mut text_left = rect.left();
    if is_system_variable {
        text_left += theme::paint_system_variable_badge(ui, rect.left_center(), row_alpha);
    }
    let text_rect =
        egui::Rect::from_min_max(egui::pos2(text_left, rect.top()), rect.right_bottom());
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let display =
        truncate_from_start_to_width(ui, name, &font_id, color, text_rect.width().max(0.0));

    if ui.is_rect_visible(rect) && !display.is_empty() {
        ui.painter().text(
            text_rect.left_center(),
            egui::Align2::LEFT_CENTER,
            &display,
            font_id,
            color,
        );
    }

    display != name
}

fn variable_controller_min_text_width(flexible_w: f32) -> f32 {
    VAR_CTRL_MIN_TEXT_COL_W.min(flexible_w / 3.0)
}

fn clamp_variable_controller_name_width(name_w: f32, flexible_w: f32) -> f32 {
    if flexible_w <= 0.0 {
        return 0.0;
    }

    let min_text_w = variable_controller_min_text_width(flexible_w);
    let max_name_w = (flexible_w - min_text_w * 2.0).max(min_text_w);
    name_w.clamp(min_text_w, max_name_w)
}

fn variable_controller_column_widths(ui: &egui::Ui, flexible_w: f32) -> [f32; 3] {
    let id = ui.id().with("var_ctrl_name_width_v1");
    let default_name_w = flexible_w * 0.4;
    let name_w = ui
        .data_mut(|data| data.get_temp::<f32>(id))
        .unwrap_or(default_name_w);
    let name_w = clamp_variable_controller_name_width(name_w, flexible_w);
    let shared_w = ((flexible_w - name_w) * 0.5).max(0.0);
    [name_w, shared_w, shared_w]
}

fn show_variable_controller_resize_handles(
    ui: &mut egui::Ui,
    table_left: f32,
    table_top: f32,
    table_bottom: f32,
    spacing_x: f32,
    flexible_w: f32,
    name_w: f32,
) {
    let id = ui.id().with("var_ctrl_name_width_v1");
    let handle_x = table_left + name_w + spacing_x * 0.5;
    let resize_radius = ui.style().interaction.resize_grab_radius_side;

    let line_rect = egui::Rect::from_min_max(
        egui::pos2(handle_x, table_top),
        egui::pos2(handle_x, table_bottom),
    )
    .expand(resize_radius);
    let response = ui.interact(
        line_rect,
        ui.id().with("var_ctrl_resize_handle"),
        egui::Sense::click_and_drag(),
    );

    if response.dragged()
        && let Some(pointer) = ui.ctx().pointer_latest_pos()
    {
        let dx = pointer.x - handle_x;
        let next_name_w = clamp_variable_controller_name_width(name_w + dx, flexible_w);
        ui.data_mut(|data| data.insert_temp(id, next_name_w));
        ui.ctx().request_repaint();
    }

    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
    }

    let stroke = if response.dragged() {
        ui.style().visuals.widgets.active.bg_stroke
    } else if response.hovered() {
        ui.style().visuals.widgets.hovered.bg_stroke
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };
    ui.painter().line_segment(
        [
            egui::pos2(handle_x, table_top),
            egui::pos2(handle_x, table_bottom),
        ],
        stroke,
    );
}

fn trim_float_text(text: String) -> String {
    let Some(exp_pos) = text.find('e').or_else(|| text.find('E')) else {
        return trim_decimal_zeros(text);
    };
    let mantissa = trim_decimal_zeros(text[..exp_pos].to_owned());
    let exponent = text[exp_pos..].trim_start_matches('e');
    format!("{mantissa}e{exponent}")
}

fn trim_decimal_zeros(mut text: String) -> String {
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    if text == "-0" { "0".to_owned() } else { text }
}

fn format_short_value(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if !value.is_finite() {
        return value.to_string();
    }

    let exponent = value.abs().log10().floor() as i32;
    let significant = VALUE_SIGNIFICANT_DIGITS as i32;
    if (-4..significant).contains(&exponent) {
        let decimals = (significant - exponent - 1).max(0) as usize;
        trim_float_text(format!("{value:.decimals$}"))
    } else {
        let precision = VALUE_SIGNIFICANT_DIGITS.saturating_sub(1);
        trim_float_text(format!("{value:.precision$e}"))
    }
}

fn format_map_value(value: Option<f64>) -> String {
    value
        .map(format_short_value)
        .unwrap_or_else(|| "-".to_owned())
}

fn format_full_value(value: Option<f64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn variable_hover_text(name: &str, value: Option<f64>) -> String {
    format!("{name}    {}", format_full_value(value))
}

fn write_select_button(
    ui: &mut egui::Ui,
    selected: bool,
    enabled: bool,
    row_alpha: f32,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let height = ui.spacing().interact_size.y;
    let desired = egui::vec2(ui.available_width(), height);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(desired, sense);
    if ui.is_rect_visible(rect) {
        if selected {
            ui.painter()
                .rect_filled(rect, 3.0, theme::SELECT_BG.gamma_multiply(row_alpha));
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, 3.0, theme::WIDGET_HOVER.gamma_multiply(row_alpha));
        }
        if selected || resp.hovered() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                ICON_ARROW_LEFT,
                egui::TextStyle::Body.resolve(ui.style()),
                theme::TEXT_STRONG.gamma_multiply(row_alpha),
            );
        }
    }
    resp
}

#[derive(Default)]
struct VariableMapHeaderOutput {
    refresh_requested: bool,
    continuous_refresh_changed: bool,
}

fn variable_map_header(
    ui: &mut egui::Ui,
    continuous_refresh: &mut bool,
    can_refresh: bool,
) -> VariableMapHeaderOutput {
    let desired = egui::vec2(ui.max_rect().width(), 24.0);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let paint_rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range())
        .expand2(egui::vec2(8.0, 0.0));

    if ui.is_rect_visible(rect) {
        let mut painter = ui.painter().clone();
        painter.set_clip_rect(paint_rect);
        painter.rect_filled(paint_rect, 0.0, theme::TAB_BAR);
        painter.text(
            paint_rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "Variable Map",
            egui::TextStyle::Body.resolve(ui.style()),
            theme::TEXT_STRONG,
        );
    }

    let button_h = 20.0;
    let button_w = 22.0;
    let button_y = rect.center().y - button_h * 0.5;
    let gap = 4.0;
    let right_margin = 8.0;
    let continuous_rect = egui::Rect::from_min_size(
        egui::pos2(paint_rect.right() - right_margin - button_w, button_y),
        egui::vec2(button_w, button_h),
    );
    let refresh_rect = egui::Rect::from_min_size(
        egui::pos2(continuous_rect.left() - gap - button_w, button_y),
        egui::vec2(button_w, button_h),
    );

    let refresh_resp = header_icon_button(
        ui,
        response.id.with("refresh"),
        refresh_rect,
        ICON_CONTINUOUS_REFRESH,
        "Refresh",
        can_refresh,
        false,
    );
    let continuous_resp = header_icon_button(
        ui,
        response.id.with("continuous_refresh"),
        continuous_rect,
        ICON_REFRESH,
        if *continuous_refresh {
            "Stop Continue Refresh"
        } else {
            "Continue Refresh"
        },
        can_refresh,
        *continuous_refresh,
    );

    let mut output = VariableMapHeaderOutput::default();
    if refresh_resp.clicked() {
        output.refresh_requested = true;
    }
    if continuous_resp.clicked() {
        *continuous_refresh = !*continuous_refresh;
        output.continuous_refresh_changed = true;
    }
    output
}

fn header_icon_button(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    icon: &str,
    tooltip: &str,
    enabled: bool,
    active: bool,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let response = ui.interact(rect, id, sense);

    if ui.is_rect_visible(rect) {
        let fill = if !enabled {
            theme::WIDGET_BG
        } else if active {
            theme::SELECT_BG
        } else if response.is_pointer_button_down_on() {
            theme::WIDGET_ACTIVE
        } else if response.hovered() {
            theme::WIDGET_HOVER
        } else {
            theme::WIDGET_BG
        };
        let text_color = if enabled {
            theme::TEXT_STRONG
        } else {
            theme::TEXT_SUBDUED
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::TextStyle::Body.resolve(ui.style()),
            text_color,
        );
    }

    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tooltip));
    response.on_hover_text(tooltip)
}

fn pinned_item_ui(
    ui: &mut egui::Ui,
    name: &str,
    value: Option<f64>,
    is_system_variable: bool,
) -> egui::Response {
    let desired = egui::vec2(ui.available_width(), 24.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        let bg = if ui.rect_contains_pointer(rect) {
            theme::WIDGET_HOVER
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 4.0, bg);

        let font_mono = egui::TextStyle::Monospace.resolve(ui.style());
        let mut name_offset = 0.0;
        if is_system_variable {
            name_offset = theme::paint_system_variable_badge(
                ui,
                rect.left_center() + egui::vec2(6.0, 0.0),
                1.0,
            );
        }
        ui.painter().text(
            rect.left_center() + egui::vec2(6.0 + name_offset, 0.0),
            egui::Align2::LEFT_CENTER,
            display_name(name, is_system_variable),
            font_mono.clone(),
            theme::TEXT_DEFAULT,
        );
        ui.painter().text(
            rect.right_center() - egui::vec2(6.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            format_map_value(value),
            font_mono,
            theme::TEXT_DEFAULT,
        );
    }
    resp
}

fn map_item_ui(ui: &mut egui::Ui, name: &str, is_system_variable: bool) -> egui::Response {
    let desired = egui::vec2(ui.available_width(), 24.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        let bg = if ui.rect_contains_pointer(rect) {
            theme::WIDGET_HOVER
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 4.0, bg);

        let font_mono = egui::TextStyle::Monospace.resolve(ui.style());
        let mut name_offset = 0.0;
        if is_system_variable {
            name_offset = theme::paint_system_variable_badge(
                ui,
                rect.left_center() + egui::vec2(6.0, 0.0),
                1.0,
            );
        }
        ui.painter().text(
            rect.left_center() + egui::vec2(6.0 + name_offset, 0.0),
            egui::Align2::LEFT_CENTER,
            display_name(name, is_system_variable),
            font_mono,
            theme::TEXT_DEFAULT,
        );
    }
    resp
}

#[derive(Default)]
pub struct VariableMapOutput {
    pub pinned_changed: bool,
    pub refresh_requested: bool,
    pub continuous_refresh_changed: bool,
    pub delete_request: Option<VarDragPayload>,
}

pub fn show_variable_map(
    ui: &mut egui::Ui,
    inspector: &mut InspectorState,
    filter_text: &mut String,
    split_frac: &mut f32,
    continuous_refresh: &mut bool,
    can_edit_variable_refs: bool,
    can_refresh: bool,
    drop_action: &mut Option<DropAction>,
) -> VariableMapOutput {
    let map_section_top = ui.cursor().top();
    let header_output = variable_map_header(ui, continuous_refresh, can_refresh);
    let mut output = VariableMapOutput {
        refresh_requested: header_output.refresh_requested,
        continuous_refresh_changed: header_output.continuous_refresh_changed,
        ..VariableMapOutput::default()
    };

    if inspector.descriptors.is_empty() {
        ui.add_space(4.0);
        ui.weak("  No descriptors enumerated");
        return output;
    }
    ui.add_space(4.0);

    let splitter_h = 8.0;
    let min_h = 48.0;
    let avail = (ui.available_height() - splitter_h).max(min_h * 2.0);
    let top_h = (avail * *split_frac).clamp(min_h, avail - min_h);
    let has_var_payload = can_edit_variable_refs
        && egui::DragAndDrop::has_payload_of_type::<VarDragPayload>(ui.ctx());
    let dragged_pinned_pos =
        egui::DragAndDrop::payload::<VarDragPayload>(ui.ctx()).and_then(|payload| {
            if !matches!(payload.source, DragSource::VariableMapPinned) {
                return None;
            }
            if payload.names.len() != 1 {
                return None;
            }
            let name = payload.names.first()?;
            inspector.pinned.iter().position(|&idx| {
                inspector
                    .descriptors
                    .get(idx)
                    .is_some_and(|descriptor| descriptor.name == *name)
            })
        });
    let mut insert_at: Option<(usize, Vec<String>)> = None;
    let mut reorder: Option<(usize, usize)> = None;
    let mut to_unpin = None;
    let mut pinned_target_hover = false;
    let pinned_section_top = ui.cursor().top();

    if inspector.pinned.is_empty() {
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), top_h),
            egui::Sense::hover(),
        );
        let hover_payload = can_edit_variable_refs
            .then_some(())
            .and_then(|_| resp.dnd_hover_payload::<VarDragPayload>());
        pinned_target_hover = hover_payload.is_some();
        if let Some(payload) = hover_payload.as_ref() {
            *drop_action = Some(dnd::action_for_target(
                &payload.source,
                DragSurface::VariableMap,
            ));
        }
        if ui.is_rect_visible(rect) {
            let bg = if hover_payload.is_some() {
                theme::DROP_TARGET_STROKE.gamma_multiply(0.24)
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, 4.0, bg);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No pinned variables",
                egui::TextStyle::Body.resolve(ui.style()),
                theme::TEXT_SUBDUED,
            );
            if hover_payload.is_some() {
                ui.painter().hline(
                    rect.x_range(),
                    rect.bottom(),
                    egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE),
                );
            }
        }
        if hover_payload.is_some()
            && let Some(payload) = resp.dnd_release_payload::<VarDragPayload>()
        {
            insert_at = Some((0, payload.names.clone()));
        }
    } else {
        egui::ScrollArea::vertical()
            .id_salt("varmap_watch_scroll")
            .auto_shrink([false, false])
            .max_height(top_h)
            .show(ui, |ui| {
                ui.push_id("varmap_watch", |ui| {
                    for (pos, &idx) in inspector.pinned.iter().enumerate() {
                        let Some(descriptor) = inspector.descriptors.get(idx) else {
                            continue;
                        };
                        let value = inspector.values.get(idx).copied().flatten();
                        let is_system_variable = inspector.is_system_variable_index(idx);
                        let resp = pinned_item_ui(ui, &descriptor.name, value, is_system_variable)
                            .on_hover_text(variable_hover_text(&descriptor.name, value));
                        if can_edit_variable_refs {
                            resp.dnd_set_drag_payload(VarDragPayload {
                                names: vec![descriptor.name.clone()],
                                source: DragSource::VariableMapPinned,
                            });
                        }
                        resp.context_menu(|ui| {
                            if ui
                                .add_enabled(can_edit_variable_refs, egui::Button::new("Unpin"))
                                .clicked()
                            {
                                to_unpin = Some(pos);
                                ui.close();
                            }
                        });

                        let hover_payload = can_edit_variable_refs
                            .then_some(())
                            .and_then(|_| resp.dnd_hover_payload::<VarDragPayload>())
                            .filter(|payload| {
                                let is_self =
                                    matches!(payload.source, DragSource::VariableMapPinned)
                                        && payload.names.len() == 1
                                        && payload.names.first() == Some(&descriptor.name);
                                !is_self
                            });
                        if let Some(payload) = hover_payload.as_ref() {
                            pinned_target_hover = true;
                            *drop_action = Some(dnd::action_for_target(
                                &payload.source,
                                DragSurface::VariableMap,
                            ));
                            let rect = resp.rect;
                            let pointer_y = resp
                                .ctx
                                .input(|input| input.pointer.hover_pos())
                                .map(|pos| pos.y)
                                .unwrap_or(rect.center().y);
                            let insert_above = pointer_y < rect.center().y;
                            let line_y = if insert_above {
                                rect.top()
                            } else {
                                rect.bottom()
                            };
                            let painter = resp.ctx.layer_painter(resp.layer_id);
                            painter.hline(
                                rect.x_range(),
                                line_y,
                                egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE),
                            );
                            if let Some(payload) = resp.dnd_release_payload::<VarDragPayload>() {
                                let target_pos = if insert_above { pos } else { pos + 1 };
                                if let Some(from) = dragged_pinned_pos {
                                    let mut to = target_pos;
                                    if from < to {
                                        to -= 1;
                                    }
                                    if to != from {
                                        reorder = Some((from, to));
                                    }
                                } else {
                                    insert_at = Some((target_pos, payload.names.clone()));
                                }
                            }
                        }
                    }
                });
            });
    }

    let pinned_section_rect = egui::Rect::from_x_y_ranges(
        ui.min_rect().x_range(),
        egui::Rangef::new(pinned_section_top, ui.cursor().top()),
    );
    if has_var_payload && !pinned_target_hover && ui.rect_contains_pointer(pinned_section_rect) {
        if let Some(payload) = egui::DragAndDrop::payload::<VarDragPayload>(ui.ctx()) {
            *drop_action = Some(dnd::action_for_target(
                &payload.source,
                DragSurface::VariableMap,
            ));
        }
        ui.painter().hline(
            pinned_section_rect.x_range(),
            pinned_section_rect.bottom(),
            egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE),
        );
        if ui.ctx().input(|input| input.pointer.any_released())
            && let Some(payload) = egui::DragAndDrop::take_payload::<VarDragPayload>(ui.ctx())
        {
            if let Some(from) = dragged_pinned_pos {
                let mut to = inspector.pinned.len();
                if from < to {
                    to -= 1;
                }
                if to != from {
                    reorder = Some((from, to));
                }
            } else {
                insert_at = Some((inspector.pinned.len(), payload.names.clone()));
            }
        }
    }

    if can_edit_variable_refs
        && let Some(pos) = to_unpin
        && pos < inspector.pinned.len()
    {
        inspector.pinned.remove(pos);
        output.pinned_changed = true;
    }
    if can_edit_variable_refs
        && let Some((from, to)) = reorder
        && from < inspector.pinned.len()
        && from != to
    {
        let item = inspector.pinned.remove(from);
        let insert_pos = to.min(inspector.pinned.len());
        inspector.pinned.insert(insert_pos, item);
        output.pinned_changed = true;
    }
    if can_edit_variable_refs
        && let Some((idx, names)) = insert_at
        && insert_pinned_names(inspector, idx, names)
    {
        output.pinned_changed = true;
    }

    let (bar_rect, bar_resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), splitter_h),
        egui::Sense::drag(),
    );
    let bar_active = bar_resp.hovered() || bar_resp.dragged();
    if bar_active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let bar_color = if bar_active {
        theme::SELECT_STROKE
    } else {
        theme::SEPARATOR
    };
    ui.painter().hline(
        bar_rect.x_range(),
        bar_rect.center().y,
        egui::Stroke::new(1.0, bar_color),
    );
    if bar_resp.dragged() {
        let new_top = (top_h + bar_resp.drag_delta().y).clamp(min_h, avail - min_h);
        *split_frac = new_top / avail;
    }

    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label("Filter:");
        ui.add(
            egui::TextEdit::singleline(filter_text)
                .desired_width(ui.available_width() - 8.0)
                .hint_text("Search descriptor name"),
        );
    });

    let filter = filter_text.to_lowercase();
    let entries = inspector.entries.clone();
    let system_entries = inspector.system_entries.clone();

    egui::ScrollArea::vertical()
        .id_salt("sources_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !system_entries.is_empty() {
                egui::CollapsingHeader::new("System Variables")
                    .default_open(false)
                    .show(ui, |ui| {
                        for entry in &system_entries {
                            show_map_entry(ui, entry, &filter, inspector, can_edit_variable_refs);
                        }
                    });
                ui.add_space(4.0);
                ui.separator();
            }

            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.strong("User Variables:");
            });
            if entries.is_empty() {
                ui.weak("No user variables enumerated");
            } else {
                for entry in &entries {
                    show_map_entry(ui, entry, &filter, inspector, can_edit_variable_refs);
                }
            }
        });

    let map_rect = egui::Rect::from_min_max(
        egui::pos2(ui.max_rect().left(), map_section_top),
        egui::pos2(ui.max_rect().right(), ui.max_rect().bottom()),
    );
    if let Some(payload) = variable_map_trash_drop(ui, map_rect, drop_action) {
        output.delete_request = Some(payload);
    }

    output
}

fn variable_map_trash_drop(
    ui: &mut egui::Ui,
    map_rect: egui::Rect,
    drop_action: &mut Option<DropAction>,
) -> Option<VarDragPayload> {
    let payload = egui::DragAndDrop::payload::<VarDragPayload>(ui.ctx())?;
    if !payload.source.can_delete() {
        return None;
    }

    let trash_rect = egui::Rect::from_min_max(
        egui::pos2(
            map_rect.left() + VARMAP_TRASH_MARGIN,
            map_rect.bottom() - VARMAP_TRASH_MARGIN - VARMAP_TRASH_HEIGHT,
        ),
        egui::pos2(
            map_rect.right() - VARMAP_TRASH_MARGIN,
            map_rect.bottom() - VARMAP_TRASH_MARGIN,
        ),
    );
    let pointer_pos = ui
        .ctx()
        .pointer_interact_pos()
        .or_else(|| ui.ctx().input(|input| input.pointer.hover_pos()));
    let trash_active = pointer_pos.is_some_and(|pos| trash_rect.contains(pos));
    if trash_active {
        *drop_action = Some(DropAction::Delete);
    }

    let layer_id = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("varmap_trash_drop"));
    let painter = ui.ctx().layer_painter(layer_id);
    let fill = if trash_active {
        egui::Color32::from_rgb(236, 60, 75)
    } else {
        theme::RED
    };
    let stroke = egui::Stroke::new(
        if trash_active { 2.0 } else { 1.0 },
        egui::Color32::from_rgb(255, 132, 142),
    );
    painter.rect_filled(trash_rect, egui::CornerRadius::same(4), fill);
    painter.rect_stroke(
        trash_rect,
        egui::CornerRadius::same(4),
        stroke,
        egui::StrokeKind::Inside,
    );

    let font = egui::TextStyle::Button.resolve(ui.style());
    painter.text(
        trash_rect.center() - egui::vec2(24.0, 0.0),
        egui::Align2::CENTER_CENTER,
        ICON_TRASH,
        font.clone(),
        theme::TEXT_STRONG,
    );
    painter.text(
        trash_rect.center() + egui::vec2(10.0, 0.0),
        egui::Align2::CENTER_CENTER,
        "Remove",
        font,
        theme::TEXT_STRONG,
    );

    if trash_active && ui.ctx().input(|input| input.pointer.any_released()) {
        egui::DragAndDrop::take_payload::<VarDragPayload>(ui.ctx())
            .map(|payload| payload.as_ref().clone())
    } else {
        None
    }
}

fn show_map_entry(
    ui: &mut egui::Ui,
    entry: &DescriptorEntry,
    filter: &str,
    inspector: &InspectorState,
    can_edit_variable_refs: bool,
) {
    match entry {
        DescriptorEntry::Var {
            label,
            full_name,
            index,
        } => {
            if !filter.is_empty() && !full_name.to_lowercase().contains(filter) {
                return;
            }
            if inspector.descriptors.get(*index).is_none() {
                return;
            }
            let is_system_variable = inspector.is_system_variable_index(*index);
            let value = inspector.values.get(*index).copied().flatten();
            let resp = map_item_ui(ui, label, is_system_variable)
                .on_hover_text(variable_hover_text(full_name, value));
            if can_edit_variable_refs {
                resp.dnd_set_drag_payload(VarDragPayload {
                    names: vec![full_name.clone()],
                    source: DragSource::VariableMap,
                });
            }
        }
        DescriptorEntry::Group {
            label,
            full_name,
            members,
        } => {
            let mut leaf_names = Vec::new();
            entry.flatten_names(&mut leaf_names);
            let mut leaf_indexes = Vec::new();
            entry.leaf_indexes(&mut leaf_indexes);

            if !filter.is_empty() {
                let group_matches = full_name.to_lowercase().contains(filter);
                let any_leaf_matches = leaf_names
                    .iter()
                    .any(|name| name.to_lowercase().contains(filter));
                if !group_matches && !any_leaf_matches {
                    return;
                }
            }

            let is_system_group = leaf_indexes
                .iter()
                .any(|index| inspector.is_system_variable_index(*index));
            let filtering = !filter.is_empty();
            let header_id = ui.make_persistent_id((full_name, filtering));
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                header_id,
                filtering,
            );

            let header_out = ui.horizontal(|ui| {
                let prev_item_spacing = ui.spacing_mut().item_spacing;
                ui.spacing_mut().item_spacing.x = 0.0;
                state.show_toggle_button(ui, theme::collapsing_arrow_icon);
                ui.spacing_mut().item_spacing = prev_item_spacing;

                let desired = egui::vec2(ui.available_width(), 20.0);
                let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
                if ui.is_rect_visible(rect) {
                    let bg = if ui.rect_contains_pointer(rect) {
                        theme::WIDGET_HOVER
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, 4.0, bg);
                    let mut text_offset = 0.0;
                    if is_system_group {
                        text_offset = theme::paint_system_variable_badge(
                            ui,
                            rect.left_center() + egui::vec2(2.0, 0.0),
                            1.0,
                        );
                    }
                    ui.painter().text(
                        rect.left_center() + egui::vec2(2.0 + text_offset, 0.0),
                        egui::Align2::LEFT_CENTER,
                        display_name(label, is_system_group),
                        egui::TextStyle::Monospace.resolve(ui.style()),
                        theme::TEXT_DEFAULT,
                    );
                }
                let resp = resp.on_hover_text(full_name.clone());
                if can_edit_variable_refs {
                    resp.dnd_set_drag_payload(VarDragPayload {
                        names: leaf_names,
                        source: DragSource::VariableMap,
                    });
                }
            });
            state.show_body_indented(&header_out.response, ui, |ui| {
                for member in members {
                    let child_filter =
                        if !filter.is_empty() && full_name.to_lowercase().contains(filter) {
                            ""
                        } else {
                            filter
                        };
                    show_map_entry(ui, member, child_filter, inspector, can_edit_variable_refs);
                }
            });
        }
    }
}

fn insert_pinned_names(inspector: &mut InspectorState, idx: usize, names: Vec<String>) -> bool {
    let mut pos = idx.min(inspector.pinned.len());
    let mut changed = false;
    for name in names {
        if let Some(descriptor_index) = inspector.index_by_name(&name)
            && !inspector.pinned.contains(&descriptor_index)
        {
            inspector.pinned.insert(pos, descriptor_index);
            pos += 1;
            changed = true;
        }
    }
    changed
}

pub struct VariablesPanelOutput {
    pub to_write: Vec<(usize, f64)>,
    pub watch_changed: bool,
}

pub fn show_variables(
    ui: &mut egui::Ui,
    inspector: &mut InspectorState,
    drop_hover_panel: &mut bool,
    drop_action: &mut Option<DropAction>,
    can_write: bool,
    can_edit_variable_refs: bool,
) -> VariablesPanelOutput {
    theme::section_header(ui, "Variable Controller");
    ui.add_space(4.0);

    let mut watch_changed = false;
    let has_var_payload = can_edit_variable_refs
        && egui::DragAndDrop::has_payload_of_type::<VarDragPayload>(ui.ctx());
    let dragged_row_idx = egui::DragAndDrop::payload::<VarDragPayload>(ui.ctx()).and_then(|p| {
        if !matches!(p.source, DragSource::VariableController) {
            return None;
        }
        p.names.first().and_then(|name| {
            inspector
                .watch_vars
                .iter()
                .position(|watch| &watch.var_name == name)
        })
    });

    let to_remove = Cell::new(None);
    let mut to_write = Vec::new();
    let mut insert_at: Option<(usize, Vec<String>)> = None;
    let mut drop_hover = false;
    let mut reorder: Option<(usize, usize)> = None;
    let section_top = ui.cursor().top();

    let row_h = 20.0;
    let num_rows = inspector.watch_vars.len();
    let saved_hover_bg = ui.visuals().widgets.hovered.bg_fill;
    ui.visuals_mut().widgets.hovered.bg_fill = egui::Color32::TRANSPARENT;

    let table_left = ui.cursor().left();
    let table_top = ui.cursor().top();
    let spacing_x = ui.spacing().item_spacing.x;
    let flexible_w =
        (ui.available_width() - VAR_CTRL_ARROW_W - ui.spacing().item_spacing.x * 3.0).max(0.0);
    let [name_w, value_w, input_w] = variable_controller_column_widths(ui, flexible_w);

    let table = TableBuilder::new(ui)
        .id_salt("var_ctrl_table_v2")
        .striped(true)
        .sense(egui::Sense::click_and_drag())
        .max_scroll_height(192.0)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(name_w))
        .column(Column::exact(value_w))
        .column(Column::exact(VAR_CTRL_ARROW_W).resizable(false))
        .column(Column::exact(input_w).resizable(false));

    table
        .header(row_h, |mut header| {
            header.col(|ui| {
                ui.strong("Name");
            });
            header.col(|ui| {
                ui.strong("Value");
            });
            header.col(|_| {});
            header.col(|ui| {
                ui.strong("Input");
            });
        })
        .body(|body| {
            body.rows(row_h, num_rows, |mut row| {
                let i = row.index();
                let descriptor_index = inspector.watch_vars[i].descriptor_index;
                let is_system_variable = inspector.is_system_variable_index(descriptor_index);
                let watch = &mut inspector.watch_vars[i];
                let value = inspector
                    .values
                    .get(watch.descriptor_index)
                    .copied()
                    .flatten();

                let being_dragged = dragged_row_idx == Some(i);
                let row_alpha = if being_dragged { 0.4 } else { 1.0 };

                let mut name_was_truncated = false;
                let name_resp = row
                    .col(|ui| {
                        let name_color = theme::TEXT_DEFAULT.gamma_multiply(row_alpha);
                        name_was_truncated = variable_name_table_cell(
                            ui,
                            &watch.var_name,
                            is_system_variable,
                            name_color,
                            row_alpha,
                        );
                    })
                    .1;
                let name_resp = if name_was_truncated {
                    name_resp.on_hover_text(watch.var_name.clone())
                } else {
                    name_resp
                };
                let value_resp = row
                    .col(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    value
                                        .map(format_short_value)
                                        .unwrap_or_else(|| "-".to_owned()),
                                )
                                .monospace()
                                .color(theme::TEXT_DEFAULT.gamma_multiply(row_alpha)),
                            )
                            .selectable(false),
                        );
                    })
                    .1;
                row.col(|ui| {
                    let resp = write_select_button(ui, watch.write_selected, can_write, row_alpha);
                    if can_write && resp.clicked() {
                        watch.write_selected = !watch.write_selected;
                    }
                });
                let write_resp = row
                    .col(|ui| {
                        ui.add_enabled(
                            can_write,
                            egui::TextEdit::singleline(&mut watch.write_buf)
                                .desired_width(ui.available_width())
                                .hint_text("value"),
                        );
                    })
                    .1;

                if can_edit_variable_refs && name_resp.drag_started() {
                    egui::DragAndDrop::set_payload(
                        &name_resp.ctx,
                        VarDragPayload {
                            names: vec![watch.var_name.clone()],
                            source: DragSource::VariableController,
                        },
                    );
                }

                let row_resp = row.response();
                let hover_payload = can_edit_variable_refs
                    .then_some(())
                    .and_then(|_| row_resp.dnd_hover_payload::<VarDragPayload>())
                    .filter(|p| {
                        let is_self = matches!(p.source, DragSource::VariableController)
                            && p.names.len() == 1
                            && inspector
                                .watch_vars
                                .get(i)
                                .is_some_and(|watch| watch.var_name == p.names[0]);
                        !is_self
                    });
                if let Some(payload) = hover_payload.as_ref() {
                    drop_hover = true;
                    *drop_action = Some(dnd::action_for_target(
                        &payload.source,
                        DragSurface::VariableController,
                    ));
                    let rect = row_resp.rect;
                    let pointer_y = row_resp
                        .ctx
                        .input(|input| input.pointer.hover_pos())
                        .map(|pos| pos.y)
                        .unwrap_or(rect.center().y);
                    let insert_above = pointer_y < rect.center().y;
                    let line_y = if insert_above {
                        rect.top()
                    } else {
                        rect.bottom()
                    };
                    let painter = row_resp.ctx.layer_painter(row_resp.layer_id);
                    painter.hline(
                        rect.x_range(),
                        line_y,
                        egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE),
                    );
                    if let Some(payload) = row_resp.dnd_release_payload::<VarDragPayload>() {
                        let target_pos = if insert_above { i } else { i + 1 };
                        if dragged_row_idx.is_some() {
                            if let Some(from) = inspector.watch_vars.iter().position(|watch| {
                                payload
                                    .names
                                    .first()
                                    .is_some_and(|name| name == &watch.var_name)
                            }) {
                                let mut to = target_pos;
                                if from < to {
                                    to -= 1;
                                }
                                if to != from {
                                    reorder = Some((from, to));
                                }
                            }
                        } else {
                            insert_at = Some((target_pos, payload.names.clone()));
                        }
                    }
                }

                let remove_menu = |ui: &mut egui::Ui| {
                    if ui
                        .add_enabled(can_edit_variable_refs, egui::Button::new("Remove"))
                        .clicked()
                    {
                        to_remove.set(Some(i));
                        ui.close();
                    }
                };
                name_resp.context_menu(remove_menu);
                value_resp.context_menu(remove_menu);
                write_resp.context_menu(remove_menu);
            });
        });

    ui.visuals_mut().widgets.hovered.bg_fill = saved_hover_bg;
    show_variable_controller_resize_handles(
        ui,
        table_left,
        table_top,
        ui.cursor().top(),
        spacing_x,
        flexible_w,
        name_w,
    );

    let section_rect = egui::Rect::from_x_y_ranges(
        ui.min_rect().x_range(),
        egui::Rangef::new(section_top, ui.cursor().top()),
    );
    if has_var_payload && ui.rect_contains_pointer(section_rect) {
        drop_hover = true;
        if let Some(payload) = egui::DragAndDrop::payload::<VarDragPayload>(ui.ctx()) {
            *drop_action = Some(dnd::action_for_target(
                &payload.source,
                DragSurface::VariableController,
            ));
        }
        ui.painter().hline(
            section_rect.x_range(),
            section_rect.bottom(),
            egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE),
        );

        if ui.ctx().input(|input| input.pointer.any_released())
            && let Some(payload) = egui::DragAndDrop::take_payload::<VarDragPayload>(ui.ctx())
        {
            if let Some(from) = dragged_row_idx {
                let mut to = inspector.watch_vars.len();
                if from < to {
                    to -= 1;
                }
                if to != from {
                    reorder = Some((from, to));
                }
            } else {
                insert_at = Some((inspector.watch_vars.len(), payload.names.clone()));
            }
        }
    }

    if can_edit_variable_refs
        && let Some((from, to)) = reorder
        && from < inspector.watch_vars.len()
        && from != to
    {
        let item = inspector.watch_vars.remove(from);
        let insert_pos = to.min(inspector.watch_vars.len());
        inspector.watch_vars.insert(insert_pos, item);
        watch_changed = true;
    }

    if can_edit_variable_refs && let Some((idx, names)) = insert_at {
        let mut pos = idx.min(inspector.watch_vars.len());
        for name in names {
            if let Some(descriptor_index) = inspector.index_by_name(&name)
                && !inspector
                    .watch_vars
                    .iter()
                    .any(|watch| watch.descriptor_index == descriptor_index)
            {
                inspector.watch_vars.insert(
                    pos,
                    WatchEntry {
                        var_name: name,
                        descriptor_index,
                        write_buf: String::new(),
                        write_selected: false,
                    },
                );
                pos += 1;
                watch_changed = true;
            }
        }
    }

    ui.add_space(ui.spacing().item_spacing.y);
    if theme::action_button_w(
        ui,
        "Write All",
        theme::SELECT_BG,
        can_write,
        ui.available_width(),
    ) {
        for watch in &inspector.watch_vars {
            if watch.write_selected
                && let Ok(value) = watch.write_buf.parse::<f64>()
            {
                to_write.push((watch.descriptor_index, value));
            }
        }
    }
    if can_edit_variable_refs && let Some(i) = to_remove.get() {
        inspector.watch_vars.remove(i);
        watch_changed = true;
    }

    *drop_hover_panel |= drop_hover;

    VariablesPanelOutput {
        to_write,
        watch_changed,
    }
}
