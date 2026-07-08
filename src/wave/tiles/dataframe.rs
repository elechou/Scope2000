use eframe::egui;
use egui_tiles::TileId;

use super::MyTilesDelegate;
use crate::theme;
use crate::wave::dnd::{self, VarDragPayload};
use crate::wave::pane::ViewPane;
use crate::wave::selection::Selection;

impl<'a> MyTilesDelegate<'a> {
    pub(super) fn dataframe_ui(
        &mut self,
        ui: &mut egui::Ui,
        tile_id: TileId,
        pane: &mut ViewPane,
    ) -> egui_tiles::UiResponse {
        let display_data: Vec<_> = if pane.series.is_empty() {
            self.inspector
                .descriptors
                .iter()
                .enumerate()
                .map(|(i, descriptor)| {
                    let val = self
                        .inspector
                        .values
                        .get(i)
                        .copied()
                        .flatten()
                        .unwrap_or(0.0);
                    (descriptor.name.as_str(), val, !descriptor.is_user())
                })
                .collect()
        } else {
            pane.series
                .iter()
                .filter_map(|s| {
                    let i = self.inspector.index_by_name(&s.var_name)?;
                    let val = self
                        .inspector
                        .values
                        .get(i)
                        .copied()
                        .flatten()
                        .unwrap_or(0.0);
                    Some((
                        s.var_name.as_str(),
                        val,
                        self.inspector.is_system_variable_index(i),
                    ))
                })
                .collect()
        };

        let (inner_resp, payload) =
            ui.dnd_drop_zone::<VarDragPayload, _>(egui::Frame::default(), |ui| {
                ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_BODY);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("dataframe_grid")
                        .striped(true)
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.strong("Variable");
                            ui.strong("Value");
                            ui.end_row();

                            for (name, value, is_system_variable) in &display_data {
                                if *is_system_variable {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x =
                                            theme::SYSTEM_VARIABLE_BADGE_GAP;
                                        theme::system_variable_badge(ui, 1.0);
                                        ui.label(
                                            egui::RichText::new(*name)
                                                .monospace()
                                                .color(theme::TEXT_DEFAULT),
                                        );
                                    });
                                } else {
                                    ui.label(
                                        egui::RichText::new(*name)
                                            .monospace()
                                            .color(theme::TEXT_DEFAULT),
                                    );
                                }
                                ui.monospace(format!("{value:.2}"));
                                ui.end_row();
                            }
                        });
                });
            });

        if self.can_edit_variable_refs
            && inner_resp.response.contains_pointer()
            && egui::DragAndDrop::payload::<VarDragPayload>(ui.ctx())
                .is_some_and(|payload| dnd::can_drop_on_wave_body(&payload.source, tile_id))
        {
            self.drop_hover_tile = Some(tile_id);
        }

        if self.can_edit_variable_refs
            && let Some(dropped) = payload
            && let Some(fb) = self.apply_drop(ui.ctx(), tile_id, pane, &dropped)
        {
            self.drop_feedback = Some(fb);
        }

        let body_rect = inner_resp.response.rect;
        let clicked_in_body = ui.input(|i| {
            i.pointer.primary_clicked()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| body_rect.contains(p))
        });
        if clicked_in_body && !super::is_pane_selected(self.selection, tile_id) {
            *self.selection = Selection::Pane(tile_id);
        }

        egui_tiles::UiResponse::None
    }
}
