use eframe::egui;

use crate::wave::pane::ViewPane;
use crate::wave::selection::Selection;

/// Viewport-level state needed by the blueprint/viewport panels.
/// This is a re-export of the fields of `ViewportState` that panels need.
pub struct ViewportPanelState<'a> {
    pub tree: &'a mut egui_tiles::Tree<ViewPane>,
    pub blueprint_order: &'a mut Vec<egui_tiles::TileId>,
    pub selection: &'a mut Selection,
    pub hovered_tile: &'a mut Option<egui_tiles::TileId>,
    pub hovered_blueprint_var: &'a mut Option<(egui_tiles::TileId, egui::Id)>,
    pub hovered_plot_var: &'a mut Option<(egui_tiles::TileId, egui::Id)>,
    pub drop_hover_panel: &'a mut bool,
}
