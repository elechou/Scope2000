use eframe::egui;

use crate::theme::{self, GLYPH_EYE, GLYPH_EYE_SLASH, GLYPH_MINUS};
use crate::variable::InspectorState;
use crate::wave::dnd::{self, DragSource, VarDragPayload};
use crate::wave::pane::{PaneKind, ViewPane};
use crate::wave::selection::Selection;

use super::ViewportPanelState;

// ---------------------------------------------------------------------------
// Blueprint panel
// ---------------------------------------------------------------------------

/// Blueprint view info -- just the panes, no container noise.
struct ViewInfo {
    id: egui_tiles::TileId,
    name: String,
    pane_kind: PaneKind,
    /// (var_name, visible, color)
    series: Vec<(String, bool, egui::Color32)>,
}

enum BlueprintAction {
    SelectPane(egui_tiles::TileId),
    SelectSeries(egui_tiles::TileId, usize),
    RemovePane(egui_tiles::TileId),
    AddPane(PaneKind),
    HoverPane(egui_tiles::TileId),
    AddVarsToPane(egui_tiles::TileId, Vec<String>),
    InsertVarsInPane(egui_tiles::TileId, usize, Vec<String>),
    RemoveSeries(egui_tiles::TileId, usize),
    ToggleSeriesVisible(egui_tiles::TileId, usize),
    SetSeriesColor(egui_tiles::TileId, usize, egui::Color32),
    ReorderSeries(egui_tiles::TileId, usize, usize),
    /// Move a series across panes.
    MoveSeriesAcross {
        from_tile: egui_tiles::TileId,
        from_idx: usize,
        to_tile: egui_tiles::TileId,
        to_idx: usize,
    },
    ReorderPane(usize, usize),
}

/// DnD payload for reordering panes in the Wave Layout list.
#[derive(Clone, Copy)]
struct PaneDragPayload {
    from: usize,
}

/// Sync `blueprint_order` with the current set of pane tiles.
fn sync_blueprint_order(
    tree: &egui_tiles::Tree<ViewPane>,
    blueprint_order: &mut Vec<egui_tiles::TileId>,
) {
    use std::collections::HashSet;
    let mut existing: HashSet<egui_tiles::TileId> = HashSet::new();
    for id in tree.tiles.tile_ids() {
        if let Some(egui_tiles::Tile::Pane(_)) = tree.tiles.get(id) {
            existing.insert(id);
        }
    }
    blueprint_order.retain(|id| existing.remove(id));
    let mut new_ids: Vec<_> = existing.into_iter().collect();
    new_ids.sort_by_key(|id| format!("{id:?}"));
    blueprint_order.extend(new_ids);
}

/// Collect all panes from the tile tree in `blueprint_order`.
fn collect_views(
    tree: &egui_tiles::Tree<ViewPane>,
    blueprint_order: &[egui_tiles::TileId],
) -> Vec<ViewInfo> {
    let mut views = Vec::new();
    for &id in blueprint_order {
        if let Some(egui_tiles::Tile::Pane(p)) = tree.tiles.get(id) {
            views.push(ViewInfo {
                id,
                name: p.name.clone(),
                pane_kind: p.kind,
                series: p
                    .series
                    .iter()
                    .map(|s| (s.var_name.clone(), s.visible, s.color))
                    .collect(),
            });
        }
    }
    views
}

