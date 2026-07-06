mod dataframe;
mod time_series;

use std::collections::HashSet;

use eframe::egui;
use egui::{Color32, Stroke, vec2};
use egui_tiles::{TileId, Tiles};

use super::dnd::{DragSource, VarDragPayload};
use super::pane::{self, PaneKind, ViewPane};
use super::selection::Selection;
use crate::theme;
use crate::wave::data::PlotData;

/// Returns true if `selection` points at the pane with id `tile_id`
/// (either `Pane(tile_id)` or any `Series(tile_id, _)`).
fn is_pane_selected(selection: &Selection, tile_id: TileId) -> bool {
    match selection {
        Selection::Pane(id) => *id == tile_id,
        Selection::Series(id, _) => *id == tile_id,
        _ => false,
    }
}

/// The egui_tiles behavior delegate — renders each pane based on its kind.
pub struct MyTilesDelegate<'a> {
    pub data: &'a PlotData,
    pub selection: &'a mut Selection,
    pub var_names: &'a [String],
    pub var_values: &'a [f64],
    pub system_var_names: &'a [String],
    pub scope_channel_limit: usize,
    pub scope_channel_names: HashSet<String>,
    pub scope_capable_var_names: HashSet<String>,
    /// Set when a pane's drop zone is hovered with a valid drag payload.
    pub drop_hover_tile: Option<TileId>,
    /// Which curve to highlight: (pane_id, var_id). Merged from blueprint hover + plot hover.
    pub highlight_var: Option<(TileId, egui::Id)>,
    /// Output: plot curve/legend hover detected this frame.
    pub hovered_plot_var: Option<(TileId, egui::Id)>,
    /// Output: a cross-pane series move was dropped on a viewport pane this
    /// frame. The source pane will need its series at `from_idx` removed after
    /// the tile tree finishes rendering.
    /// Fields: (from_tile, from_idx, to_tile, var_name)
    pub pending_cross_pane_move: Option<(TileId, usize, TileId, String)>,
    /// Output: feedback from the last drop action this frame.
    pub drop_feedback: Option<super::dnd::DropFeedback>,
    pub can_edit_variable_refs: bool,
    pub time_axis_sync_group: Option<egui::Id>,
}

