use std::cell::Cell;

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::theme;
use crate::wave::dnd::{DragSource, VarDragPayload};

use super::{DescriptorEntry, InspectorState, WatchEntry};

const ICON_ARROW_LEFT: &str = "\u{f060}";
const ICON_REFRESH: &str = "\u{f021}";
const ICON_REFRESH_OFF: &str = "\u{f4f4}";

fn pin_button(ui: &mut egui::Ui, row_rect: egui::Rect, id_src: &str, pinned: bool) -> bool {
    if !pinned && !ui.rect_contains_pointer(row_rect) {
        return false;
    }
    let icon_rect = egui::Rect::from_center_size(
        row_rect.right_center() - egui::vec2(20.0, 0.0),
        egui::vec2(18.0, 18.0),
    );
    let resp = ui.interact(
        icon_rect,
        ui.id().with(("pin", id_src)),
        egui::Sense::click(),
    );
    if resp.hovered() {
        ui.painter()
            .rect_filled(icon_rect, 3.0, theme::WIDGET_HOVER);
    }
    let color = if pinned || resp.hovered() {
        theme::TEXT_STRONG
    } else {
        theme::TEXT_SUBDUED
    };
    let icon = if pinned && resp.hovered() {
        ICON_REFRESH_OFF
    } else {
        ICON_REFRESH
    };
    ui.painter().text(
        icon_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::TextStyle::Body.resolve(ui.style()),
        color,
    );
    resp.clicked()
}

fn pinned_item_ui(ui: &mut egui::Ui, name: &str, value: Option<f64>) -> (egui::Response, bool) {
    let desired = egui::vec2(ui.available_width(), 24.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
    let mut unpin = false;
    if ui.is_rect_visible(rect) {
        let bg = if ui.rect_contains_pointer(rect) {
            theme::WIDGET_HOVER
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 4.0, bg);

        let font_mono = egui::TextStyle::Monospace.resolve(ui.style());
        ui.painter().text(
            rect.left_center() + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            name,
            font_mono.clone(),
            theme::TEXT_DEFAULT,
        );
        ui.painter().text(
            rect.right_center() - egui::vec2(34.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            value
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "-".to_owned()),
            font_mono,
            theme::TEXT_DEFAULT,
        );
        unpin = pin_button(ui, rect, name, true);
    }
    (resp, unpin)
}

fn map_item_ui(
    ui: &mut egui::Ui,
    name: &str,
    type_label: &str,
    pinned: bool,
) -> (egui::Response, bool) {
    let desired = egui::vec2(ui.available_width(), 24.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
    let mut pin_clicked = false;
    if ui.is_rect_visible(rect) {
        let bg = if ui.rect_contains_pointer(rect) {
            theme::WIDGET_HOVER
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 4.0, bg);

        let font_mono = egui::TextStyle::Monospace.resolve(ui.style());
        ui.painter().text(
            rect.left_center() + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            name,
            font_mono.clone(),
            theme::TEXT_DEFAULT,
        );
        ui.painter().text(
            rect.right_center() - egui::vec2(42.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            type_label,
            font_mono,
            theme::TEXT_SUBDUED,
        );
        pin_clicked = pin_button(ui, rect, name, pinned);
    }
    (resp, pin_clicked)
}

pub fn show_variable_map(
    ui: &mut egui::Ui,
    inspector: &mut InspectorState,
    filter_text: &mut String,
    split_frac: &mut f32,
) -> bool {
    theme::section_header(ui, "Variable Map");

    if inspector.descriptors.is_empty() {
        ui.add_space(4.0);
        ui.weak("  No descriptors enumerated");
        return false;
    }
    ui.add_space(4.0);

    let mut changed = false;
    let splitter_h = 8.0;
    let min_h = 48.0;
    let avail = (ui.available_height() - splitter_h).max(min_h * 2.0);
    let top_h = (avail * *split_frac).clamp(min_h, avail - min_h);

    if inspector.pinned.is_empty() {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), top_h),
            egui::Sense::hover(),
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Pin variables to watch",
            egui::TextStyle::Body.resolve(ui.style()),
            theme::TEXT_SUBDUED,
        );
    } else {
        egui::ScrollArea::vertical()
            .id_salt("varmap_watch_scroll")
            .auto_shrink([false, false])
            .max_height(top_h)
            .show(ui, |ui| {
                ui.push_id("varmap_watch", |ui| {
                    let mut to_unpin = None;
                    for (pos, &idx) in inspector.pinned.iter().enumerate() {
                        let Some(descriptor) = inspector.descriptors.get(idx) else {
                            continue;
                        };
                        let value = inspector.values.get(idx).copied().flatten();
                        let (resp, unpin) = pinned_item_ui(ui, &descriptor.name, value);
                        resp.dnd_set_drag_payload(VarDragPayload {
                            names: vec![descriptor.name.clone()],
                            source: DragSource::Copy,
                        });
                        if unpin {
                            to_unpin = Some(pos);
                        }
                    }
                    if let Some(pos) = to_unpin {
                        inspector.pinned.remove(pos);
                        changed = true;
                    }
                });
            });
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
    let mut pin_toggles = Vec::new();

    egui::ScrollArea::vertical()
        .id_salt("sources_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !system_entries.is_empty() {
                egui::CollapsingHeader::new("System Variables")
                    .default_open(false)
                    .show(ui, |ui| {
                        for entry in &system_entries {
                            show_map_entry(ui, entry, &filter, inspector, &mut pin_toggles);
                        }
                    });
                ui.add_space(4.0);
                ui.separator();
            }

            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.strong("All Variables:");
            });
            if entries.is_empty() {
                ui.weak("No user variables enumerated");
            } else {
                for entry in &entries {
                    show_map_entry(ui, entry, &filter, inspector, &mut pin_toggles);
                }
            }
        });

    for idx in pin_toggles {
        if let Some(pos) = inspector.pinned.iter().position(|&p| p == idx) {
            inspector.pinned.remove(pos);
        } else {
            inspector.pinned.push(idx);
        }
        changed = true;
    }

    changed
}

