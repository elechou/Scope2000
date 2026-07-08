use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;

use eframe::egui;

use crate::app::ScopeApp;
use crate::app::state::{
    CsvExportConfig, VARMAP_SPLIT_DEFAULT, WORKSPACE_AUTOSAVE_DEBOUNCE, WatchRef, WorkspaceState,
    WorkspaceStore,
};
use crate::console::LogLevel;
use crate::variable::WatchEntry;
use crate::wave::tiles;

impl ScopeApp {
    pub(in crate::app) fn restore_workspace_layout(&mut self) {
        let workspace = &self.workspace;
        self.wave.settings = workspace.acquisition.clone();
        self.wave.settings.clamp();
        self.wave.settings_snapshot = self.wave.settings.clone();

        self.csv.snapshot_dir = workspace.csv_export.snapshot_dir.clone();
        self.csv.filename_template = if workspace.csv_export.filename_template.is_empty() {
            "scope_{$DateTime}".to_owned()
        } else {
            workspace.csv_export.filename_template.clone()
        };
        self.csv.ultra_fast = workspace.csv_export.ultra_fast;
        self.csv.save_with_screenshot = workspace.csv_export.save_with_screenshot;

        if let Some(ref json) = workspace.layout.tree_json {
            self.viewport.tree =
                serde_json::from_str(json).unwrap_or_else(|_| tiles::create_default_tree());
        } else {
            self.viewport.tree = tiles::create_default_tree();
        }
        self.viewport.blueprint_order = workspace
            .layout
            .blueprint_order
            .iter()
            .map(|&id| egui_tiles::TileId::from_u64(id))
            .collect();

        self.ui.varmap_split = workspace
            .layout
            .varmap_split
            .unwrap_or(VARMAP_SPLIT_DEFAULT);
        self.ui.varmap_continuous_refresh = workspace.layout.varmap_continuous_refresh;
        self.ui.show_system_panel = workspace.layout.show_system_panel;
        self.ui.show_console_panel = workspace.layout.show_console_panel;
        self.ui.show_selection_panel = workspace.layout.show_selection_panel;
        self.ui.data_panel_width = workspace.layout.data_panel_width;
        self.ui.selection_panel_width = workspace.layout.selection_panel_width;
        self.ui.console_height = workspace.layout.console_height;
        self.ui.apply_panel_sizes = true;
    }

    pub(in crate::app) fn restore_workspace_watch_once(&mut self) {
        if self.workspace_watch_restored
            || !self.descriptor_catalog_ready
            || !self.project.can_reconcile(self.hardware.info.as_ref())
        {
            return;
        }
        self.workspace_watch_restored = true;
        let (missing_pinned, missing_watch) = restore_watch_refs(
            &mut self.inspector,
            &self.workspace.pinned,
            &self.workspace.watch,
        );
        self.project.unresolved.pinned = missing_pinned;
        self.project.unresolved.watch = missing_watch;
        self.reconcile_layout_refs();
    }

    fn reconcile_layout_refs(&mut self) {
        let available: BTreeSet<&str> = self
            .inspector
            .descriptors
            .iter()
            .map(|descriptor| descriptor.name.as_str())
            .collect();
        let mut wave = BTreeSet::new();
        for id in self.viewport.tree.tiles.tile_ids() {
            let Some(egui_tiles::Tile::Pane(pane)) = self.viewport.tree.tiles.get(id) else {
                continue;
            };
            for series in &pane.series {
                if !available.contains(series.var_name.as_str()) {
                    wave.insert(series.var_name.clone());
                }
            }
        }
        self.project.unresolved.wave = wave.into_iter().collect();
        self.project.unresolved.trigger = self
            .wave
            .settings
            .trigger_source
            .iter()
            .filter(|name| !available.contains(name.as_str()))
            .cloned()
            .collect();
    }