impl<'a> egui_tiles::Behavior<ViewPane> for MyTilesDelegate<'a> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        tile_id: TileId,
        pane: &mut ViewPane,
    ) -> egui_tiles::UiResponse {
        match pane.kind {
            PaneKind::TimeSeries => self.time_series_ui(ui, tile_id, pane),
            PaneKind::Dataframe => self.dataframe_ui(ui, tile_id, pane),
        }
    }

    fn tab_title_for_pane(&mut self, pane: &ViewPane) -> egui::WidgetText {
        pane.name.as_str().into()
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }

    fn paint_on_top_of_tile(
        &self,
        painter: &egui::Painter,
        _style: &egui::Style,
        _tile_id: TileId,
        rect: egui::Rect,
    ) {
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, theme::SEPARATOR),
            egui::StrokeKind::Inside,
        );
    }

    // ---- Tab bar styling ----

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> Color32 {
        theme::TAB_BAR
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        28.0
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        1.0
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> Stroke {
        Stroke::new(1.0, theme::SEPARATOR)
    }

    fn tab_title_spacing(&self, _visuals: &egui::Visuals) -> f32 {
        8.0
    }

    // ---- Tab rendering ----

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &Tiles<ViewPane>,
        _tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        if state.active {
            theme::BG_BODY
        } else {
            Color32::TRANSPARENT
        }
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &Tiles<ViewPane>,
        _tile_id: TileId,
        _state: &egui_tiles::TabState,
    ) -> Stroke {
        Stroke::NONE
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &Tiles<ViewPane>,
        _tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> Color32 {
        if state.active {
            theme::TEXT_STRONG
        } else {
            theme::TEXT_SUBDUED
        }
    }

    fn tab_ui(
        &mut self,
        tiles: &mut Tiles<ViewPane>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        let icon_uri = tiles.get(tile_id).and_then(|t| match t {
            egui_tiles::Tile::Pane(p) => Some(p.kind.icon_uri()),
            _ => None,
        });

        let title = self.tab_title_for_tile(tiles, tile_id);
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let galley = title.into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            font_id.clone(),
        );

        let icon_width = if icon_uri.is_some() {
            theme::ICON_SIZE.x + 4.0
        } else {
            0.0
        };
        let x_margin = self.tab_title_spacing(ui.visuals());

        let close_size = egui::Vec2::splat(self.close_button_outer_size());
        let close_padding = if state.closable {
            4.0 + close_size.x
        } else {
            0.0
        };

        let button_width = icon_width + galley.size().x + 2.0 * x_margin + close_padding;
        let (_, tab_rect) = ui.allocate_space(vec2(button_width, ui.available_height()));

        let response = ui
            .interact(tab_rect, id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Grab);

        let is_selected = is_pane_selected(self.selection, tile_id);

        if ui.is_rect_visible(tab_rect) && !state.is_being_dragged {
            let bg = if is_selected {
                theme::SELECT_BG
            } else if state.active {
                theme::BG_BODY
            } else if response.hovered() {
                theme::WIDGET_HOVER
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(tab_rect, 0.0, bg);

            if state.active {
                let underline_rect = egui::Rect::from_min_max(
                    egui::pos2(tab_rect.left(), tab_rect.bottom() - 2.0),
                    tab_rect.right_bottom(),
                );
                ui.painter()
                    .rect_filled(underline_rect, 0.0, theme::SELECT_STROKE);
            }

            let text_color = if is_selected || state.active {
                theme::TEXT_STRONG
            } else if response.hovered() {
                theme::TEXT_DEFAULT
            } else {
                theme::TEXT_SUBDUED
            };

            let mut text_x = tab_rect.left() + x_margin;
            if let Some(uri) = icon_uri {
                let icon_pos = egui::pos2(text_x, tab_rect.center().y - theme::ICON_SIZE.y / 2.0);
                let icon_rect = egui::Rect::from_min_size(icon_pos, theme::ICON_SIZE);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(icon_rect));
                child.add(
                    egui::Image::new(uri)
                        .fit_to_exact_size(theme::ICON_SIZE)
                        .tint(text_color),
                );
                text_x += theme::ICON_SIZE.x + 4.0;
            }

            let text_pos = egui::pos2(text_x, tab_rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(text_pos, galley, text_color);

            if state.closable {
                let close_rect = egui::Align2::RIGHT_CENTER
                    .align_size_within_rect(close_size, tab_rect.shrink(x_margin));
                let close_id = ui.auto_id_with("tab_close");
                let close_resp = ui
                    .interact(close_rect, close_id, egui::Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::Default);

                let close_vis = ui.style().interact(&close_resp);
                let xr = close_rect
                    .shrink(self.close_button_inner_margin())
                    .expand(close_vis.expansion);
                ui.painter()
                    .line_segment([xr.left_top(), xr.right_bottom()], close_vis.fg_stroke);
                ui.painter()
                    .line_segment([xr.right_top(), xr.left_bottom()], close_vis.fg_stroke);

                if (close_resp.clicked() || response.clicked_by(egui::PointerButton::Middle))
                    && self.on_tab_close(tiles, tile_id)
                {
                    tiles.remove(tile_id);
                }
            }
        }

        if response.clicked() && !is_pane_selected(self.selection, tile_id) {
            *self.selection = Selection::Pane(tile_id);
        }

        self.on_tab_button(tiles, tile_id, response)
    }

    fn is_tab_closable(&self, _tiles: &Tiles<ViewPane>, _tile_id: TileId) -> bool {
        true
    }

    // ---- Drag & drop ----

    fn drag_ui(&mut self, tiles: &Tiles<ViewPane>, ui: &mut egui::Ui, tile_id: TileId) {
        let mut frame = egui::Frame::popup(ui.style());
        frame.fill = frame.fill.gamma_multiply(0.5);
        frame.show(ui, |ui| {
            let text = self.tab_title_for_tile(tiles, tile_id);
            ui.label(text);
        });
    }

    fn dragged_overlay_color(&self, _visuals: &egui::Visuals) -> Color32 {
        theme::BG_BODY.gamma_multiply(0.5)
    }

    fn drag_preview_stroke(&self, _visuals: &egui::Visuals) -> Stroke {
        Stroke::new(2.0, theme::SELECT_STROKE)
    }

    fn drag_preview_color(&self, _visuals: &egui::Visuals) -> Color32 {
        theme::SELECT_BG.gamma_multiply(0.3)
    }

    fn on_edit(&mut self, _edit_action: egui_tiles::EditAction) {}
}

