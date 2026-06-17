use eframe::egui;

use crate::app::ScopeApp;
use crate::app::state::{CsvExportConfig, VARMAP_SPLIT_DEFAULT, WatchRef, WorkspaceState};
use crate::console::LogLevel;
use crate::variable::WatchEntry;
use crate::wave::tiles;

impl ScopeApp {
    pub(in crate::app) fn restore_workspace_layout(&mut self) {
        let workspace = &self.config.workspace;
        self.wave.settings = workspace.acquisition.clone();
        self.wave.settings.clamp();
        self.wave.settings_snapshot = self.wave.settings.clone();
        self.plot_data.set_max_points(self.wave.settings.max_points);

        if !workspace.csv_export.snapshot_dir.is_empty() {
            self.csv.snapshot_dir = workspace.csv_export.snapshot_dir.clone();
        }
        if !workspace.csv_export.filename_template.is_empty() {
            self.csv.filename_template = workspace.csv_export.filename_template.clone();
        }
        self.csv.ultra_fast = workspace.csv_export.ultra_fast;

        if let Some(ref json) = workspace.layout.tree_json {
            if let Ok(tree) = serde_json::from_str(json) {
                self.viewport.tree = tree;
            }
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
        self.ui.data_panel_width = workspace.layout.data_panel_width;
        self.ui.selection_panel_width = workspace.layout.selection_panel_width;
        self.ui.console_height = workspace.layout.console_height;
        self.ui.apply_panel_sizes = true;
    }

    pub(in crate::app) fn restore_workspace_watch_once(&mut self) {
        if self.workspace_watch_restored {
            return;
        }
        self.workspace_watch_restored = true;
        let workspace = &self.config.workspace;
        restore_watch_refs(&mut self.inspector, &workspace.pinned, &workspace.watch);
    }

    pub(in crate::app) fn save_workspace_with_log(&mut self) {
        self.save_workspace();
        self.log
            .push(LogLevel::Notice, "Workspace saved".to_owned());
    }

    pub(in crate::app) fn save_workspace(&mut self) {
        self.config.port = self.hardware.port.clone();
        self.config.baud = self.hardware.baud;
        self.config.workspace = self.snapshot_workspace();
        if let Err(error) = self.config.save() {
            self.log
                .push(LogLevel::Warn, format!("Failed to save workspace: {error}"));
        }
    }

    fn snapshot_workspace(&self) -> WorkspaceState {
        let mut workspace = self.config.workspace.clone();
        workspace.acquisition = self.wave.settings.clone();
        workspace.watch = self
            .inspector
            .watch_vars
            .iter()
            .map(|watch| WatchRef {
                var_name: watch.var_name.clone(),
            })
            .collect();
        workspace.pinned = self
            .inspector
            .pinned
            .iter()
            .filter_map(|&index| self.inspector.descriptors.get(index))
            .map(|descriptor| WatchRef {
                var_name: descriptor.name.clone(),
            })
            .collect();
        workspace.csv_export = CsvExportConfig {
            snapshot_dir: self.csv.snapshot_dir.clone(),
            filename_template: self.csv.filename_template.clone(),
            ultra_fast: self.csv.ultra_fast,
        };
        workspace.layout.tree_json = serde_json::to_string_pretty(&self.viewport.tree).ok();
        workspace.layout.blueprint_order = self
            .viewport
            .blueprint_order
            .iter()
            .map(|id| id.0)
            .collect();
        workspace.layout.varmap_split = Some(self.ui.varmap_split);
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

fn restore_watch_refs(
    inspector: &mut crate::variable::InspectorState,
    pinned: &[WatchRef],
    watch: &[WatchRef],
) {
    inspector.pinned = pinned
        .iter()
        .filter_map(|watch| inspector.index_by_name(&watch.var_name))
        .collect();
    inspector.watch_vars = watch
        .iter()
        .filter_map(|watch| {
            let descriptor_index = inspector.index_by_name(&watch.var_name)?;
            Some(WatchEntry {
                var_name: watch.var_name.clone(),
                descriptor_index,
                write_buf: String::new(),
            })
        })
        .collect();
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
            group: 0,
        }
    }

    #[test]
    fn workspace_restore_drops_missing_variables() {
        let mut inspector = InspectorState::default();
        inspector.set_descriptors(vec![descriptor("present.pin"), descriptor("present.watch")]);
        let pinned = vec![
            WatchRef {
                var_name: "present.pin".to_owned(),
            },
            WatchRef {
                var_name: "missing.pin".to_owned(),
            },
        ];
        let watch = vec![
            WatchRef {
                var_name: "present.watch".to_owned(),
            },
            WatchRef {
                var_name: "missing.watch".to_owned(),
            },
        ];

        restore_watch_refs(&mut inspector, &pinned, &watch);

        assert_eq!(inspector.pinned, vec![0]);
        assert_eq!(inspector.watch_vars.len(), 1);
        assert_eq!(inspector.watch_vars[0].var_name, "present.watch");
    }
}
