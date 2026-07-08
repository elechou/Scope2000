use std::path::PathBuf;

/// Frozen copy of wave data, ready for background CSV writing.
pub struct CsvSnapshot {
    /// Column headers sorted alphabetically by variable name.
    pub channel_names: Vec<String>,
    /// Shared trigger-relative time axis in seconds.
    pub times: Vec<f64>,
    /// `values[channel_index][sample_index]` — parallel to `times`.
    pub values: Vec<Vec<f64>>,
}

/// Pending overwrite confirmation for Quick Snapshot.
pub struct OverwritePending {
    pub csv_path: PathBuf,
    pub screenshot_path: Option<PathBuf>,
    pub snapshot: CsvSnapshot,
}

/// CSV export state: Quick Snapshot config + transient save status.
pub struct CsvState {
    /// Quick Snapshot output directory (persisted in workspace.toml).
    pub snapshot_dir: String,
    /// Filename template, e.g. `"wave_{$DateTime}"` (persisted).
    pub filename_template: String,
    /// When true, "Save Data" uses ultra-fast snapshot instead of file dialog.
    pub ultra_fast: bool,
    /// When true, saving CSV also saves a PNG screenshot beside it.
    pub save_with_screenshot: bool,

    /// Whether the settings window is open.
    pub show_settings: bool,
    /// Receiver for the background CSV write thread's result.
    pub save_rx: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    /// Target path waiting for egui's screenshot event.
    pub pending_screenshot_path: Option<PathBuf>,
    /// Receiver for the background PNG screenshot write thread's result.
    pub screenshot_save_rx: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    /// Set when a Quick Snapshot would overwrite an existing file.
    pub overwrite_pending: Option<OverwritePending>,
}

impl Default for CsvState {
    fn default() -> Self {
        Self {
            snapshot_dir: String::new(),
            filename_template: "wave_{$DateTime}".to_string(),
            ultra_fast: false,
            save_with_screenshot: false,
            show_settings: false,
            save_rx: None,
            pending_screenshot_path: None,
            screenshot_save_rx: None,
            overwrite_pending: None,
        }
    }
}
