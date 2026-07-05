use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::app::state::VARMAP_SPLIT_DEFAULT;
use crate::wave::AcquisitionSettings;

pub(crate) const WORKSPACE_AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) struct WorkspaceAutosaveState {
    saved_fingerprint: Option<u64>,
    observed_fingerprint: Option<u64>,
    dirty_since: Option<Instant>,
}

impl WorkspaceAutosaveState {
    pub fn new() -> Self {
        Self {
            saved_fingerprint: None,
            observed_fingerprint: None,
            dirty_since: None,
        }
    }

    pub fn reset(&mut self, fingerprint: u64) {
        self.saved_fingerprint = Some(fingerprint);
        self.observed_fingerprint = Some(fingerprint);
        self.dirty_since = None;
    }

    pub fn observe(&mut self, now: Instant, fingerprint: u64) -> bool {
        let Some(saved) = self.saved_fingerprint else {
            self.reset(fingerprint);
            return false;
        };
        if fingerprint == saved {
            self.observed_fingerprint = Some(fingerprint);
            self.dirty_since = None;
            return false;
        }
        if self.observed_fingerprint != Some(fingerprint) {
            self.observed_fingerprint = Some(fingerprint);
            self.dirty_since = Some(now);
            return false;
        }
        self.dirty_since
            .is_some_and(|since| now.duration_since(since) >= WORKSPACE_AUTOSAVE_DEBOUNCE)
    }

    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.dirty_since
            .map(|since| WORKSPACE_AUTOSAVE_DEBOUNCE.saturating_sub(now.duration_since(since)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AppConfig {
    pub format_version: u32,
    pub port: String,
    pub baud: u32,
    pub last_project_name: Option<String>,
    pub legacy_migration_complete: bool,
    /// Compatibility field for the pre-project app.toml format.
    #[serde(rename = "workspace", skip_serializing_if = "Option::is_none")]
    pub legacy_workspace: Option<WorkspaceState>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            format_version: 1,
            port: String::new(),
            baud: 115_200,
            last_project_name: None,
            legacy_migration_complete: false,
            legacy_workspace: None,
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("scope2000"))
    }

    pub fn path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("app.toml"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .filter(|config: &Self| config.format_version == 1)
            .unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        write_toml_atomic(&path, self)
    }
}

pub(crate) fn write_toml_atomic<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, toml::to_string_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WatchRef {
    pub var_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub write_buf: String,
    #[serde(skip_serializing_if = "is_false")]
    pub write_selected: bool,
}

impl Default for WatchRef {
    fn default() -> Self {
        Self {
            var_name: String::new(),
            write_buf: String::new(),
            write_selected: false,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_global_workspace_deserializes_as_legacy() {
        let old = r#"
port = "/dev/test"
baud = 230400

[workspace]
[[workspace.pinned]]
var_name = "control_ticks"
"#;

        let config: AppConfig = toml::from_str(old).unwrap();

        assert_eq!(config.port, "/dev/test");
        assert_eq!(config.baud, 230_400);
        assert_eq!(config.format_version, 1);
        assert_eq!(
            config.legacy_workspace.unwrap().pinned,
            vec![WatchRef {
                var_name: "control_ticks".to_owned(),
                ..WatchRef::default()
            }]
        );
        assert!(!config.legacy_migration_complete);
    }

    #[test]
    fn migrated_global_config_does_not_serialize_a_workspace() {
        let config = AppConfig {
            legacy_migration_complete: true,
            last_project_name: Some("demo".to_owned()),
            ..AppConfig::default()
        };

        let text = toml::to_string(&config).unwrap();

        assert!(!text.contains("[workspace]"));
        assert!(text.contains("last_project_name = \"demo\""));
    }

    #[test]
    fn autosave_waits_for_a_stable_debounce_window() {
        let start = Instant::now();
        let mut state = WorkspaceAutosaveState::new();
        state.reset(1);

        assert!(!state.observe(start, 2));
        assert!(!state.observe(start + Duration::from_secs(1), 2));
        assert!(!state.observe(start + Duration::from_millis(1500), 3));
        assert!(!state.observe(start + Duration::from_secs(3), 3));
        assert!(state.observe(start + Duration::from_millis(3500), 3));

        state.reset(3);
        assert!(!state.observe(start + Duration::from_secs(4), 3));
        assert!(state.remaining(start + Duration::from_secs(4)).is_none());
    }
}
