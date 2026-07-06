use eframe::{egui, emath};

use crate::theme;
use crate::variable::InspectorState;
use crate::wave::data::PlotData;
use crate::wave::dnd;
use crate::wave::pane::{PaneKind, ViewPane};
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
    scope_channel_limit: usize,
    can_edit_variable_refs: bool,
) -> Option<dnd::DropFeedback> {
    let mut viewport_drop_feedback: Option<dnd::DropFeedback> = None;
    let var_names = inspector.var_names();
    let var_values = inspector.display_values();
    let system_var_names = inspector.system_var_names();
    let scope_channel_names = crate::wave::collect_time_series_vars(&vp.tree.tiles)
        .into_iter()
        .filter(|name| {
            inspector
                .descriptor_by_name(name)
                .is_some_and(|descriptor| descriptor.is_scope())
        })
        .collect();
    let scope_capable_var_names = inspector
        .descriptors
        .iter()
        .filter(|descriptor| descriptor.is_scope())
        .map(|descriptor| descriptor.name.clone())
        .collect();
    // Blueprint hover (same frame) takes priority over plot hover (prev frame)
    let highlight_var = vp.hovered_blueprint_var.or(*vp.hovered_plot_var);
    let time_axis_sync_group = time_axis_sync_group_id(vp, plot_data);
    let (drop_hover_tile, pending_move, drop_feedback) = {
        let mut delegate = MyTilesDelegate {
            data: plot_data,
            selection: vp.selection,
            var_names: &var_names,
            var_values: &var_values,
            system_var_names: &system_var_names,
            scope_channel_limit,
            scope_channel_names,
            scope_capable_var_names,
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
        if let Some(payload) = egui::DragAndDrop::payload::<dnd::VarDragPayload>(ui.ctx()) {
            *vp.drop_action = Some(dnd::action_for_target(
                &payload.source,
                dnd::DragSurface::WaveLayout,
            ));
        }
        let stroke = egui::Stroke::new(2.0, theme::DROP_TARGET_STROKE);
        painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
        painter.rect_filled(
            rect.shrink(2.0),
            0.0,
            theme::DROP_TARGET_STROKE.gamma_multiply(0.1),
        );
    }

    // Drag pill near cursor.
    if let Some(payload) = egui::DragAndDrop::payload::<dnd::VarDragPayload>(ui.ctx())
        && let Some(pointer_pos) = ui.ctx().pointer_interact_pos()
    {
        let pill_layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_pill"));
        let mut pill_ui = egui::Ui::new(
            ui.ctx().clone(),
            egui::Id::new("pill_ui"),
            egui::UiBuilder::new().layer_id(pill_layer),
        );

        let action = *vp.drop_action;
        let droppable = action.is_some();
        pill_ui.set_opacity(if droppable { 0.86 } else { 0.5 });
        let (fill, stroke_color) = match action {
            Some(dnd::DropAction::Delete) => (
                theme::RED.gamma_multiply(0.78),
                theme::RED.gamma_multiply(1.15),
            ),
            Some(dnd::DropAction::Add | dnd::DropAction::Move) => (
                theme::DRAG_PILL_DROPPABLE_FILL,
                theme::DRAG_PILL_DROPPABLE_STROKE,
            ),
            None => (
                theme::DRAG_PILL_NONDROPPABLE_FILL,
                theme::DRAG_PILL_NONDROPPABLE_STROKE,
            ),
        };
        let pill_frame = egui::Frame {
            fill,
            stroke: egui::Stroke::new(1.0, stroke_color),
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
        let icon_uri = action.map(|action| match action {
            dnd::DropAction::Add => theme::ICON_DND_COPY,
            dnd::DropAction::Move => theme::ICON_DND_MOVE,
            dnd::DropAction::Delete => theme::ICON_DND_TRASH,
        });

        let resp = pill_frame
            .show(&mut pill_ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if let Some(icon_uri) = icon_uri {
                        ui.add(
                            egui::Image::new(icon_uri)
                                .fit_to_exact_size(egui::vec2(14.0, 14.0))
                                .tint(ui.visuals().widgets.inactive.text_color()),
                        );
                    }
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

fn time_axis_sync_group_id(vp: &ViewportPanelState<'_>, plot_data: &PlotData) -> Option<egui::Id> {
    vp.tree
        .tiles
        .tile_ids()
        .filter(|&tile_id| {
            matches!(
                vp.tree.tiles.get(tile_id),
                Some(egui_tiles::Tile::Pane(pane))
                    if pane.kind == PaneKind::TimeSeries
                        && pane.properties.sync_time_axis
                        && time_series_pane_has_samples(pane, plot_data)
            )
        })
        .min_by_key(|tile_id| tile_id.0)
        .map(|tile_id| egui::Id::new(("scope2000_wave_time_axis", tile_id.0)))
}

fn time_series_pane_has_samples(pane: &ViewPane, plot_data: &PlotData) -> bool {
    pane.series.iter().any(|series| {
        plot_data
            .series
            .get(&series.var_name)
            .is_some_and(|data| !data.times.is_empty())
    })
}
