use std::sync::mpsc;
use std::time::Instant;

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::app::state::{
    LocalProject, MutationPolicy, PROJECT_MANAGER_SPLIT_DEFAULT, ProjectBinding, ProjectStatus,
    UNTITLED_PROJECT, UnresolvedRefs, WorkspaceStore, refresh_local_build, scan_project_directory,
};
use crate::app::{LOCAL_METADATA_REFRESH_PERIOD, ScopeApp};
use crate::console::LogLevel;
use crate::source::SystemCommand;
use crate::theme;

impl ScopeApp {
    pub(in crate::app) fn project_policy(&self) -> MutationPolicy {
        self.project.policy(self.hardware.info.as_ref())
    }

    pub(in crate::app) fn start_system(&mut self) {
        if !self.project_policy().system_start {
            self.log.push(
                LogLevel::Warn,
                "System Start blocked by project safety state".to_owned(),
            );
            return;
        }
        self.send_system_command(SystemCommand::Start);
    }

    pub(in crate::app) fn handle_firmware_project(&mut self) {
        let Some(device_name) = self
            .hardware
            .info
            .as_ref()
            .map(|info| info.project_name.clone())
        else {
            return;
        };

        if let Some(local) = self.project.local.clone() {
            if local.name == device_name && device_name != UNTITLED_PROJECT {
                self.project.registry.projects.insert(
                    device_name.clone(),
                    ProjectBinding {
                        project_file: Some(local.project_file),
                        verified: true,
                        build_time_utc: local.build_time_utc,
                        build_hash: local.build_hash,
                    },
                );
                if let Err(error) = self.project.registry.save() {
                    self.log.push(
                        LogLevel::Warn,
                        format!("Failed to save project binding: {error}"),
                    );
                }
                self.offer_legacy_migration(&device_name);
            }
            self.begin_local_project_metadata_refresh();
            return;
        }

        if self.project.active_name.as_deref() != Some(device_name.as_str()) {
            self.activate_workspace(Some(device_name.clone()), None);
        }
        if device_name != UNTITLED_PROJECT {
            if let Some(local) = self.project.registry.valid_local(&device_name, true) {
                self.project.local = Some(local);
                self.begin_local_project_metadata_refresh();
            }
            self.offer_legacy_migration(&device_name);
        }
    }

    pub(in crate::app) fn handle_firmware_disconnect(&mut self) {
        if self.project.active_name.as_deref() == Some(UNTITLED_PROJECT)
            && self.project.local.is_none()
        {
            self.activate_workspace(None, None);
        }
    }

    fn offer_legacy_migration(&mut self, device_name: &str) {
        if device_name != UNTITLED_PROJECT
            && !self.config.legacy_migration_complete
            && self.config.legacy_workspace.is_some()
        {
            self.project.show_migration = true;
        }
    }

    fn project_switch_blocked(&mut self) -> bool {
        if self.hardware.is_running() || self.wave.active || self.wave.restart_pending.is_some() {
            self.log.push(
                LogLevel::Warn,
                "Stop the user system and Wave acquisition before switching projects".to_owned(),
            );
            self.ui.show_project_switch_warning = true;
            true
        } else {
            false
        }
    }

    fn activate_workspace(&mut self, name: Option<String>, local: Option<LocalProject>) {
        self.save_workspace();
        self.project.active_name = name.clone();
        self.project.local = local;
        self.local_report_path = None;
        self.project.unresolved = UnresolvedRefs::default();
        self.workspace = name
            .as_deref()
            .map(WorkspaceStore::load)
            .unwrap_or_default();
        self.wave.clear_binding();
        self.plot_data.clear();
        self.inspector.pinned.clear();
        self.inspector.watch_vars.clear();
        self.workspace_watch_restored = false;
        self.restore_workspace_layout();
        self.restore_workspace_watch_once();
        self.reset_workspace_autosave_baseline();
        self.config.last_project_name = name.filter(|name| name != UNTITLED_PROJECT);
        self.touch_active_project_cache();
        if let Err(error) = self.config.save() {
            self.log.push(
                LogLevel::Warn,
                format!("Failed to save active project: {error}"),
            );
        }
    }

