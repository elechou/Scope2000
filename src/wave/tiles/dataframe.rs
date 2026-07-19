use eframe::egui;
use egui_extras::{Column, TableBuilder};
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
        let show_all = pane.series.is_empty();
        let selected_data: Vec<_> = if show_all {
            Vec::new()
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
        let row_count = if show_all {
            self.inspector.descriptors.len()
        } else {
            selected_data.len()
        };

        let (inner_resp, payload) =
            ui.dnd_drop_zone::<VarDragPayload, _>(egui::Frame::default(), |ui| {
                ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_BODY);
                TableBuilder::new(ui)
                    .id_salt(("dataframe_table", tile_id))
                    .striped(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::remainder())
                    .column(Column::exact(96.0))
                    .header(20.0, |mut header| {
                        header.col(|ui| {
                            ui.strong("Variable");
                        });
                        header.col(|ui| {
                            ui.strong("Value");
                        });
                    })
                    .body(|body| {
                        body.rows(20.0, row_count, |mut row| {
                            let row_index = row.index();
                            let (name, value, is_system_variable) = if show_all {
                                let descriptor = &self.inspector.descriptors[row_index];
                                let value = self
                                    .inspector
                                    .values
                                    .get(row_index)
                                    .copied()
                                    .flatten()
                                    .unwrap_or(0.0);
                                (descriptor.name.as_str(), value, !descriptor.is_user())
                            } else {
                                selected_data[row_index]
                            };
                            row.col(|ui| {
                                if is_system_variable {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x =
                                            theme::SYSTEM_VARIABLE_BADGE_GAP;
                                        theme::system_variable_badge(ui, 1.0);
                                        ui.label(
                                            egui::RichText::new(name)
                                                .monospace()
                                                .color(theme::TEXT_DEFAULT),
                                        );
                                    });
                                } else {
                                    ui.label(
                                        egui::RichText::new(name)
                                            .monospace()
                                            .color(theme::TEXT_DEFAULT),
                                    );
                                }
                            });
                            row.col(|ui| {
                                ui.monospace(format!("{value:.2}"));
                            });
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
