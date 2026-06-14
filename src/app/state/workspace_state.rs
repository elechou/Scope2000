use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app::state::VARMAP_SPLIT_DEFAULT;
use crate::wave::AcquisitionSettings;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AppConfig {
    pub port: String,
    pub baud: u32,
    pub workspace: WorkspaceState,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: 115_200,
            workspace: WorkspaceState::default(),
        }
    }
}

impl AppConfig {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("scope2000").join("app.toml"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub(crate) struct WorkspaceState {
    pub acquisition: AcquisitionSettings,
    pub watch: Vec<WatchRef>,
    pub pinned: Vec<WatchRef>,
    pub layout: LayoutState,
    pub csv_export: CsvExportConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WatchRef {
    pub var_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LayoutState {
    pub tree_json: Option<String>,
    pub blueprint_order: Vec<u64>,
    pub varmap_split: Option<f32>,
    pub data_panel_width: Option<f32>,
    pub selection_panel_width: Option<f32>,
    pub console_height: Option<f32>,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            tree_json: None,
            blueprint_order: Vec::new(),
            varmap_split: Some(VARMAP_SPLIT_DEFAULT),
            data_panel_width: None,
            selection_panel_width: None,
            console_height: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CsvExportConfig {
    pub snapshot_dir: String,
    pub filename_template: String,
    pub ultra_fast: bool,
}

impl Default for CsvExportConfig {
    fn default() -> Self {
        Self {
            snapshot_dir: String::new(),
            filename_template: "scope_{$DateTime}".to_owned(),
            ultra_fast: false,
        }
    }
}