    pub(in crate::app) fn save_workspace_with_log(&mut self) {
        if self.project.active_name.is_none()
            || self.project.active_name.as_deref() == Some(crate::app::state::UNTITLED_PROJECT)
        {
            self.save_workspace();
            self.log.push(
                LogLevel::Warn,
                "No persistent project workspace is active".to_owned(),
            );
            return;
        }
        self.save_workspace();
        self.log
            .push(LogLevel::Notice, "Workspace saved".to_owned());
    }

    pub(in crate::app) fn save_workspace(&mut self) {
        self.config.port = self.hardware.port.clone();
        self.config.baud = self.hardware.baud;
        self.config.last_project_name = self
            .project
            .active_name
            .clone()
            .filter(|name| name != crate::app::state::UNTITLED_PROJECT);
        self.touch_active_project_cache();
        self.workspace = self.snapshot_workspace();
        let mut workspace_saved = false;
        if let Some(name) = self
            .project
            .active_name
            .as_deref()
            .filter(|name| *name != crate::app::state::UNTITLED_PROJECT)
        {
            match WorkspaceStore::save(name, &self.workspace) {
                Ok(()) => workspace_saved = true,
                Err(error) => self
                    .log
                    .push(LogLevel::Warn, format!("Failed to save workspace: {error}")),
            }
        }
        if let Err(error) = self.project.registry.save() {
            self.log.push(
                LogLevel::Warn,
                format!("Failed to save project registry: {error}"),
            );
        }
        if let Err(error) = self.config.save() {
            self.log.push(
                LogLevel::Warn,
                format!("Failed to save application settings: {error}"),
            );
        }
        if workspace_saved {
            self.reset_workspace_autosave_baseline();
        }
    }

    fn workspace_fingerprint(&self) -> u64 {
        let workspace = self.snapshot_workspace();
        let mut hasher = DefaultHasher::new();
        match toml::to_string(&workspace) {
            Ok(text) => text.hash(&mut hasher),
            Err(_) => format!("{workspace:?}").hash(&mut hasher),
        }
        hasher.finish()
    }

    pub(in crate::app) fn reset_workspace_autosave_baseline(&mut self) {
        let fingerprint = self.workspace_fingerprint();
        self.workspace_autosave.reset(fingerprint);
    }

    pub(in crate::app) fn poll_workspace_autosave(&mut self, ctx: &egui::Context) {
        if self
            .project
            .active_name
            .as_deref()
            .is_none_or(|name| name == crate::app::state::UNTITLED_PROJECT)
        {
            return;
        }
        let now = Instant::now();
        let fingerprint = self.workspace_fingerprint();
        if self.workspace_autosave.observe(now, fingerprint) {
            self.save_workspace();
        } else if let Some(remaining) = self.workspace_autosave.remaining(now) {
            ctx.request_repaint_after(remaining.max(WORKSPACE_AUTOSAVE_DEBOUNCE / 20));
        }
    }

    pub(in crate::app) fn snapshot_workspace(&self) -> WorkspaceState {
        let mut workspace = self.workspace.clone();
        workspace.acquisition = self.wave.settings.clone();

        // Before a matching descriptor catalog has been reconciled, keep the
        // loaded name-based refs untouched. This prevents a mismatch catalog
        // from erasing the active local project's pins and watches.
        if self.workspace_watch_restored {
            let (pinned, watch) = snapshot_watch_refs(
                &self.inspector,
                &self.project.unresolved.pinned,
                &self.project.unresolved.watch,
            );
            workspace.pinned = pinned;
            workspace.watch = watch;
        }

        workspace.csv_export = CsvExportConfig {
            snapshot_dir: self.csv.snapshot_dir.clone(),
            filename_template: self.csv.filename_template.clone(),
            ultra_fast: self.csv.ultra_fast,
            save_with_screenshot: self.csv.save_with_screenshot,
        };
        workspace.layout.tree_json = serde_json::to_string_pretty(&self.viewport.tree).ok();
        workspace.layout.blueprint_order = self
            .viewport
            .blueprint_order
            .iter()
            .map(|id| id.0)
            .collect();
        workspace.layout.varmap_split = Some(self.ui.varmap_split);
        workspace.layout.varmap_continuous_refresh = self.ui.varmap_continuous_refresh;
        workspace.layout.show_system_panel = self.ui.show_system_panel;
        workspace.layout.show_console_panel = self.ui.show_console_panel;
        workspace.layout.show_selection_panel = self.ui.show_selection_panel;
        workspace.layout.data_panel_width = self.ui.data_panel_width;
        workspace.layout.selection_panel_width = self.ui.selection_panel_width;
        workspace.layout.console_height = self.ui.console_height;
        workspace
    }