    pub(in crate::app) fn touch_active_project_cache(&mut self) {
        let Some(name) = self
            .project
            .active_name
            .clone()
            .filter(|name| name != UNTITLED_PROJECT)
        else {
            return;
        };
        for evicted in self.project.registry.touch(&name) {
            if let Err(error) = WorkspaceStore::delete(&evicted) {
                self.log.push(
                    LogLevel::Warn,
                    format!("Failed to evict project cache {evicted}: {error}"),
                );
            }
        }
        if let Err(error) = self.project.registry.save() {
            self.log.push(
                LogLevel::Warn,
                format!("Failed to save recent projects: {error}"),
            );
        }
    }

    pub(in crate::app) fn begin_project_index(&mut self) {
        self.begin_project_index_for(None);
    }

    fn begin_project_index_for(&mut self, target: Option<String>) {
        if self.project_switch_blocked() || self.project_scan.is_some() {
            return;
        }
        let Some(root) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(scan_project_directory(&root));
        });
        self.project_index_target = target;
        self.project_scan = Some(rx);
    }

    pub(in crate::app) fn poll_project_scan(&mut self) {
        let Some(rx) = &self.project_scan else {
            return;
        };
        match rx.try_recv() {
            Ok(candidates) => {
                self.project_scan = None;
                if candidates.is_empty() {
                    self.project_index_target = None;
                    self.log.push(
                        LogLevel::Warn,
                        "No CCS .project file found in the selected directory".to_owned(),
                    );
                } else if candidates.len() == 1 {
                    match candidates.into_iter().next().unwrap().project {
                        Ok(project) => self.choose_local_project(project),
                        Err(error) => {
                            self.project_index_target = None;
                            self.log.push(LogLevel::Error, error);
                        }
                    }
                } else {
                    self.project_candidates = candidates;
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.project_scan = None;
                self.project_index_target = None;
                self.log
                    .push(LogLevel::Error, "CCS project scan failed".to_owned());
            }
        }
    }

    /// Re-scans the bound CCS project on an interval so a recompile that
    /// produces a fresh build report is picked up while the app is running.
    pub(in crate::app) fn maybe_refresh_local_project_metadata(&mut self) {
        if self.project.local.is_none()
            || self.project_metadata_scan.is_some()
            || Instant::now() < self.next_metadata_refresh
        {
            return;
        }
        self.begin_local_project_metadata_refresh();
    }

    pub(in crate::app) fn begin_local_project_metadata_refresh(&mut self) {
        if self.project_metadata_scan.is_some() {
            return;
        }
        let Some(local) = self.project.local.as_ref() else {
            return;
        };
        let project_file = local.project_file.clone();
        let name = local.name.clone();
        // Only reuse the cached report if it lives under this project's own
        // directory, so a stale path from a same-named project is never trusted.
        let cached_report = self.local_report_path.clone().filter(|report| {
            project_file
                .parent()
                .is_some_and(|dir| report.starts_with(dir))
        });
        self.next_metadata_refresh = Instant::now() + LOCAL_METADATA_REFRESH_PERIOD;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result =
                refresh_local_build(project_file, name, cached_report).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.project_metadata_scan = Some(rx);
    }

    pub(in crate::app) fn poll_local_project_metadata(&mut self) {
        let Some(rx) = &self.project_metadata_scan else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(scan)) => {
                self.project_metadata_scan = None;
                let project = scan.project;
                let still_active = self.project.local.as_ref().is_some_and(|current| {
                    current.name == project.name && current.project_file == project.project_file
                });
                if !still_active {
                    self.begin_local_project_metadata_refresh();
                    return;
                }
                // Remember which report fed this build so the next tick re-reads
                // just that file instead of walking the project tree again.
                self.local_report_path = scan.report_path;
                // The periodic re-scan re-reads identical metadata most of the
                // time; only mutate and persist when the build actually moved
                // so we do not rewrite projects.toml on every tick.
                let unchanged = self.project.local.as_ref().is_some_and(|current| {
                    current.build_time_utc == project.build_time_utc
                        && current.build_hash == project.build_hash
                });
                if unchanged {
                    return;
                }
                if let Some(binding) = self.project.registry.projects.get_mut(&project.name)
                    && binding.project_file.as_ref() == Some(&project.project_file)
                {
                    binding.build_time_utc = project.build_time_utc;
                    binding.build_hash = project.build_hash;
                }
                self.project.local = Some(project);
                if let Err(error) = self.project.registry.save() {
                    self.log.push(
                        LogLevel::Warn,
                        format!("Failed to cache CCS build metadata: {error}"),
                    );
                }
            }
            Ok(Err(error)) => {
                self.project_metadata_scan = None;
                self.log.push(
                    LogLevel::Warn,
                    format!("Failed to refresh CCS build metadata: {error}"),
                );
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.project_metadata_scan = None;
                self.log.push(
                    LogLevel::Warn,
                    "CCS build metadata refresh failed".to_owned(),
                );
            }
        }
    }

    fn choose_local_project(&mut self, project: LocalProject) {
        if self.project_switch_blocked() {
            return;
        }
        if let Some(target) = self.project_index_target.take()
            && project.name != target
        {
            self.log.push(
                LogLevel::Error,
                format!(
                    "Cannot bind CCS project {} to cached project {target}",
                    project.name
                ),
            );
            return;
        }
        if project.name != UNTITLED_PROJECT
            && let Some(existing) = self.project.registry.projects.get(&project.name)
            && existing.project_file.as_ref() != Some(&project.project_file)
        {
            self.pending_rebind = Some(project);
            return;
        }
        self.accept_local_project(project);
    }

    fn accept_local_project(&mut self, project: LocalProject) {
        let name = project.name.clone();
        if name != UNTITLED_PROJECT {
            let verified_by_device = self
                .hardware
                .info
                .as_ref()
                .is_some_and(|info| info.project_name == name);
            let verified_by_existing_binding = self
                .project
                .registry
                .projects
                .get(&name)
                .is_some_and(|binding| {
                    binding.verified && binding.project_file.as_ref() == Some(&project.project_file)
                });
            self.project.registry.projects.insert(
                name.clone(),
                ProjectBinding {
                    project_file: Some(project.project_file.clone()),
                    verified: verified_by_device || verified_by_existing_binding,
                    build_time_utc: project.build_time_utc,
                    build_hash: project.build_hash,
                },
            );
        }
        self.activate_workspace(Some(name.clone()), Some(project));
        self.begin_local_project_metadata_refresh();
        if self
            .hardware
            .info
            .as_ref()
            .is_some_and(|info| info.project_name == name)
        {
            self.offer_legacy_migration(&name);
        }
        if let Err(error) = self.project.registry.save() {
            self.log.push(
                LogLevel::Warn,
                format!("Failed to save project binding: {error}"),
            );
        }
    }

    pub(in crate::app) fn open_recent_project(&mut self, name: &str) {
        if self.project_switch_blocked() || !self.project.registry.projects.contains_key(name) {
            return;
        }
        let local = self.project.registry.valid_local(name, false);
        if let Some(info) = &self.hardware.info
            && info.project_name != name
            && local.is_none()
        {
            self.log.push(
                LogLevel::Warn,
                format!(
                    "Cannot open firmware-only cache {name} while connected to {}",
                    info.project_name
                ),
            );
            return;
        }
        self.activate_workspace(Some(name.to_owned()), local);
    }

    pub(in crate::app) fn project_panel(&mut self, ui: &mut egui::Ui) {
        let status = self.project.status(self.hardware.info.as_ref());
        let build_mismatch = self.project.build_mismatch(self.hardware.info.as_ref());
        let build_matches = self.project.build_matches(self.hardware.info.as_ref());
        let fill = if build_mismatch {
            // Same project, stale build: flag the name red as an alarm. This is
            // purely advisory — the mutation policy stays Matched so the user
            // can keep viewing and operating.
            theme::RED.gamma_multiply(0.78)
        } else {
            match status {
                ProjectStatus::Matched => theme::SELECT_BG,
                ProjectStatus::FirmwareOnly | ProjectStatus::UntitledDemo => theme::YELLOW,
                ProjectStatus::Conflict => theme::RED.gamma_multiply(0.78),
                ProjectStatus::NoProject
                | ProjectStatus::LocalUnverified
                | ProjectStatus::CachedDisconnected => theme::TAB_BAR,
            }
        };
        theme::section_header_colored(ui, &self.project.title(self.hardware.info.as_ref()), fill);
        ui.add_space(4.0);

        if let Some(local) = &self.project.local {
            ui.label(format!("CCS Project:  {}", local.name));
            ui.label(format!("Built Time: {}", local.build_time_display_text()));
        } else {
            ui.weak("Running from firmware...");
            ui.horizontal(|ui| {
                ui.weak("or");
                if ui.link("Index CCS Project").clicked() {
                    self.begin_project_index();
                }
            });
            let active_name = self
                .hardware
                .info
                .as_ref()
                .map(|info| info.project_name.as_str())
                .or(self.project.active_name.as_deref());
            if active_name.is_some_and(|name| {
                self.project
                    .registry
                    .projects
                    .get(name)
                    .is_some_and(|binding| {
                        binding.verified
                            && binding
                                .project_file
                                .as_ref()
                                .is_some_and(|path| !path.is_file())
                    })
            }) {
                ui.colored_label(theme::YELLOW, "Cached local binding is unavailable");
            }
        }
        ui.separator();
        if let Some(info) = &self.hardware.info {
            ui.label(format!("Firmware Project: {}", info.project_name));
            ui.label(format!(
                "Built Time: {}",
                info.build_time_local_text()
                    .unwrap_or_else(|| "Not available".to_owned())
            ));
        } else {
            ui.weak("Waiting for firmware connection...");
        }
        ui.separator();
        if status == ProjectStatus::Conflict {
            ui.colored_label(theme::RED, "Project mismatch — device mutations blocked");
        } else if status == ProjectStatus::UntitledDemo {
            ui.colored_label(theme::YELLOW, "Workspace will not be saved");
        } else if build_mismatch {
            ui.colored_label(
                theme::RED,
                "CCS build differs from connected firmware — repo artifact is not what's running",
            );
        } else if build_matches {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(
                    egui::RichText::new("Built binary matches the firmware on device.").small(),
                );
            });
        }
        let unresolved = self.project.unresolved.count();
        if unresolved > 0
            && ui
                .button(format!("Review {unresolved} missing variable reference(s)"))
                .clicked()
        {
            self.project.show_missing = true;
        }

        if self.project_scan.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Scanning CCS projects...");
            });
        }
    }

    pub(in crate::app) fn show_project_modals(&mut self, ui: &egui::Ui) {
        self.show_project_switch_warning(ui);
        self.show_candidate_modal(ui);
        self.show_rebind_modal(ui);
        self.show_migration_modal(ui);
        self.show_missing_modal(ui);
        self.show_project_manager(ui);
        self.show_delete_project_modal(ui);
    }

    fn show_project_switch_warning(&mut self, ui: &egui::Ui) {
        if !self.ui.show_project_switch_warning {
            return;
        }
        egui::Modal::new("project_switch_warning".into()).show(ui.ctx(), |ui| {
            ui.set_width(360.0);
            theme::modal_title(ui, "Project Switch Blocked");
            ui.label("Stop the user system and Wave acquisition before switching projects.");
            if theme::modal_button(ui, "OK", theme::WIDGET_BG) {
                self.ui.show_project_switch_warning = false;
            }
        });
    }

    fn show_candidate_modal(&mut self, ui: &egui::Ui) {
        if self.project_candidates.is_empty() {
            return;
        }
        let mut selected = None;
        let mut cancel = false;
        egui::Modal::new("project_candidates_modal".into()).show(ui.ctx(), |ui| {
            ui.set_width(560.0);
            theme::modal_title(ui, "Select CCS Project");
            ui.label("Multiple .project files were found. Select exactly one project.");
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (index, candidate) in self.project_candidates.iter().enumerate() {
                        match &candidate.project {
                            Ok(project) => {
                                let matches_target = self
                                    .project_index_target
                                    .as_ref()
                                    .is_none_or(|target| target == &project.name);
                                if ui
                                    .add_enabled(
                                        matches_target,
                                        egui::Button::new(format!(
                                            "{}  —  {}",
                                            project.name,
                                            candidate.relative_path.display()
                                        )),
                                    )
                                    .clicked()
                                {
                                    selected = Some(index);
                                }
                            }
                            Err(error) => {
                                ui.add_enabled(
                                    false,
                                    egui::Button::new(format!(
                                        "{} — {error}",
                                        candidate.relative_path.display()
                                    )),
                                );
                            }
                        }
                    }
                });
            if theme::modal_button(ui, "Cancel", theme::WIDGET_BG) {
                cancel = true;
            }
        });
        if cancel {
            self.project_candidates.clear();
            self.project_index_target = None;
        } else if let Some(index) = selected {
            let candidate = self.project_candidates.remove(index);
            self.project_candidates.clear();
            if let Ok(project) = candidate.project {
                self.choose_local_project(project);
            }
        }
    }

    fn show_rebind_modal(&mut self, ui: &egui::Ui) {
        let Some(project) = self.pending_rebind.clone() else {
            return;
        };
        let mut accept = false;
        let mut cancel = false;
        egui::Modal::new("project_rebind_modal".into()).show(ui.ctx(), |ui| {
            ui.set_width(460.0);
            theme::modal_title(ui, "Replace Project Binding?");
            ui.label(format!(
                "{} is already bound to another directory.",
                project.name
            ));
            ui.label(project.project_file.display().to_string());
            ui.label("The new path remains Unverified until a matching firmware handshake.");
            ui.horizontal(|ui| {
                if theme::modal_button(ui, "Replace", theme::YELLOW) {
                    accept = true;
                }
                if theme::modal_button(ui, "Cancel", theme::WIDGET_BG) {
                    cancel = true;
                }
            });
        });
        if accept {
            self.pending_rebind = None;
            self.accept_local_project(project);
        } else if cancel {
            self.pending_rebind = None;
        }
    }

    fn show_migration_modal(&mut self, ui: &egui::Ui) {
        if !self.project.show_migration {
            return;
        }
        let mut import = false;
        let mut keep = false;
        egui::Modal::new("legacy_workspace_migration".into()).show(ui.ctx(), |ui| {
            ui.set_width(440.0);
            theme::modal_title(ui, "Legacy Workspace Found");
            ui.label("The previous Scope2000 workspace has no project identity.");
            ui.label("A backup is preserved as legacy-workspace.toml.");
            ui.horizontal(|ui| {
                if theme::modal_button(ui, "Import into this Project", theme::SELECT_BG) {
                    import = true;
                }
                if theme::modal_button(ui, "Keep Project Workspace", theme::WIDGET_BG) {
                    keep = true;
                }
            });
        });
        if import || keep {
            if import && let Some(legacy) = self.config.legacy_workspace.clone() {
                self.workspace = legacy;
                self.restore_workspace_layout();
                self.workspace_watch_restored = false;
                self.restore_workspace_watch_once();
            }
            self.config.legacy_migration_complete = true;
            self.config.legacy_workspace = None;
            self.project.show_migration = false;
            self.save_workspace();
        }
    }

    fn show_missing_modal(&mut self, ui: &egui::Ui) {
        if !self.project.show_missing {
            return;
        }
        let mut close = false;
        let mut remove: Option<(&'static str, String)> = None;
        let mut remove_all = false;
        egui::Modal::new("missing_variable_refs".into()).show(ui.ctx(), |ui| {
            ui.set_width(520.0);
            theme::modal_title(ui, "Missing Variable References");
            egui::ScrollArea::vertical()
                .max_height(340.0)
                .show(ui, |ui| {
                    missing_group(
                        ui,
                        "Pinned",
                        "pinned",
                        &self.project.unresolved.pinned,
                        &mut remove,
                    );
                    missing_group(
                        ui,
                        "Watch",
                        "watch",
                        &self.project.unresolved.watch,
                        &mut remove,
                    );
                    missing_group(
                        ui,
                        "Wave",
                        "wave",
                        &self.project.unresolved.wave,
                        &mut remove,
                    );
                    missing_group(
                        ui,
                        "Trigger",
                        "trigger",
                        &self.project.unresolved.trigger,
                        &mut remove,
                    );
                });
            ui.horizontal(|ui| {
                if theme::modal_button(ui, "Remove All", theme::RED) {
                    remove_all = true;
                }
                if theme::modal_button(ui, "Close", theme::WIDGET_BG) {
                    close = true;
                }
            });
        });

        if remove_all {
            let wave: std::collections::BTreeSet<_> =
                self.project.unresolved.wave.drain(..).collect();
            self.remove_wave_refs(&wave);
            if !self.project.unresolved.trigger.is_empty() {
                self.wave.settings.trigger_source = None;
            }
            self.project.unresolved = UnresolvedRefs::default();
        } else if let Some((kind, name)) = remove {
            match kind {
                "pinned" => self
                    .project
                    .unresolved
                    .pinned
                    .retain(|value| value != &name),
                "watch" => self.project.unresolved.watch.retain(|value| value != &name),
                "wave" => {
                    self.project.unresolved.wave.retain(|value| value != &name);
                    self.remove_wave_refs(&[name.clone()].into_iter().collect());
                }
                "trigger" => {
                    self.project
                        .unresolved
                        .trigger
                        .retain(|value| value != &name);
                    if self.wave.settings.trigger_source.as_deref() == Some(name.as_str()) {
                        self.wave.settings.trigger_source = None;
                    }
                }
                _ => {}
            }
        }
        if close || self.project.unresolved.count() == 0 {
            self.project.show_missing = false;
        }
    }

    fn show_project_manager(&mut self, ui: &egui::Ui) {
        if !self.project.show_project_manager {
            return;
        }
        let records: Vec<_> = self
            .project
            .registry
            .recent_names()
            .filter_map(|name| {
                let binding = self.project.registry.projects.get(name)?;
                Some((
                    name.to_owned(),
                    binding.project_file.clone(),
                    binding.verified,
                ))
            })
            .collect();
        let mut open = self.project.show_project_manager;
        let mut action = None;
        egui::Window::new("Projects")
            .id(egui::Id::new("project_manager_window"))
            .open(&mut open)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ui.ctx().content_rect().center())
            .default_width(760.0)
            .default_height(460.0)
            .resizable(true)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.project.project_search)
                            .desired_width(260.0)
                            .hint_text("Project name or CCS path"),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Index CCS Project...").clicked() {
                            action = Some(ProjectManagerAction::IndexNew);
                        }
                    });
                });
                ui.separator();

                let filter = self.project.project_search.to_lowercase();
                let visible_records: Vec<_> = records
                    .iter()
                    .filter(|(name, project_file, _)| {
                        if filter.is_empty() {
                            return true;
                        }
                        let path_text = project_file
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "Not indexed".to_owned());
                        name.to_lowercase().contains(&filter)
                            || path_text.to_lowercase().contains(&filter)
                    })
                    .collect();

                let spacing_x = ui.spacing().item_spacing.x;
                let scroll_w = ui.spacing().scroll.allocated_width();
                let available_w = (ui.available_width() - scroll_w).max(0.0);
                let action_w = project_manager_action_width();
                let adjustable_w = (available_w - action_w - spacing_x * 2.0).max(0.0);
                let split =
                    project_manager_clamped_split(self.project.project_manager_split, adjustable_w);
                let project_w = adjustable_w * split;
                let ccs_index_w = adjustable_w - project_w;
                self.project.project_manager_split = split;

                let mut split_delta = 0.0;
                TableBuilder::new(ui)
                    .id_salt("project_manager_table")
                    .striped(true)
                    .max_scroll_height(330.0)
                    .auto_shrink(egui::Vec2b::new(false, true))
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::exact(project_w).clip(true))
                    .column(Column::exact(ccs_index_w).clip(true))
                    .column(Column::exact(action_w).clip(true).resizable(false))
                    .header(PROJECT_MANAGER_HEADER_HEIGHT, |mut header| {
                        header.col(|ui| {
                            ui.strong("Project");
                            split_delta = show_project_manager_splitter(ui);
                        });
                        header.col(|ui| {
                            ui.strong("CCS Index");
                        });
                        header.col(|ui| {
                            ui.strong("Actions");
                        });
                    })
                    .body(|body| {
                        body.rows(
                            PROJECT_MANAGER_ROW_HEIGHT,
                            visible_records.len(),
                            |mut row| {
                                let (name, project_file, verified) = visible_records[row.index()];
                                let path_text = project_file
                                    .as_ref()
                                    .map(|path| path.display().to_string())
                                    .unwrap_or_else(|| "Not indexed".to_owned());

                                row.col(|ui| {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(name).monospace())
                                            .truncate(),
                                    );
                                });
                                row.col(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.vertical(|ui| {
                                        ui.add(egui::Label::new(path_text).truncate());
                                        if project_file.is_some() {
                                            ui.weak(if *verified {
                                                "Verified"
                                            } else {
                                                "Unverified"
                                            });
                                        }
                                    });
                                });
                                row.col(|ui| {
                                    show_project_manager_actions(
                                        ui,
                                        name,
                                        project_file.is_some(),
                                        &mut action,
                                    );
                                });
                            },
                        );
                    });

                if split_delta != 0.0 && adjustable_w > 0.0 {
                    let next_project_w = (project_w + split_delta).clamp(
                        project_manager_min_project_width(adjustable_w),
                        adjustable_w - project_manager_min_index_width(adjustable_w),
                    );
                    self.project.project_manager_split = next_project_w / adjustable_w;
                    ui.ctx().request_repaint();
                }
                ui.separator();
                ui.weak(format!("{} / 100 cached projects", records.len()));
            });
        self.project.show_project_manager = open;

        match action {
            Some(ProjectManagerAction::IndexNew) => self.begin_project_index(),
            Some(ProjectManagerAction::SetIndex(name)) => {
                self.begin_project_index_for(Some(name));
            }
            Some(ProjectManagerAction::Unbind(name)) => self.unbind_project(&name),
            Some(ProjectManagerAction::Delete(name)) => {
                self.pending_delete_project = Some(name);
            }
            None => {}
        }
    }

    fn unbind_project(&mut self, name: &str) {
        if self.project.active_name.as_deref() == Some(name) && self.project_switch_blocked() {
            return;
        }
        if self.project.active_name.as_deref() == Some(name) {
            self.save_workspace();
        }
        self.project.registry.unbind(name);
        if self
            .project
            .local
            .as_ref()
            .is_some_and(|local| local.name == name)
        {
            if let Some(device_name) = self
                .hardware
                .info
                .as_ref()
                .map(|info| info.project_name.clone())
            {
                self.activate_workspace(Some(device_name), None);
            } else {
                self.project.local = None;
            }
        }
        if let Err(error) = self.project.registry.save() {
            self.log.push(
                LogLevel::Warn,
                format!("Failed to unbind project {name}: {error}"),
            );
        }
    }

    fn show_delete_project_modal(&mut self, ui: &egui::Ui) {
        let Some(name) = self.pending_delete_project.clone() else {
            return;
        };
        let mut delete = false;
        let mut cancel = false;
        egui::Modal::new("delete_project_cache_modal".into()).show(ui.ctx(), |ui| {
            ui.set_width(420.0);
            theme::modal_title(ui, "Delete Project Cache?");
            ui.label(format!(
                "Delete the binding and complete saved workspace for {name}?"
            ));
            ui.label("The CCS source directory will not be modified.");
            ui.horizontal(|ui| {
                if theme::modal_button(ui, "Delete Cache", theme::RED) {
                    delete = true;
                }
                if theme::modal_button(ui, "Cancel", theme::WIDGET_BG) {
                    cancel = true;
                }
            });
        });
        if delete {
            self.pending_delete_project = None;
            self.delete_project_cache(&name);
        } else if cancel {
            self.pending_delete_project = None;
        }
    }

    fn delete_project_cache(&mut self, name: &str) {
        let is_active = self.project.active_name.as_deref() == Some(name);
        if is_active && self.project_switch_blocked() {
            return;
        }
        self.project.registry.remove(name);
        if let Err(error) = WorkspaceStore::delete(name) {
            self.log.push(
                LogLevel::Warn,
                format!("Failed to delete project cache {name}: {error}"),
            );
            return;
        }

        if is_active {
            let device_name = self
                .hardware
                .info
                .as_ref()
                .map(|info| info.project_name.clone())
                .filter(|name| name != UNTITLED_PROJECT);
            self.project.active_name = device_name.clone();
            self.project.local = None;
            self.project.unresolved = UnresolvedRefs::default();
            self.workspace = device_name
                .as_deref()
                .filter(|device| *device != name)
                .map(WorkspaceStore::load)
                .unwrap_or_default();
            self.wave.clear_binding();
            self.plot_data.clear();
            self.inspector.pinned.clear();
            self.inspector.watch_vars.clear();
            self.workspace_watch_restored = false;
            self.restore_workspace_layout();
            self.restore_workspace_watch_once();
            self.config.last_project_name = device_name;
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
    }

    fn remove_wave_refs(&mut self, names: &std::collections::BTreeSet<String>) {
        for id in self.viewport.tree.tiles.tile_ids().collect::<Vec<_>>() {
            if let Some(egui_tiles::Tile::Pane(pane)) = self.viewport.tree.tiles.get_mut(id) {
                pane.series
                    .retain(|series| !names.contains(&series.var_name));
            }
        }
    }
}

