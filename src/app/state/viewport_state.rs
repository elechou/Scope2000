use eframe::egui;

use crate::wave::pane::ViewPane;
use crate::wave::selection::Selection;
use crate::wave::tiles;

pub(crate) struct ViewportState {
    pub tree: egui_tiles::Tree<ViewPane>,
    pub blueprint_order: Vec<egui_tiles::TileId>,
    pub selection: Selection,
    pub hovered_tile: Option<egui_tiles::TileId>,
    pub hovered_blueprint_var: Option<(egui_tiles::TileId, egui::Id)>,
    pub hovered_plot_var: Option<(egui_tiles::TileId, egui::Id)>,
    pub drop_hover_panel: bool,
}

impl ViewportState {
    pub fn new() -> Self {
        Self {
            tree: tiles::create_default_tree(),
            blueprint_order: Vec::new(),
            selection: Selection::None,
            hovered_tile: None,
            hovered_blueprint_var: None,
            hovered_plot_var: None,
            drop_hover_panel: false,
        }
    }

    pub fn reset_layout(&mut self) {
        self.tree = tiles::create_default_tree();
        self.blueprint_order.clear();
        self.selection = Selection::None;
        self.hovered_tile = None;
        self.hovered_blueprint_var = None;
        self.hovered_plot_var = None;
        self.drop_hover_panel = false;
    }
}
