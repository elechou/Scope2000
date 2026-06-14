use eframe::egui;

use crate::theme;
use crate::wave::pane::{AxisRange, LegendCorner, PaneKind};
use crate::wave::selection::Selection;

use super::ViewportPanelState;

// ---------------------------------------------------------------------------
// Selection panel
// ---------------------------------------------------------------------------

const PROP_LABEL_WIDTH: f32 = 60.0;

/// Fixed-width, left-aligned property label for grid cells.
fn prop_label(ui: &mut egui::Ui, text: &str) {
    let height = ui.spacing().interact_size.y;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(PROP_LABEL_WIDTH, height), egui::Sense::hover());
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        theme::TEXT_SUBDUED,
    );
}

/// Section heading: separator + bold title (non-collapsible).
fn section_heading(ui: &mut egui::Ui, title: &str) {
    ui.separator();
    ui.label(
        egui::RichText::new(title)
            .color(theme::TEXT_DEFAULT)
            .strong(),
    );
}

pub fn show_selection_panel(ui: &mut egui::Ui, vp: &mut ViewportPanelState<'_>) {
    theme::section_header(ui, "Selection");
    ui.add_space(4.0);

    match &vp.selection.clone() {
        Selection::None => {
            ui.weak("Click a view or series to inspect");
        }
        Selection::Pane(tile_id) => {
            let tile_id = *tile_id;
            show_pane_selection(ui, vp, tile_id);
        }
        Selection::Series(tile_id, idx) => {
            let tile_id = *tile_id;
            let idx = *idx;
            show_series_selection(ui, vp, tile_id, idx);
        }
    }
}

fn show_pane_selection(
    ui: &mut egui::Ui,
    vp: &mut ViewportPanelState<'_>,
    tile_id: egui_tiles::TileId,
) {
    let info = vp.tree.tiles.get(tile_id).and_then(|t| {
        if let egui_tiles::Tile::Pane(p) = t {
            Some((p.kind, p.series.clone(), p.properties.clone()))
        } else {
            None
        }
    });
    let Some((kind, series, mut props)) = info else {
        ui.weak("View no longer exists");
        return;
    };

    ui.push_id(("pane_identity", tile_id), |ui| {
        egui::Grid::new("pane_identity_grid")
            .num_columns(2)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                prop_label(ui, "Name");
                if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(tile_id) {
                    ui.add(
                        egui::TextEdit::singleline(&mut p.name).desired_width(ui.available_width()),
                    );
                }
                ui.end_row();

                prop_label(ui, "View type");
                ui.strong(kind.label());
                ui.end_row();
            });
    });
    ui.add_space(2.0);

    let mut axis_dirty = false;
    if kind == PaneKind::TimeSeries {
        // ---- Plot background ----
        section_heading(ui, "Plot background");
        ui.add_space(1.0);
        egui::Grid::new("bg_grid")
            .num_columns(2)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                prop_label(ui, "Color");
                theme::color_swatch(ui, &mut props.background);
                ui.end_row();

                prop_label(ui, "Show grid");
                ui.checkbox(&mut props.show_grid, "");
                ui.end_row();
            });

        // ---- Plot legend ----
        section_heading(ui, "Plot legend");
        ui.add_space(1.0);
        egui::Grid::new("legend_grid")
            .num_columns(2)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                prop_label(ui, "Corner");
                egui::ComboBox::from_id_salt("legend_corner")
                    .selected_text(props.legend_corner.label())
                    .show_ui(ui, |ui| {
                        for &c in LegendCorner::ALL {
                            ui.selectable_value(&mut props.legend_corner, c, c.label());
                        }
                    });
                ui.end_row();

                prop_label(ui, "Visible");
                ui.checkbox(&mut props.legend_visible, "");
                ui.end_row();
            });

        // ---- Time axis ----
        section_heading(ui, "Time axis");
        ui.add_space(1.0);
        axis_range_row(
            ui,
            "time_axis",
            &mut props.time_axis_range,
            props.last_bounds_x,
            &mut axis_dirty,
        );

        // ---- Scalar axis ----
        section_heading(ui, "Scalar axis");
        ui.add_space(1.0);
        axis_range_row(
            ui,
            "scalar_axis",
            &mut props.scalar_axis_range,
            props.last_bounds_y,
            &mut axis_dirty,
        );

        if axis_dirty {
            props.axis_apply_pending = true;
        }

        // Write properties back
        if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(tile_id) {
            p.properties = props;
        }
    }

    // ---- Series list ----
    if kind == PaneKind::TimeSeries && !series.is_empty() {
        section_heading(ui, "Series");
        {
            let mut toggle = None;
            let mut remove = None;
            let mut select = None;
            let mut color_change: Option<(usize, egui::Color32)> = None;
            for (i, s) in series.iter().enumerate() {
                let is_selected = matches!(
                    &*vp.selection,
                    Selection::Series(tid, sidx) if *tid == tile_id && *sidx == i
                );

                let desired = egui::vec2(ui.available_width(), 20.0);
                let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());

                let swatch_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 13.0, rect.center().y),
                    egui::Vec2::splat(theme::COLOR_SWATCH_SIZE),
                );
                let swatch_id = resp.id.with(("swatch", i));
                let swatch_resp = ui.interact(swatch_rect, swatch_id, egui::Sense::click());

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
                let over_swatch = pointer_pos.is_some_and(|p| swatch_rect.contains(p));
                let over_eye = pointer_pos.is_some_and(|p| eye_rect.contains(p));
                let over_minus = pointer_pos.is_some_and(|p| minus_rect.contains(p));

                if resp.clicked() && !over_swatch && !over_minus && !over_eye {
                    select = Some(i);
                }
                if over_minus && resp.clicked() {
                    remove = Some(i);
                }
                if over_eye && resp.clicked() {
                    toggle = Some(i);
                }

                let mut edited = s.color;
                if theme::color_swatch_popup(&swatch_resp, &mut edited) {
                    color_change = Some((i, edited));
                }

                if ui.is_rect_visible(rect) {
                    let active = is_selected || resp.hovered();
                    if active {
                        let bg = if is_selected {
                            theme::SELECT_BG
                        } else {
                            theme::WIDGET_HOVER
                        };
                        ui.painter().rect_filled(rect, 4.0, bg);
                    }

                    theme::color_swatch_at(ui, swatch_rect, &swatch_resp, s.color);

                    let text_color = if is_selected {
                        theme::TEXT_STRONG
                    } else if !s.visible {
                        theme::TEXT_SUBDUED
                    } else {
                        theme::TEXT_DEFAULT
                    };
                    let font = egui::TextStyle::Monospace.resolve(ui.style());
                    ui.painter().text(
                        rect.left_center() + egui::vec2(24.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        s.label(),
                        font,
                        text_color,
                    );

                    if resp.hovered() {
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
                            theme::GLYPH_MINUS,
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
                        let eye_glyph = if s.visible {
                            theme::GLYPH_EYE
                        } else {
                            theme::GLYPH_EYE_SLASH
                        };
                        ui.painter().text(
                            eye_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            eye_glyph,
                            btn_font,
                            if over_eye {
                                theme::TEXT_STRONG
                            } else if s.visible {
                                theme::TEXT_DEFAULT
                            } else {
                                theme::TEXT_SUBDUED
                            },
                        );
                    }
                }
            }
            if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(tile_id) {
                if let Some(i) = toggle
                    && let Some(s) = p.series.get_mut(i)
                {
                    s.visible = !s.visible;
                }
                if let Some((i, c)) = color_change
                    && let Some(s) = p.series.get_mut(i)
                {
                    s.color = c;
                }
                if let Some(i) = remove
                    && i < p.series.len()
                {
                    p.series.remove(i);
                    if matches!(
                        &*vp.selection,
                        Selection::Series(tid, sidx) if *tid == tile_id && *sidx == i
                    ) {
                        *vp.selection = Selection::Pane(tile_id);
                    }
                }
            }
            if let Some(i) = select {
                *vp.selection = Selection::Series(tile_id, i);
            }
        }
    }
}