/// Render the blueprint panel. Returns `Some(kind)` if the user clicked
/// "+ Time Series" or "+ Dataframe" and the caller should create a new pane.
/// Returns (add_pane_request, drop_feedback).
pub fn show_blueprint(
    ui: &mut egui::Ui,
    vp: &mut ViewportPanelState<'_>,
    inspector: &InspectorState,
    can_edit_variable_refs: bool,
) -> (Option<PaneKind>, Option<dnd::DropFeedback>) {
    let mut wants_add_pane: Option<PaneKind> = None;
    let mut blueprint_feedback: Option<dnd::DropFeedback> = None;

    theme::section_header(ui, "Wave Layout");

    *vp.hovered_tile = None;
    *vp.hovered_blueprint_var = None;
    sync_blueprint_order(vp.tree, vp.blueprint_order);
    let views = collect_views(vp.tree, vp.blueprint_order);
    let mut actions = Vec::new();

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let w = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if theme::icon_button(ui, theme::ICON_TIMESERIES, "+ Time Series", w).clicked() {
            actions.push(BlueprintAction::AddPane(PaneKind::TimeSeries));
        }
        if theme::icon_button(ui, theme::ICON_DATAFRAME, "+ Dataframe", w).clicked() {
            actions.push(BlueprintAction::AddPane(PaneKind::Dataframe));
        }
    });
    ui.add_space(4.0);

    let drag_in_flight = egui::DragAndDrop::has_any_payload(ui.ctx());

    egui::ScrollArea::vertical()
        .id_salt("blueprint_scroll")
        .show(ui, |ui| {
            for (view_idx, v) in views.iter().enumerate() {
                let is_sel = matches!(vp.selection, Selection::Pane(s) if *s == v.id);
                let title = v.name.clone();
                let id = v.id;
                let series = &v.series;

                // Custom header bar
                let desired = egui::vec2(ui.available_width(), 24.0);
                let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());

                let collapse_id = ui.make_persistent_id(("bp_open", id));
                let mut open = ui.data(|d| d.get_temp::<bool>(collapse_id).unwrap_or(true));

                if resp.double_clicked() {
                    open = !open;
                    ui.data_mut(|d| d.insert_temp(collapse_id, open));
                }
                if resp.clicked() {
                    actions.push(BlueprintAction::SelectPane(id));
                }
                if resp.hovered() && !drag_in_flight {
                    actions.push(BlueprintAction::HoverPane(id));
                }

                // Drag source: pane reorder.
                if resp.drag_started() {
                    egui::DragAndDrop::set_payload(ui.ctx(), PaneDragPayload { from: view_idx });
                }
                let being_dragged_pane = egui::DragAndDrop::payload::<PaneDragPayload>(ui.ctx())
                    .is_some_and(|p| p.from == view_idx);

                // Drop target for pane reorder.
                let pane_hover_payload = resp
                    .dnd_hover_payload::<PaneDragPayload>()
                    .filter(|p| p.from != view_idx);
                if pane_hover_payload.is_some()
                    && let Some(payload) = resp.dnd_release_payload::<PaneDragPayload>()
                {
                    let interact_y = ui
                        .input(|i| i.pointer.interact_pos())
                        .map(|p| p.y)
                        .unwrap_or(rect.center().y);
                    let insert_above = interact_y < rect.center().y;
                    let mut to = if insert_above { view_idx } else { view_idx + 1 };
                    if payload.from < to {
                        to -= 1;
                    }
                    if to != payload.from {
                        actions.push(BlueprintAction::ReorderPane(payload.from, to));
                    }
                }

                // Drop target: accept VarDragPayload
                let var_hover_on_header = resp.dnd_hover_payload::<VarDragPayload>().filter(|p| {
                    !matches!(
                        p.source,
                        DragSource::MoveFromPane { tile_id, .. }
                            if tile_id == id
                    )
                });
                let has_drop_hover = var_hover_on_header.is_some();
                if has_drop_hover {
                    *vp.drop_hover_panel = true;
                    if let Some(payload) = resp.dnd_release_payload::<VarDragPayload>() {
                        let mode = dnd::effective_drag_mode(ui.ctx(), &payload.source);
                        match (&payload.source, mode) {
                            (_, dnd::DragMode::Copy) => {
                                actions.push(BlueprintAction::AddVarsToPane(
                                    id,
                                    payload.names.clone(),
                                ));
                            }
                            (
                                DragSource::MoveFromPane {
                                    tile_id: from_tile,
                                    index: from_idx,
                                },
                                dnd::DragMode::Move,
                            ) => {
                                if *from_tile != id {
                                    let to_idx = vp
                                        .tree
                                        .tiles
                                        .get(id)
                                        .and_then(|t| {
                                            if let egui_tiles::Tile::Pane(p) = t {
                                                Some(p.series.len())
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or(0);
                                    actions.push(BlueprintAction::MoveSeriesAcross {
                                        from_tile: *from_tile,
                                        from_idx: *from_idx,
                                        to_tile: id,
                                        to_idx,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if ui.is_rect_visible(rect) {
                    let row_alpha: f32 = if being_dragged_pane { 0.4 } else { 1.0 };
                    let pane_drop_active = pane_hover_payload.is_some();
                    let bg = if has_drop_hover || pane_drop_active {
                        theme::DROP_TARGET_STROKE.gamma_multiply(0.3)
                    } else if is_sel {
                        theme::SELECT_BG
                    } else if resp.hovered() && !drag_in_flight {
                        theme::WIDGET_HOVER
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    ui.painter()
                        .rect_filled(rect, 4.0, bg.gamma_multiply(row_alpha));

                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let text_color = if is_sel {
                        theme::TEXT_STRONG
                    } else {
                        theme::TEXT_DEFAULT
                    };
                    let text_color = text_color.gamma_multiply(row_alpha);
                    let icon_pos = egui::pos2(
                        rect.left() + 6.0,
                        rect.center().y - theme::ICON_SIZE.y / 2.0,
                    );
                    let icon_rect = egui::Rect::from_min_size(icon_pos, theme::ICON_SIZE);
                    ui.put(
                        icon_rect,
                        egui::Image::new(v.pane_kind.icon_uri())
                            .fit_to_exact_size(theme::ICON_SIZE)
                            .tint(text_color),
                    );
                    ui.painter().text(
                        rect.left_center() + egui::vec2(6.0 + theme::ICON_SIZE.x + 4.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        &title,
                        font,
                        text_color,
                    );

                    if pane_hover_payload.is_some() {
                        let pointer_y = ui
                            .input(|i| i.pointer.hover_pos())
                            .map(|p| p.y)
                            .unwrap_or(rect.center().y);
                        let insert_above = pointer_y < rect.center().y;
                        let line_y = if insert_above {
                            rect.top()
                        } else {
                            rect.bottom()
                        };
                        ui.painter().hline(
                            rect.x_range(),
                            line_y,
                            egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE),
                        );
                    }
                }

                resp.context_menu(|ui| {
                    if ui.button("Remove").clicked() {
                        actions.push(BlueprintAction::RemovePane(id));
                        ui.close();
                    }
                });

                if open {
                    ui.indent(collapse_id, |ui| {
                        for (idx, (var, visible, color)) in series.iter().enumerate() {
                            let var_id = egui::Id::new(var.as_str());
                            let visible = *visible;
                            let color = *color;
                            let show_dot = v.pane_kind == PaneKind::TimeSeries;

                            let is_highlighted = matches!(
                                vp.hovered_plot_var,
                                Some((tid, vid)) if *tid == id && *vid == var_id
                            );

                            let is_series_selected = matches!(
                                vp.selection,
                                Selection::Series(tid, sidx) if *tid == id && *sidx == idx
                            );

                            let desired = egui::vec2(ui.available_width(), 20.0);
                            let (rect, resp) =
                                ui.allocate_exact_size(desired, egui::Sense::click_and_drag());

                            let swatch_rect = egui::Rect::from_center_size(
                                egui::pos2(rect.left() + 13.0, rect.center().y),
                                egui::Vec2::splat(theme::COLOR_SWATCH_SIZE),
                            );
                            let swatch_resp = if show_dot {
                                Some(ui.interact(
                                    swatch_rect,
                                    resp.id.with("swatch"),
                                    egui::Sense::click(),
                                ))
                            } else {
                                None
                            };

                            let btn_size = egui::vec2(20.0, rect.height());
                            let eye_rect = egui::Rect::from_min_size(
                                egui::pos2(rect.right() - 4.0 - btn_size.x, rect.top()),
                                btn_size,
                            );
                            let minus_rect = egui::Rect::from_min_size(
                                egui::pos2(eye_rect.left() - btn_size.x, rect.top()),
                                btn_size,
                            );

                            let pointer_pos = ui.input(|i| i.pointer.hover_pos());
                            let over_swatch =
                                show_dot && pointer_pos.is_some_and(|p| swatch_rect.contains(p));
                            let over_eye = pointer_pos.is_some_and(|p| eye_rect.contains(p));
                            let over_minus = pointer_pos.is_some_and(|p| minus_rect.contains(p));
                            let over_button = over_eye || over_minus || over_swatch;

                            if resp.drag_started() && !over_button {
                                egui::DragAndDrop::set_payload(
                                    ui.ctx(),
                                    VarDragPayload {
                                        names: vec![var.clone()],
                                        source: DragSource::MoveFromPane {
                                            tile_id: id,
                                            index: idx,
                                        },
                                    },
                                );
                            }

                            let being_dragged = egui::DragAndDrop::payload::<VarDragPayload>(
                                ui.ctx(),
                            )
                            .is_some_and(|p| match p.source {
                                DragSource::MoveFromPane { tile_id, index } => {
                                    tile_id == id && index == idx
                                }
                                _ => false,
                            });

                            let var_hover_payload =
                                resp.dnd_hover_payload::<VarDragPayload>().filter(|p| {
                                    !matches!(
                                        p.source,
                                        DragSource::MoveFromPane { tile_id, index }
                                            if tile_id == id && index == idx
                                    )
                                });
                            if var_hover_payload.is_some() {
                                *vp.drop_hover_panel = true;
                                if let Some(payload) = resp.dnd_release_payload::<VarDragPayload>()
                                {
                                    let interact_y = ui
                                        .input(|i| i.pointer.interact_pos())
                                        .map(|p| p.y)
                                        .unwrap_or(rect.center().y);
                                    let insert_above = interact_y < rect.center().y;
                                    let insert_at = if insert_above { idx } else { idx + 1 };
                                    let mode = dnd::effective_drag_mode(ui.ctx(), &payload.source);
                                    match (&payload.source, mode) {
                                        (_, dnd::DragMode::Copy) => {
                                            // Copy mode (Source panel, or Ctrl+Move)
                                            actions.push(BlueprintAction::InsertVarsInPane(
                                                id,
                                                insert_at,
                                                payload.names.clone(),
                                            ));
                                        }
                                        (
                                            DragSource::MoveFromPane {
                                                tile_id: from_tile,
                                                index: from_idx,
                                            },
                                            dnd::DragMode::Move,
                                        ) => {
                                            if *from_tile == id {
                                                let mut to = insert_at;
                                                if *from_idx < to {
                                                    to -= 1;
                                                }
                                                if to != *from_idx {
                                                    actions.push(BlueprintAction::ReorderSeries(
                                                        id, *from_idx, to,
                                                    ));
                                                }
                                            } else {
                                                actions.push(BlueprintAction::MoveSeriesAcross {
                                                    from_tile: *from_tile,
                                                    from_idx: *from_idx,
                                                    to_tile: id,
                                                    to_idx: insert_at,
                                                });
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            let series_drop_active = var_hover_payload.is_some();
                            let can_hover_link = resp.hovered()
                                && !being_dragged
                                && (!drag_in_flight || series_drop_active);
                            if can_hover_link {
                                *vp.hovered_blueprint_var = Some((id, var_id));
                            }

                            if resp.clicked() {
                                if over_swatch {
                                    // handled by swatch's own popup
                                } else if over_minus {
                                    actions.push(BlueprintAction::RemoveSeries(id, idx));
                                } else if over_eye {
                                    actions.push(BlueprintAction::ToggleSeriesVisible(id, idx));
                                } else {
                                    actions.push(BlueprintAction::SelectSeries(id, idx));
                                }
                            }

                            if let Some(sr) = swatch_resp.as_ref() {
                                let mut edited = color;
                                if theme::color_swatch_popup(sr, &mut edited) {
                                    actions.push(BlueprintAction::SetSeriesColor(id, idx, edited));
                                }
                            }

                            if ui.is_rect_visible(rect) {
                                let hover_for_bg =
                                    resp.hovered() && (!drag_in_flight || series_drop_active);
                                let active = is_highlighted || hover_for_bg || is_series_selected;

                                let row_alpha: f32 = if being_dragged { 0.4 } else { 1.0 };

                                if active {
                                    let bg = if is_series_selected {
                                        theme::SELECT_BG
                                    } else {
                                        theme::WIDGET_HOVER
                                    };
                                    ui.painter().rect_filled(
                                        rect,
                                        4.0,
                                        bg.gamma_multiply(row_alpha),
                                    );
                                }
                                let font = egui::TextStyle::Monospace.resolve(ui.style());
                                let is_system_variable = inspector.is_system_variable_name(var);
                                let text_color = if active {
                                    theme::TEXT_STRONG
                                } else if !visible {
                                    theme::TEXT_SUBDUED
                                } else {
                                    theme::TEXT_DEFAULT
                                };
                                let text_color = text_color.gamma_multiply(row_alpha);
                                let mut text_x = if show_dot {
                                    let swatch_color = if visible {
                                        color.gamma_multiply(row_alpha)
                                    } else {
                                        theme::TEXT_SUBDUED.gamma_multiply(row_alpha)
                                    };
                                    if let Some(sr) = swatch_resp.as_ref() {
                                        theme::color_swatch_at(ui, swatch_rect, sr, swatch_color);
                                    }
                                    24.0
                                } else {
                                    6.0
                                };
                                if is_system_variable {
                                    let badge_alpha = row_alpha * if visible { 1.0 } else { 0.6 };
                                    text_x += theme::paint_system_variable_badge(
                                        ui,
                                        rect.left_center() + egui::vec2(text_x, 0.0),
                                        badge_alpha,
                                    );
                                }
                                ui.painter().text(
                                    rect.left_center() + egui::vec2(text_x, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    var,
                                    font.clone(),
                                    text_color,
                                );

                                let drag_in_flight = egui::DragAndDrop::has_any_payload(ui.ctx());
                                if resp.hovered() && !drag_in_flight {
                                    let btn_font = egui::TextStyle::Body.resolve(ui.style());

                                    let minus_bg = if over_minus {
                                        theme::WIDGET_HOVER
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    ui.painter().rect_filled(minus_rect, 3.0, minus_bg);
                                    ui.painter().text(
                                        minus_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        GLYPH_MINUS,
                                        btn_font.clone(),
                                        if over_minus {
                                            theme::TEXT_STRONG
                                        } else {
                                            theme::TEXT_SUBDUED
                                        },
                                    );

                                    let eye_bg = if over_eye {
                                        theme::WIDGET_HOVER
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    ui.painter().rect_filled(eye_rect, 3.0, eye_bg);
                                    let eye_glyph =
                                        if visible { GLYPH_EYE } else { GLYPH_EYE_SLASH };
                                    ui.painter().text(
                                        eye_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        eye_glyph,
                                        btn_font,
                                        if over_eye {
                                            theme::TEXT_STRONG
                                        } else if visible {
                                            theme::TEXT_DEFAULT
                                        } else {
                                            theme::TEXT_SUBDUED
                                        },
                                    );
                                }

                                if var_hover_payload.is_some() {
                                    let pointer_y =
                                        pointer_pos.map(|p| p.y).unwrap_or(rect.center().y);
                                    let insert_above = pointer_y < rect.center().y;
                                    let line_y = if insert_above {
                                        rect.top()
                                    } else {
                                        rect.bottom()
                                    };
                                    ui.painter().hline(
                                        rect.x_range(),
                                        line_y,
                                        egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE),
                                    );
                                }
                            }
                        }
                        if series.is_empty() {
                            ui.weak("(drag variables here)");
                        }
                    });
                }
            }
        });

    for action in actions {
        match action {
            BlueprintAction::SelectPane(id) => *vp.selection = Selection::Pane(id),
            BlueprintAction::SelectSeries(tile_id, idx) => {
                *vp.selection = Selection::Series(tile_id, idx);
            }
            BlueprintAction::RemovePane(id) => {
                if matches!(vp.selection, Selection::Pane(sid) if *sid == id)
                    || matches!(vp.selection, Selection::Series(sid, _) if *sid == id)
                {
                    *vp.selection = Selection::None;
                }
                if let Some(pid) = vp.tree.tiles.parent_of(id)
                    && let Some(egui_tiles::Tile::Container(c)) = vp.tree.tiles.get_mut(pid)
                {
                    c.remove_child(id);
                }
                vp.tree.tiles.remove(id);
                vp.blueprint_order.retain(|tid| *tid != id);
            }
            BlueprintAction::AddPane(kind) => {
                wants_add_pane = Some(kind);
            }
            BlueprintAction::HoverPane(id) => *vp.hovered_tile = Some(id),
            BlueprintAction::AddVarsToPane(id, names) => {
                if !can_edit_variable_refs {
                    continue;
                }
                if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(id) {
                    for name in &names {
                        if p.series.iter().any(|s| &s.var_name == name) {
                            blueprint_feedback = Some(dnd::DropFeedback::Duplicate {
                                var_name: name.clone(),
                                pane_name: p.name.clone(),
                            });
                        } else {
                            let idx = p.series.len();
                            p.add_series(name.clone(), crate::wave::pane::default_color(idx));
                        }
                    }
                }
            }
            BlueprintAction::InsertVarsInPane(id, insert_at, names) => {
                if !can_edit_variable_refs {
                    continue;
                }
                if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(id) {
                    let mut cursor = insert_at.min(p.series.len());
                    for name in &names {
                        if p.series.iter().any(|s| &s.var_name == name) {
                            blueprint_feedback = Some(dnd::DropFeedback::Duplicate {
                                var_name: name.clone(),
                                pane_name: p.name.clone(),
                            });
                        } else {
                            let color_idx = p.series.len();
                            p.series.insert(
                                cursor,
                                crate::wave::pane::SeriesConfig::new(
                                    name.clone(),
                                    crate::wave::pane::default_color(color_idx),
                                ),
                            );
                            cursor += 1;
                        }
                    }
                }
            }
            BlueprintAction::RemoveSeries(tile_id, idx) => {
                if !can_edit_variable_refs {
                    continue;
                }
                if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(tile_id)
                    && idx < p.series.len()
                {
                    p.series.remove(idx);
                    *vp.selection = match &*vp.selection {
                        Selection::Series(tid, sidx) if *tid == tile_id => {
                            if *sidx == idx {
                                Selection::Pane(tile_id)
                            } else if *sidx > idx {
                                Selection::Series(tile_id, *sidx - 1)
                            } else {
                                vp.selection.clone()
                            }
                        }
                        _ => vp.selection.clone(),
                    };
                }
            }
            BlueprintAction::ToggleSeriesVisible(tile_id, idx) => {
                if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(tile_id)
                    && let Some(s) = p.series.get_mut(idx)
                {
                    s.visible = !s.visible;
                }
            }
            BlueprintAction::SetSeriesColor(tile_id, idx, color) => {
                if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(tile_id)
                    && let Some(s) = p.series.get_mut(idx)
                {
                    s.color = color;
                }
            }
            BlueprintAction::MoveSeriesAcross {
                from_tile,
                from_idx,
                to_tile,
                to_idx,
            } => {
                if from_tile == to_tile {
                    continue;
                }
                // Get the variable name from source to check for duplicates FIRST
                let var_name = vp.tree.tiles.get(from_tile).and_then(|t| match t {
                    egui_tiles::Tile::Pane(p) => p.series.get(from_idx).map(|s| s.var_name.clone()),
                    _ => None,
                });
                let Some(var_name) = var_name else { continue };

                // Check destination for duplicate BEFORE removing from source
                let is_dup = vp.tree.tiles.get(to_tile).is_some_and(|t| match t {
                    egui_tiles::Tile::Pane(p) => p.series.iter().any(|s| s.var_name == var_name),
                    _ => false,
                });
                if is_dup {
                    let pane_name = vp
                        .tree
                        .tiles
                        .get(to_tile)
                        .map(|t| match t {
                            egui_tiles::Tile::Pane(p) => p.name.clone(),
                            _ => String::new(),
                        })
                        .unwrap_or_default();
                    blueprint_feedback = Some(dnd::DropFeedback::Duplicate {
                        var_name,
                        pane_name,
                    });
                    continue; // Don't remove from source — kick back
                }

                // Safe to move: remove from source, insert into destination
                let moved =
                    if let Some(egui_tiles::Tile::Pane(src)) = vp.tree.tiles.get_mut(from_tile) {
                        if from_idx < src.series.len() {
                            Some(src.series.remove(from_idx))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                if let Some(series_cfg) = moved {
                    if let Selection::Series(tid, sidx) = &*vp.selection {
                        if *tid == from_tile && *sidx == from_idx {
                            *vp.selection = Selection::None;
                        } else if *tid == from_tile && *sidx > from_idx {
                            *vp.selection = Selection::Series(*tid, *sidx - 1);
                        }
                    }
                    if let Some(egui_tiles::Tile::Pane(dst)) = vp.tree.tiles.get_mut(to_tile) {
                        let at = to_idx.min(dst.series.len());
                        dst.series.insert(at, series_cfg);
                    }
                }
            }
            BlueprintAction::ReorderPane(from, to) => {
                if from < vp.blueprint_order.len() && from != to {
                    let id = vp.blueprint_order.remove(from);
                    let insert_at = to.min(vp.blueprint_order.len());
                    vp.blueprint_order.insert(insert_at, id);
                }
            }
            BlueprintAction::ReorderSeries(tile_id, from, to) => {
                if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(tile_id)
                    && from < p.series.len()
                    && to <= p.series.len()
                    && from != to
                {
                    let item = p.series.remove(from);
                    let insert_at = to.min(p.series.len());
                    p.series.insert(insert_at, item);
                    if let Selection::Series(tid, sidx) = &*vp.selection
                        && *tid == tile_id
                    {
                        let new_sel = if *sidx == from {
                            insert_at
                        } else if from < *sidx && *sidx <= insert_at {
                            *sidx - 1
                        } else if insert_at <= *sidx && *sidx < from {
                            *sidx + 1
                        } else {
                            *sidx
                        };
                        *vp.selection = Selection::Series(tile_id, new_sel);
                    }
                }
            }
        }
    }

    (wants_add_pane, blueprint_feedback)
}