impl<'a> MyTilesDelegate<'a> {
    pub(crate) fn is_system_variable_name(&self, name: &str) -> bool {
        self.system_var_names
            .iter()
            .any(|system_name| system_name == name)
    }

    /// Apply a VarDragPayload drop to a pane. Returns feedback for the status bar.
    pub(crate) fn apply_drop(
        &mut self,
        _ctx: &egui::Context,
        tile_id: TileId,
        pane: &mut ViewPane,
        payload: &VarDragPayload,
    ) -> Option<super::dnd::DropFeedback> {
        if !self.can_edit_variable_refs {
            return None;
        }
        use super::dnd::{DragSurface, DropAction, DropFeedback};

        let action = super::dnd::action_for_target(&payload.source, DragSurface::WaveLayout);
        match (payload.source, action) {
            (_, DropAction::Add) => {
                let mut added = Vec::new();
                let mut added_scope_channels = 0;
                for var_name in &payload.names {
                    if pane.series.iter().any(|s| &s.var_name == var_name) {
                        return Some(DropFeedback::Duplicate {
                            var_name: var_name.clone(),
                            pane_name: pane.name.clone(),
                        });
                    }
                    if let Some(feedback) =
                        self.scope_channel_limit_feedback(pane, var_name, added_scope_channels)
                    {
                        return Some(feedback);
                    }
                    let color_idx = pane.series.len();
                    pane.add_series(var_name.clone(), pane::default_color(color_idx));
                    if self.adds_scope_channel(pane, var_name) {
                        added_scope_channels += 1;
                    }
                    added.push(var_name.clone());
                }
                if added.is_empty() {
                    None
                } else {
                    Some(DropFeedback::Added {
                        var_names: added,
                        pane_name: pane.name.clone(),
                    })
                }
            }
            (
                DragSource::WaveLayout {
                    tile_id: from_tile,
                    index: from_idx,
                },
                DropAction::Move,
            ) => {
                if from_tile == tile_id {
                    return None;
                }
                if let Some(name) = payload.names.first() {
                    if pane.series.iter().any(|s| &s.var_name == name) {
                        return Some(DropFeedback::Duplicate {
                            var_name: name.clone(),
                            pane_name: pane.name.clone(),
                        });
                    }
                    if let Some(feedback) = self.scope_channel_limit_feedback(pane, name, 0) {
                        return Some(feedback);
                    }
                    let color_idx = pane.series.len();
                    pane.add_series(name.clone(), pane::default_color(color_idx));
                    self.pending_cross_pane_move =
                        Some((from_tile, from_idx, tile_id, name.clone()));
                    Some(DropFeedback::Added {
                        var_names: vec![name.clone()],
                        pane_name: pane.name.clone(),
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn scope_channel_limit_feedback(
        &self,
        pane: &ViewPane,
        var_name: &str,
        pending_new_scope_channels: usize,
    ) -> Option<super::dnd::DropFeedback> {
        if self.scope_channel_limit == 0 || !self.adds_scope_channel(pane, var_name) {
            return None;
        }
        let next_count = self.scope_channel_names.len() + pending_new_scope_channels + 1;
        (next_count > self.scope_channel_limit).then(|| {
            super::dnd::DropFeedback::ScopeChannelLimit {
                var_name: var_name.to_owned(),
                limit: self.scope_channel_limit,
            }
        })
    }

    fn adds_scope_channel(&self, pane: &ViewPane, var_name: &str) -> bool {
        pane.kind == PaneKind::TimeSeries
            && self.scope_capable_var_names.contains(var_name)
            && !self.scope_channel_names.contains(var_name)
    }
}

/// Create a default tile tree with one empty time-series plot.
pub fn create_default_tree() -> egui_tiles::Tree<ViewPane> {
    let mut tiles = egui_tiles::Tiles::default();
    let plot1 = tiles.insert_pane(ViewPane::new("Time Series 1", PaneKind::TimeSeries));
    let root = tiles.insert_tab_tile(vec![plot1]);
    egui_tiles::Tree::new("viewport", root, tiles)
}
