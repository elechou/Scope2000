use eframe::egui;

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
    /// Variable dragged from the Variable Source or Variable Controller — duplicates (copy).
    Copy,
    /// Series dragged from a pane in the Wave Layout — relocates (move) unless Ctrl held.
    MoveFromPane {
        tile_id: egui_tiles::TileId,
        index: usize,
    },
}

// ---------------------------------------------------------------------------
// Effective drag mode (Ctrl modifier)
// ---------------------------------------------------------------------------

/// The resolved drag operation: Copy always duplicates, Move removes from source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragMode {
    Copy,
    Move,
}

/// Determine whether the current drag should copy or move.
///
/// - `DragSource::Copy` → always `DragMode::Copy`
/// - `DragSource::MoveFromPane` → `DragMode::Move` by default,
///   but `DragMode::Copy` when **Ctrl** is held.
pub fn effective_drag_mode(ctx: &egui::Context, source: &DragSource) -> DragMode {
    match source {
        DragSource::Copy => DragMode::Copy,
        DragSource::MoveFromPane { .. } => {
            if ctx.input(|i| i.modifiers.ctrl) {
                DragMode::Copy
            } else {
                DragMode::Move
            }
        }
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