fn show_map_entry(
    ui: &mut egui::Ui,
    entry: &DescriptorEntry,
    filter: &str,
    inspector: &InspectorState,
    pin_toggles: &mut Vec<usize>,
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
            let Some(descriptor) = inspector.descriptors.get(*index) else {
                return;
            };
            let is_pinned = inspector.pinned.contains(index);
            let (resp, pin_clicked) = map_item_ui(ui, label, descriptor.var.ty.label(), is_pinned);
            resp.dnd_set_drag_payload(VarDragPayload {
                names: vec![full_name.clone()],
                source: DragSource::Copy,
            });
            if pin_clicked {
                pin_toggles.push(*index);
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

            let all_pinned = !leaf_indexes.is_empty()
                && leaf_indexes.iter().all(|i| inspector.pinned.contains(i));
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
                let mut pin_clicked = false;
                if ui.is_rect_visible(rect) {
                    let bg = if ui.rect_contains_pointer(rect) {
                        theme::WIDGET_HOVER
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, 4.0, bg);
                    ui.painter().text(
                        rect.left_center() + egui::vec2(2.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::TextStyle::Monospace.resolve(ui.style()),
                        theme::TEXT_DEFAULT,
                    );
                    pin_clicked = pin_button(ui, rect, full_name, all_pinned);
                }
                resp.dnd_set_drag_payload(VarDragPayload {
                    names: leaf_names,
                    source: DragSource::Copy,
                });
                pin_clicked
            });
            if header_out.inner {
                if all_pinned {
                    pin_toggles.extend(leaf_indexes.iter().copied());
                } else {
                    pin_toggles.extend(
                        leaf_indexes
                            .iter()
                            .copied()
                            .filter(|index| !inspector.pinned.contains(index)),
                    );
                }
            }
            state.show_body_indented(&header_out.response, ui, |ui| {
                for member in members {
                    let child_filter =
                        if !filter.is_empty() && full_name.to_lowercase().contains(filter) {
                            ""
                        } else {
                            filter
                        };
                    show_map_entry(ui, member, child_filter, inspector, pin_toggles);
                }
            });
        }
    }
}

pub struct VariablesPanelOutput {
    pub to_write: Vec<(usize, f64)>,
    pub watch_changed: bool,
}

