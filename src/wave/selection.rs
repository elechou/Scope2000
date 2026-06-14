/// The currently selected item in the Wave layout.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Selection {
    #[default]
    None,
    /// A pane in the Blueprint tree.
    Pane(egui_tiles::TileId),
    /// A specific series within a pane: (pane tile ID, series index).
    Series(egui_tiles::TileId, usize),
}