enum ProjectManagerAction {
    IndexNew,
    SetIndex(String),
    Unbind(String),
    Delete(String),
}

const PROJECT_MANAGER_HEADER_HEIGHT: f32 = 24.0;
const PROJECT_MANAGER_ROW_HEIGHT: f32 = 40.0;
const PROJECT_MANAGER_ACTION_BUTTON_HEIGHT: f32 = 22.0;
const PROJECT_MANAGER_ACTION_GAP: f32 = 4.0;
const PROJECT_MANAGER_SET_INDEX_W: f32 = 104.0;
const PROJECT_MANAGER_UNBIND_W: f32 = 58.0;
const PROJECT_MANAGER_DELETE_W: f32 = 88.0;

fn project_manager_action_width() -> f32 {
    PROJECT_MANAGER_SET_INDEX_W
        + PROJECT_MANAGER_UNBIND_W
        + PROJECT_MANAGER_DELETE_W
        + PROJECT_MANAGER_ACTION_GAP * 2.0
}

fn project_manager_min_project_width(adjustable_w: f32) -> f32 {
    96.0_f32.min(adjustable_w * 0.5)
}

fn project_manager_min_index_width(adjustable_w: f32) -> f32 {
    160.0_f32.min(adjustable_w - project_manager_min_project_width(adjustable_w))
}