pub fn show_variables(
    ui: &mut egui::Ui,
    inspector: &mut InspectorState,
    drop_hover_panel: &mut bool,
) -> VariablesPanelOutput {
    theme::section_header(ui, "Variable Controller");
    ui.add_space(4.0);

    let mut watch_changed = false;
    let has_var_payload = egui::DragAndDrop::has_payload_of_type::<VarDragPayload>(ui.ctx());
    let dragged_row_idx = egui::DragAndDrop::payload::<VarDragPayload>(ui.ctx()).and_then(|p| {
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

    let arrow_w = 24.0;
    let weighted_w = (ui.available_width() - arrow_w - ui.spacing().item_spacing.x * 3.0).max(0.0);
    let unit_w = weighted_w / 10.0;
    let name_w = unit_w * 4.0;
    let value_w = unit_w * 3.0;
    let write_w = unit_w * 3.0;

    let table = TableBuilder::new(ui)
        .id_salt("var_ctrl_table")
        .striped(true)
        .sense(egui::Sense::click_and_drag())
        .max_scroll_height(192.0)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(name_w))
        .column(Column::exact(value_w))
        .column(Column::exact(arrow_w))
        .column(Column::exact(write_w));

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
                ui.strong("Write");
            });
        })
        .body(|body| {
            body.rows(row_h, num_rows, |mut row| {
                let i = row.index();
                let watch = &mut inspector.watch_vars[i];
                let value = inspector
                    .values
                    .get(watch.descriptor_index)
                    .copied()
                    .flatten();

                let being_dragged = dragged_row_idx == Some(i);
                let row_alpha = if being_dragged { 0.4 } else { 1.0 };

                let name_resp = row
                    .col(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&watch.var_name)
                                    .monospace()
                                    .color(theme::TEXT_DEFAULT.gamma_multiply(row_alpha)),
                            )
                            .truncate()
                            .selectable(false),
                        );
                    })
                    .1;
                let value_resp = row
                    .col(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    value
                                        .map(|value| format!("{value:.4}"))
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
                    let height = ui.spacing().interact_size.y;
                    let desired = egui::vec2(ui.available_width(), height);
                    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 3.0, theme::WIDGET_HOVER);
                        }
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            ICON_ARROW_LEFT,
                            egui::TextStyle::Body.resolve(ui.style()),
                            if resp.hovered() {
                                theme::TEXT_STRONG
                            } else {
                                theme::TEXT_DEFAULT
                            },
                        );
                    }
                    if resp.clicked()
                        && let Ok(value) = watch.write_buf.parse::<f64>()
                    {
                        to_write.push((watch.descriptor_index, value));
                    }
                });
                let write_resp = row
                    .col(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut watch.write_buf)
                                .desired_width(ui.available_width())
                                .hint_text("value"),
                        );
                    })
                    .1;

                if name_resp.drag_started() {
                    egui::DragAndDrop::set_payload(
                        &name_resp.ctx,
                        VarDragPayload {
                            names: vec![watch.var_name.clone()],
                            source: DragSource::Copy,
                        },
                    );
                }

                let row_resp = row.response();
                let hover_payload = row_resp.dnd_hover_payload::<VarDragPayload>().filter(|p| {
                    let is_self = p.names.len() == 1
                        && inspector
                            .watch_vars
                            .get(i)
                            .is_some_and(|watch| watch.var_name == p.names[0]);
                    !is_self
                });
                if hover_payload.is_some() {
                    drop_hover = true;
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
                    if ui.button("Remove").clicked() {
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

    let section_rect = egui::Rect::from_x_y_ranges(
        ui.min_rect().x_range(),
        egui::Rangef::new(section_top, ui.cursor().top()),
    );
    if has_var_payload && ui.rect_contains_pointer(section_rect) {
        drop_hover = true;
        ui.painter().hline(
            section_rect.x_range(),
            section_rect.bottom(),
            egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE),
        );

        if ui.ctx().input(|input| input.pointer.any_released())
            && let Some(payload) = egui::DragAndDrop::take_payload::<VarDragPayload>(ui.ctx())
        {
            insert_at = Some((inspector.watch_vars.len(), payload.names.clone()));
        }
    }

    if let Some((from, to)) = reorder
        && from < inspector.watch_vars.len()
        && from != to
    {
        let item = inspector.watch_vars.remove(from);
        let insert_pos = to.min(inspector.watch_vars.len());
        inspector.watch_vars.insert(insert_pos, item);
        watch_changed = true;
    }

    if let Some((idx, names)) = insert_at {
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
        true,
        ui.available_width(),
    ) {
        for watch in &inspector.watch_vars {
            if let Ok(value) = watch.write_buf.parse::<f64>() {
                to_write.push((watch.descriptor_index, value));
            }
        }
    }
    if let Some(i) = to_remove.get() {
        inspector.watch_vars.remove(i);
        watch_changed = true;
    }

    *drop_hover_panel |= drop_hover;

    VariablesPanelOutput {
        to_write,
        watch_changed,
    }
}
