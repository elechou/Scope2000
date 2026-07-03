use eframe::egui;

use crate::app::state::UiState;
use crate::theme;

pub enum MenuAction {
    OpenProject,
    OpenRecentProject(String),
    ManageProjects,
    SaveWorkspace,
    ResetLayout,
}

/// Show the top menu bar and dock toggles.
pub fn show(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    can_configure_connection: bool,
    recent_projects: &[String],
) -> Option<MenuAction> {
    let mut action = None;
    let mut quit_clicked = false;

    egui::Panel::top("menu_bar")
        .frame(theme::menu_bar_frame())
        .show_separator_line(true)
        .show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.add_enabled(false, egui::Button::new("New Project..."));
                    if ui.button("Open Project...").clicked() {
                        action = Some(MenuAction::OpenProject);
                        ui.close_kind(egui::UiKind::Menu);
                    }
                    ui.menu_button("Open Recent Project...", |ui| {
                        if recent_projects.is_empty() {
                            ui.add_enabled(false, egui::Button::new("No Recent Projects"));
                        } else {
                            for name in recent_projects.iter().take(10) {
                                if ui.button(name).clicked() {
                                    action = Some(MenuAction::OpenRecentProject(name.clone()));
                                    ui.close_kind(egui::UiKind::Menu);
                                }
                            }
                        }
                        ui.separator();
                        if ui.button("More Projects...").clicked() {
                            action = Some(MenuAction::ManageProjects);
                            ui.close_kind(egui::UiKind::Menu);
                        }
                    });
                    if ui.button("Project Manager").clicked() {
                        action = Some(MenuAction::ManageProjects);
                        ui.close_kind(egui::UiKind::Menu);
                    }
                    ui.separator();
                    if ui.button("Save Workspace").clicked() {
                        action = Some(MenuAction::SaveWorkspace);
                        ui.close_kind(egui::UiKind::Menu);
                    }
                    if ui.button("Reset Layout").clicked() {
                        action = Some(MenuAction::ResetLayout);
                        ui.close_kind(egui::UiKind::Menu);
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        quit_clicked = true;
                        ui.close_kind(egui::UiKind::Menu);
                    }
                });

                ui.menu_button("Settings", |ui| {
                    if ui
                        .add_enabled(can_configure_connection, egui::Button::new("Connect"))
                        .clicked()
                    {
                        ui_state.show_connection_settings = true;
                        ui.close_kind(egui::UiKind::Menu);
                    }
                });

                ui.menu_button("Toolbox", |ui| {
                    if ui.button("Current Sensor Calibration").clicked() {
                        ui_state.show_current_sensor_calibration = true;
                        ui.close_kind(egui::UiKind::Menu);
                    }
                });

                ui.menu_button("About", |ui| {
                    if ui.button("About Scope2000").clicked() {
                        ui_state.show_about_window = true;
                        ui.close_kind(egui::UiKind::Menu);
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::dock_toggle(
                        ui,
                        "\u{F10AB}",
                        ui_state.show_selection_panel,
                        "Toggle Selection panel",
                    )
                    .clicked()
                    {
                        ui_state.show_selection_panel = !ui_state.show_selection_panel;
                    }
                    if theme::dock_toggle(
                        ui,
                        "\u{F10A9}",
                        ui_state.show_console_panel,
                        "Toggle Console panel",
                    )
                    .clicked()
                    {
                        ui_state.show_console_panel = !ui_state.show_console_panel;
                    }
                    if theme::dock_toggle(
                        ui,
                        "\u{F10AA}",
                        ui_state.show_system_panel,
                        "Toggle System panel",
                    )
                    .clicked()
                    {
                        ui_state.show_system_panel = !ui_state.show_system_panel;
                    }
                });
            });
        });

    if quit_clicked {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }

    action
}
