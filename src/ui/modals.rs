use eframe::egui;

use crate::app::state::{HardwareState, UiState};
use crate::theme;

/// Cancel a close request while the board is Running and surface the stop warning.
pub fn handle_close_guard(ui: &egui::Ui, hardware: &HardwareState, ui_state: &mut UiState) {
    if ui.ctx().input(|i| i.viewport().close_requested()) && hardware.is_running() {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ui_state.stop_warning_action = Some("Close");
    }
}

/// Modal: "Board is still running -- stop it before doing X" warning.
pub fn show_stop_warning(ui: &egui::Ui, ui_state: &mut UiState) {
    if let Some(action) = ui_state.stop_warning_action {
        egui::Modal::new("stop_warning_modal".into()).show(ui.ctx(), |ui| {
            ui.set_width(280.0);
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                theme::modal_title(ui, "Board is still running");
                ui.add_space(4.0);
                ui.label(format!(
                    "Please stop the board before performing \"{action}\"."
                ));
                ui.add_space(12.0);
                if theme::modal_button(ui, "OK", theme::WIDGET_BG) {
                    ui_state.stop_warning_action = None;
                }
                ui.add_space(4.0);
            });
        });
    }
}

/// "About Scope2000" modal (triggered from the menu bar).
pub fn show_about_window(ui: &egui::Ui, ui_state: &mut UiState) {
    if ui_state.show_about_window {
        egui::Modal::new("about_modal".into()).show(ui.ctx(), |ui| {
            ui.set_width(300.0);
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Scope2000")
                        .strong()
                        .size(18.0)
                        .color(theme::TEXT_STRONG),
                );
                ui.label(
                    egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .color(theme::TEXT_SUBDUED),
                );
                ui.add_space(8.0);
                ui.label("Native host UI for Viewer2000,");
                ui.label("built with Rust + egui.");
                ui.add_space(12.0);
                if theme::modal_button(ui, "OK", theme::WIDGET_BG) {
                    ui_state.show_about_window = false;
                }
                ui.add_space(4.0);
            });
        });
    }
}