    pub(in crate::app) fn apply_saved_panel_sizes(&mut self, ctx: &egui::Context) {
        use egui::containers::panel::PanelState;
        let set = |id: &str, size: egui::Vec2| {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            ctx.data_mut(|data| data.insert_persisted(egui::Id::new(id), PanelState { rect }));
        };
        if let Some(width) = self.ui.data_panel_width {
            set("data_panel", egui::vec2(width, 0.0));
        }
        if let Some(width) = self.ui.selection_panel_width {
            set("selection_panel", egui::vec2(width, 0.0));
        }
        if let Some(height) = self.ui.console_height {
            set("console_panel", egui::vec2(0.0, height));
        }
    }

    pub(in crate::app) fn record_panel_sizes(&mut self, ctx: &egui::Context) {
        if self.ui.apply_panel_sizes {
            return;
        }
        use egui::containers::panel::PanelState;
        if let Some(state) = PanelState::load(ctx, egui::Id::new("data_panel")) {
            self.ui.data_panel_width = Some(state.rect.width());
        }
        if let Some(state) = PanelState::load(ctx, egui::Id::new("selection_panel")) {
            self.ui.selection_panel_width = Some(state.rect.width());
        }
        if let Some(state) = PanelState::load(ctx, egui::Id::new("console_panel")) {
            self.ui.console_height = Some(state.rect.height());
        }
    }
}

fn snapshot_watch_refs(
    inspector: &crate::variable::InspectorState,
    unresolved_pinned: &[String],
    unresolved_watch: &[String],
) -> (Vec<WatchRef>, Vec<WatchRef>) {
    let mut pinned: Vec<String> = inspector
        .pinned
        .iter()
        .filter_map(|&index| inspector.descriptors.get(index))
        .map(|descriptor| descriptor.name.clone())
        .collect();
    for name in unresolved_pinned {
        if !pinned.contains(name) {
            pinned.push(name.clone());
        }
    }
    let mut watch: Vec<WatchRef> = inspector
        .watch_vars
        .iter()
        .map(|watch| WatchRef {
            var_name: watch.var_name.clone(),
            write_buf: watch.write_buf.clone(),
            write_selected: watch.write_selected,
        })
        .collect();
    for name in unresolved_watch {
        if !watch.iter().any(|watch| watch.var_name == *name) {
            watch.push(WatchRef {
                var_name: name.clone(),
                ..WatchRef::default()
            });
        }
    }
    (
        pinned
            .into_iter()
            .map(|var_name| WatchRef {
                var_name,
                ..WatchRef::default()
            })
            .collect(),
        watch,
    )
}

