// ---------------------------------------------------------------------------
// Drag payload & source
// ---------------------------------------------------------------------------

/// Unified drag-and-drop payload for variables: carries the variable names plus
/// a tag indicating where the drag originated.
#[derive(Debug, Clone)]
pub struct VarDragPayload {
    pub names: Vec<String>,
    pub source: DragSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragSource {
    /// Variable dragged from the descriptor catalog in Variable Map.
    VariableMap,
    /// Pinned variable dragged inside Variable Map.
    VariableMapPinned,
    /// Watched variable dragged from Variable Controller.
    VariableController,
    /// Series dragged from a pane in Wave Layout.
    WaveLayout {
        tile_id: egui_tiles::TileId,
        index: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragSurface {
    VariableMap,
    VariableController,
    WaveLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAction {
    Add,
    Move,
    Delete,
}

impl DragSource {
    pub fn surface(self) -> DragSurface {
        match self {
            Self::VariableMap | Self::VariableMapPinned => DragSurface::VariableMap,
            Self::VariableController => DragSurface::VariableController,
            Self::WaveLayout { .. } => DragSurface::WaveLayout,
        }
    }

    pub fn can_delete(self) -> bool {
        !matches!(self, Self::VariableMap)
    }
}

/// Resolve the visible operation from the hovered drop target, not from the
/// drag source alone.
pub fn action_for_target(source: &DragSource, target: DragSurface) -> DropAction {
    if source.surface() != target {
        return DropAction::Add;
    }

    match (source, target) {
        (DragSource::VariableMapPinned, DragSurface::VariableMap)
        | (DragSource::VariableController, DragSurface::VariableController)
        | (DragSource::WaveLayout { .. }, DragSurface::WaveLayout) => DropAction::Move,
        (DragSource::VariableMap, DragSurface::VariableMap) => DropAction::Add,
        _ => DropAction::Add,
    }
}

pub fn can_drop_on_wave_body(source: &DragSource, target_tile: egui_tiles::TileId) -> bool {
    match source {
        DragSource::WaveLayout { tile_id, .. } => *tile_id != target_tile,
        DragSource::VariableMap
        | DragSource::VariableMapPinned
        | DragSource::VariableController => true,
    }
}

// ---------------------------------------------------------------------------
// Drop feedback (for status bar messages)
// ---------------------------------------------------------------------------

pub enum DropFeedback {
    /// Successfully added variable(s) to a pane.
    Added {
        var_names: Vec<String>,
        pane_name: String,
    },
    /// Drop was rejected because the variable(s) are already in the target pane.
    Duplicate { var_name: String, pane_name: String },
    /// Drop was rejected because it would exceed the device scope channel limit.
    ScopeChannelLimit { var_name: String, limit: usize },
}

impl DropFeedback {
    /// Format as a human-readable status-bar message.
    pub fn message(&self) -> String {
        match self {
            Self::Added {
                var_names,
                pane_name,
            } => {
                if var_names.len() == 1 {
                    format!("Added {} to {}", var_names[0], pane_name)
                } else {
                    format!("Added {} variables to {}", var_names.len(), pane_name)
                }
            }
            Self::Duplicate {
                var_name,
                pane_name,
            } => {
                format!("{} already in {}", var_name, pane_name)
            }
            Self::ScopeChannelLimit { var_name, limit } => {
                format!("Viewer2000 scope channel limit is {limit}; {var_name} was not added")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pill label helpers
// ---------------------------------------------------------------------------

/// Format variable names for the drag pill.
/// Shows up to 3 names, then "…+N" for the rest.
pub fn pill_label(names: &[String]) -> String {
    match names.len() {
        0 => "0 variables".to_string(),
        1 => names[0].clone(),
        2 => format!("{}, {}", names[0], names[1]),
        3 => format!("{}, {}, {}", names[0], names[1], names[2]),
        n => format!("{}, {}, {} …+{}", names[0], names[1], names[2], n - 3),
    }
}
