use crate::app::ScopeApp;
use crate::wave::pane::{PaneKind, ViewPane};

impl ScopeApp {
    pub(in crate::app) fn add_pane(&mut self, kind: PaneKind) {
        let number = Self::next_pane_number(&self.viewport.tree.tiles, kind);
        let name = format!("{} {}", kind.label(), number);
        let id = self
            .viewport
            .tree
            .tiles
            .insert_pane(ViewPane::new(name, kind));
        let added = if let Some(root_id) = self.viewport.tree.root {
            if let Some(egui_tiles::Tile::Container(container)) =
                self.viewport.tree.tiles.get_mut(root_id)
            {
                container.add_child(id);
                true
            } else {
                false
            }
        } else {
            false
        };
        if !added {
            let root = self.viewport.tree.tiles.insert_vertical_tile(vec![id]);
            self.viewport.tree.root = Some(root);
        }
    }
}