fn show_series_selection(
    ui: &mut egui::Ui,
    vp: &mut ViewportPanelState<'_>,
    tile_id: egui_tiles::TileId,
    idx: usize,
) {
    let info = vp.tree.tiles.get(tile_id).and_then(|t| {
        if let egui_tiles::Tile::Pane(p) = t {
            p.series.get(idx).cloned()
        } else {
            None
        }
    });
    let Some(mut sc) = info else {
        *vp.selection = Selection::Pane(tile_id);
        return;
    };

    ui.label(egui::RichText::new(sc.label()).strong());

    section_heading(ui, "Visualizer");
    ui.add_space(1.0);
    egui::Grid::new("series_props_grid")
        .num_columns(2)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            prop_label(ui, "Name");
            ui.monospace(&sc.var_name);
            ui.end_row();

            prop_label(ui, "Color");
            theme::color_swatch(ui, &mut sc.color);
            ui.end_row();

            prop_label(ui, "Width");
            ui.add(
                egui::DragValue::new(&mut sc.width)
                    .range(0.5..=5.0)
                    .speed(0.1)
                    .fixed_decimals(1),
            );
            ui.end_row();

            prop_label(ui, "Visible");
            ui.checkbox(&mut sc.visible, "");
            ui.end_row();
        });

    // Write changes back
    if let Some(egui_tiles::Tile::Pane(p)) = vp.tree.tiles.get_mut(tile_id)
        && let Some(s) = p.series.get_mut(idx)
    {
        *s = sc;
    }
}

/// Axis range row: Auto checkbox + inline range display/edit.
fn axis_range_row(
    ui: &mut egui::Ui,
    id_salt: &str,
    range: &mut AxisRange,
    last_bounds: (f64, f64),
    dirty: &mut bool,
) {
    let mut is_auto = matches!(range, AxisRange::Auto);
    let prev_auto = is_auto;

    egui::Grid::new(id_salt)
        .num_columns(2)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            prop_label(ui, "Auto");
            ui.checkbox(&mut is_auto, "");
            ui.end_row();

            if prev_auto && !is_auto {
                let (lo, hi) = last_bounds;
                *range = AxisRange::Manual {
                    min: (lo * 1000.0).round() / 1000.0,
                    max: (hi * 1000.0).round() / 1000.0,
                };
                *dirty = true;
            } else if !prev_auto && is_auto {
                *range = AxisRange::Auto;
                *dirty = true;
            }

            prop_label(ui, "Range");
            match range {
                AxisRange::Auto => {
                    ui.label(
                        egui::RichText::new(format!(
                            "{:.3}  -  {:.3}",
                            last_bounds.0, last_bounds.1
                        ))
                        .color(theme::TEXT_SUBDUED),
                    );
                }
                AxisRange::Manual { min, max } => {
                    ui.horizontal(|ui| {
                        let min_resp =
                            ui.add(egui::DragValue::new(min).speed(0.01).max_decimals(3));
                        ui.label("-");
                        let max_resp =
                            ui.add(egui::DragValue::new(max).speed(0.01).max_decimals(3));
                        if min_resp.changed() || max_resp.changed() {
                            *dirty = true;
                        }
                    });
                }
            }
            ui.end_row();
        });
}
