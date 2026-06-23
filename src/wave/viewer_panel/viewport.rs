use eframe::{egui, emath};

use crate::theme;
use crate::variable::InspectorState;
use crate::wave::data::PlotData;
use crate::wave::dnd;
use crate::wave::pane::PaneKind;
use crate::wave::selection::Selection;
use crate::wave::tiles::MyTilesDelegate;

use super::ViewportPanelState;

// ---------------------------------------------------------------------------
// Viewport panel
// ---------------------------------------------------------------------------

pub fn show_viewport(
    ui: &mut egui::Ui,
    vp: &mut ViewportPanelState<'_>,
    plot_data: &PlotData,
    inspector: &InspectorState,
    can_edit_variable_refs: bool,
) -> Option<dnd::DropFeedback> {
    let mut viewport_drop_feedback: Option<dnd::DropFeedback> = None;
    let var_names = inspector.var_names();
    let var_values = inspector.display_values();
    // Blueprint hover (same frame) takes priority over plot hover (prev frame)
    let highlight_var = vp.hovered_blueprint_var.or(*vp.hovered_plot_var);
    let time_axis_sync_group = time_axis_sync_group_id(vp);
    let (drop_hover_tile, pending_move, drop_feedback) = {
        let mut delegate = MyTilesDelegate {
            data: plot_data,
            selection: vp.selection,
            var_names: &var_names,
            var_values: &var_values,
            drop_hover_tile: None,
            highlight_var,
            hovered_plot_var: None,
            pending_cross_pane_move: None,
            drop_feedback: None,
            can_edit_variable_refs,
            time_axis_sync_group,
        };
        vp.tree.ui(&mut delegate, ui);
        *vp.hovered_plot_var = delegate.hovered_plot_var;
        (
            delegate.drop_hover_tile,
            delegate.pending_cross_pane_move,
            delegate.drop_feedback,
        )
    };

    // Apply cross-pane move: copy source color to dest, then remove source
    if let Some((from_tile, from_idx, to_tile, var_name)) = pending_move {
        // Grab color + width from source before removing
        let src_style = if let Some(egui_tiles::Tile::Pane(src)) = vp.tree.tiles.get(from_tile) {
            src.series.get(from_idx).map(|s| (s.color, s.width))
        } else {
            None
        };
        // Apply source styling to destination (overwrite default palette color)
        if let Some((color, width)) = src_style
            && let Some(egui_tiles::Tile::Pane(dst)) = vp.tree.tiles.get_mut(to_tile)
            && let Some(s) = dst.series.iter_mut().find(|s| s.var_name == var_name)
        {
            s.color = color;
            s.width = width;
        }
        // Remove from source
        if let Some(egui_tiles::Tile::Pane(src)) = vp.tree.tiles.get_mut(from_tile)
            && from_idx < src.series.len()
        {
            src.series.remove(from_idx);
            if let Selection::Series(tid, sidx) = &*vp.selection {
                if *tid == from_tile && *sidx == from_idx {
                    *vp.selection = Selection::None;
                } else if *tid == from_tile && *sidx > from_idx {
                    *vp.selection = Selection::Series(*tid, *sidx - 1);
                }
            }
        }
    }

    // Report drop feedback to status bar
    if let Some(fb) = drop_feedback {
        viewport_drop_feedback = Some(fb);
    }

    // ---- Visual overlays ----
    let top_layer_id = egui::LayerId::new(ui.layer_id().order, ui.id().with("viewport_overlays"));
    ui.ctx().set_sublayer(ui.layer_id(), top_layer_id);
    let painter = ui.painter().clone().with_layer_id(top_layer_id);

    // Blueprint hover -> viewport highlight stroke
    if let Some(hover_id) = *vp.hovered_tile
        && let Some(rect) = vp.tree.tiles.rect(hover_id)
        && !ui.rect_contains_pointer(rect)
    {
        let hover_stroke = ui.style().visuals.widgets.active.fg_stroke;
        painter.rect_stroke(rect, 0.0, hover_stroke, egui::StrokeKind::Inside);
    }

    // Drop target blue overlay
    if let Some(drop_id) = drop_hover_tile
        && let Some(rect) = vp.tree.tiles.rect(drop_id)
    {
        let stroke = egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE);
        painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
        painter.rect_filled(
            rect.shrink(2.0),
            0.0,
            theme::DROP_TARGET_STROKE.gamma_multiply(0.1),
        );
    }

    // Drag pill near cursor — shows variable names + copy/move SVG icon
    if let Some(payload) = egui::DragAndDrop::payload::<dnd::VarDragPayload>(ui.ctx())
        && let Some(pointer_pos) = ui.ctx().pointer_interact_pos()
    {
        let pill_layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_pill"));
        let mut pill_ui = egui::Ui::new(
            ui.ctx().clone(),
            egui::Id::new("pill_ui"),
            egui::UiBuilder::new().layer_id(pill_layer),
        );

        let droppable = drop_hover_tile.is_some() || *vp.drop_hover_panel;
        pill_ui.set_opacity(if droppable { 0.8 } else { 0.5 });
        let pill_frame = egui::Frame {
            fill: if droppable {
                theme::DRAG_PILL_DROPPABLE_FILL
            } else {
                theme::DRAG_PILL_NONDROPPABLE_FILL
            },
            stroke: egui::Stroke::new(
                1.0,
                if droppable {
                    theme::DRAG_PILL_DROPPABLE_STROKE
                } else {
                    theme::DRAG_PILL_NONDROPPABLE_STROKE
                },
            ),
            corner_radius: egui::CornerRadius::same(2),
            inner_margin: egui::Margin {
                left: 6,
                right: 9,
                top: 5,
                bottom: 4,
            },
            outer_margin: egui::Margin::same(1),
            ..Default::default()
        };

        let label = dnd::pill_label(&payload.names);
        let mode = dnd::effective_drag_mode(ui.ctx(), &payload.source);
        let icon_uri = match mode {
            dnd::DragMode::Copy => theme::ICON_DND_COPY,
            dnd::DragMode::Move => theme::ICON_DND_MOVE,
        };

        let resp = pill_frame
            .show(&mut pill_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add(
                        egui::Image::new(icon_uri)
                            .fit_to_exact_size(egui::vec2(14.0, 14.0))
                            .tint(ui.visuals().widgets.inactive.text_color()),
                    );
                    let text_color = ui.visuals().widgets.inactive.text_color();
                    ui.label(egui::RichText::new(label).color(text_color));
                });
            })
            .response;

        let delta = pointer_pos - resp.rect.right_bottom();
        ui.ctx()
            .transform_layer_shapes(pill_layer, emath::TSTransform::from_translation(delta));
    }

    viewport_drop_feedback
}

fn time_axis_sync_group_id(vp: &ViewportPanelState<'_>) -> Option<egui::Id> {
    vp.tree
        .tiles
        .tile_ids()
        .filter(|&tile_id| {
            matches!(
                vp.tree.tiles.get(tile_id),
                Some(egui_tiles::Tile::Pane(pane))
                    if pane.kind == PaneKind::TimeSeries && pane.properties.sync_time_axis
            )
        })
        .min_by_key(|tile_id| tile_id.0)
        .map(|tile_id| egui::Id::new(("scope2000_wave_time_axis", tile_id.0)))
}