fn project_manager_clamped_split(split: f32, adjustable_w: f32) -> f32 {
    if adjustable_w <= 0.0 {
        return PROJECT_MANAGER_SPLIT_DEFAULT;
    }

    let min_split = project_manager_min_project_width(adjustable_w) / adjustable_w;
    let max_split = 1.0 - project_manager_min_index_width(adjustable_w) / adjustable_w;
    split.clamp(min_split, max_split)
}

fn show_project_manager_splitter(ui: &mut egui::Ui) -> f32 {
    let rect = ui.max_rect();
    let handle_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - 4.0, rect.top()),
        egui::pos2(rect.right() + 4.0, rect.bottom()),
    );
    let response = ui
        .interact(
            handle_rect,
            ui.make_persistent_id("project_manager_project_ccs_splitter"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let active = response.hovered() || response.dragged();
    let color = if active {
        theme::TEXT_SUBDUED
    } else {
        theme::SEPARATOR
    };
    ui.painter().vline(
        handle_rect.center().x,
        handle_rect.y_range(),
        egui::Stroke::new(1.0, color),
    );
    if response.dragged() {
        response.drag_delta().x
    } else {
        0.0
    }
}

fn show_project_manager_actions(
    ui: &mut egui::Ui,
    name: &str,
    has_project_file: bool,
    action: &mut Option<ProjectManagerAction>,
) {
    ui.spacing_mut().item_spacing.x = PROJECT_MANAGER_ACTION_GAP;
    let size = |width| egui::vec2(width, PROJECT_MANAGER_ACTION_BUTTON_HEIGHT);

    if ui
        .add_sized(
            size(PROJECT_MANAGER_SET_INDEX_W),
            egui::Button::new(if has_project_file {
                "Change Index..."
            } else {
                "Set Index..."
            }),
        )
        .clicked()
    {
        *action = Some(ProjectManagerAction::SetIndex(name.to_owned()));
    }
    if ui
        .add_enabled_ui(has_project_file, |ui| {
            ui.add_sized(size(PROJECT_MANAGER_UNBIND_W), egui::Button::new("Unbind"))
        })
        .inner
        .clicked()
    {
        *action = Some(ProjectManagerAction::Unbind(name.to_owned()));
    }
    if ui
        .add_sized(
            size(PROJECT_MANAGER_DELETE_W),
            egui::Button::new("Delete Cache"),
        )
        .clicked()
    {
        *action = Some(ProjectManagerAction::Delete(name.to_owned()));
    }
}

fn missing_group(
    ui: &mut egui::Ui,
    title: &str,
    kind: &'static str,
    names: &[String],
    remove: &mut Option<(&'static str, String)>,
) {
    if names.is_empty() {
        return;
    }
    ui.strong(title);
    for name in names {
        ui.horizontal(|ui| {
            ui.monospace(name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Remove").clicked() {
                    *remove = Some((kind, name.clone()));
                }
            });
        });
    }
    ui.separator();
}
