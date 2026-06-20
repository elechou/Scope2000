use eframe::egui;
use egui_tiles::TileId;

use super::MyTilesDelegate;
use crate::theme;
use crate::wave::dnd::VarDragPayload;
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
            self.var_names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let val = self.var_values.get(i).copied().unwrap_or(0.0);
                    (name.clone(), val)
                })
                .collect()
        } else {
            pane.series
                .iter()
                .filter_map(|s| {
                    self.var_names
                        .iter()
                        .position(|n| *n == s.var_name)
                        .map(|i| {
                            let val = self.var_values.get(i).copied().unwrap_or(0.0);
                            (s.var_name.clone(), val)
                        })
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

                            for (name, value) in &display_data {
                                ui.label(name);
                                ui.monospace(format!("{value:.2}"));
                                ui.end_row();
                            }
                        });
                });
            });

        if self.can_edit_variable_refs
            && inner_resp.response.contains_pointer()
            && egui::DragAndDrop::has_payload_of_type::<VarDragPayload>(ui.ctx())
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