fn restore_watch_refs(
    inspector: &mut crate::variable::InspectorState,
    pinned: &[WatchRef],
    watch: &[WatchRef],
) -> (Vec<String>, Vec<String>) {
    let mut missing_pinned = Vec::new();
    inspector.pinned = pinned
        .iter()
        .filter_map(|watch| match inspector.index_by_name(&watch.var_name) {
            Some(index) => Some(index),
            None => {
                missing_pinned.push(watch.var_name.clone());
                None
            }
        })
        .collect();
    let mut missing_watch = Vec::new();
    inspector.watch_vars = watch
        .iter()
        .filter_map(|watch| {
            let Some(descriptor_index) = inspector.index_by_name(&watch.var_name) else {
                missing_watch.push(watch.var_name.clone());
                return None;
            };
            Some(WatchEntry {
                var_name: watch.var_name.clone(),
                descriptor_index,
                write_buf: watch.write_buf.clone(),
                write_selected: watch.write_selected,
            })
        })
        .collect();
    (missing_pinned, missing_watch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{VarDescriptor, VarRef, VarType};
    use crate::variable::InspectorState;

    fn descriptor(name: &str) -> VarDescriptor {
        VarDescriptor {
            name: name.to_owned(),
            var: VarRef {
                addr: 0,
                ty: VarType::F32,
            },
            kind: 0,
            prescaler: 1,
        }
    }

    #[test]
    fn workspace_restore_retains_missing_variables_as_unresolved() {
        let mut inspector = InspectorState::default();
        inspector.set_descriptors(vec![descriptor("present.pin"), descriptor("present.watch")]);
        let pinned = vec![
            WatchRef {
                var_name: "present.pin".to_owned(),
                ..WatchRef::default()
            },
            WatchRef {
                var_name: "missing.pin".to_owned(),
                ..WatchRef::default()
            },
        ];
        let watch = vec![
            WatchRef {
                var_name: "present.watch".to_owned(),
                ..WatchRef::default()
            },
            WatchRef {
                var_name: "missing.watch".to_owned(),
                ..WatchRef::default()
            },
        ];
        let (missing_pinned, missing_watch) = restore_watch_refs(&mut inspector, &pinned, &watch);
        assert_eq!(inspector.pinned, vec![0]);
        assert_eq!(missing_pinned, vec!["missing.pin"]);
        assert_eq!(missing_watch, vec!["missing.watch"]);
    }

    #[test]
    fn workspace_watch_refs_preserve_write_inputs() {
        let mut inspector = InspectorState::default();
        inspector.set_descriptors(vec![descriptor("present.watch")]);
        let watch = vec![WatchRef {
            var_name: "present.watch".to_owned(),
            write_buf: "12.5".to_owned(),
            write_selected: true,
        }];

        let (missing_pinned, missing_watch) = restore_watch_refs(&mut inspector, &[], &watch);

        assert!(missing_pinned.is_empty());
        assert!(missing_watch.is_empty());
        assert_eq!(inspector.watch_vars.len(), 1);
        assert_eq!(inspector.watch_vars[0].write_buf, "12.5");
        assert!(inspector.watch_vars[0].write_selected);

        let (_, saved_watch) = snapshot_watch_refs(&inspector, &[], &[]);
        assert_eq!(saved_watch, watch);
    }

    #[test]
    fn complete_catalog_replacement_preserves_refs_that_become_missing() {
        let mut inspector = InspectorState::default();
        inspector.set_descriptors(vec![descriptor("kept.pin"), descriptor("kept.watch")]);
        let initial_pinned = vec![WatchRef {
            var_name: "kept.pin".to_owned(),
            ..WatchRef::default()
        }];
        let initial_watch = vec![WatchRef {
            var_name: "kept.watch".to_owned(),
            ..WatchRef::default()
        }];
        let unresolved = restore_watch_refs(&mut inspector, &initial_pinned, &initial_watch);
        let (saved_pinned, saved_watch) =
            snapshot_watch_refs(&inspector, &unresolved.0, &unresolved.1);

        inspector.set_descriptors(Vec::new());
        let (missing_pinned, missing_watch) =
            restore_watch_refs(&mut inspector, &saved_pinned, &saved_watch);

        assert_eq!(missing_pinned, vec!["kept.pin"]);
        assert_eq!(missing_watch, vec!["kept.watch"]);
    }
}
