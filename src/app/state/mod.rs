#[path = "hardware_state.rs"]
mod hardware_state;
#[path = "ui_state.rs"]
mod ui_state;
#[path = "viewport_state.rs"]
mod viewport_state;
#[path = "workspace_state.rs"]
mod workspace_state;

pub(crate) use self::{
    hardware_state::HardwareState,
    ui_state::{UiState, VARMAP_SPLIT_DEFAULT},
    viewport_state::ViewportState,
    workspace_state::{AppConfig, CsvExportConfig, WatchRef, WorkspaceState},
};
