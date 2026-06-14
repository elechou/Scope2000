use eframe::egui;

use crate::theme;

/// Empty viewport shown before any descriptor table has been enumerated.
pub fn show(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.25);
        ui.label(
            egui::RichText::new("Scope2000")
                .size(24.0)
                .color(theme::TEXT_STRONG),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Connect to Viewer2000 to enumerate descriptors.")
                .color(theme::TEXT_SUBDUED),
        );
    });
}
