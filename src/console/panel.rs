use eframe::egui;

use crate::console::{LogBuffer, LogLevel};
use crate::theme;

/// Show the console log panel.
pub fn show(ui: &mut egui::Ui, log: &mut LogBuffer) {
    theme::section_header(ui, "Console");
    ui.add_space(2.0);

    // Header row: level filter, entry count, clear button
    let mut clear_clicked = false;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("log_level_filter")
            .selected_text(log.log_min_level.label())
            .width(48.0)
            .show_ui(ui, |ui| {
                for &lvl in &[
                    LogLevel::Debug,
                    LogLevel::Info,
                    LogLevel::Notice,
                    LogLevel::Warn,
                    LogLevel::Error,
                ] {
                    ui.selectable_value(&mut log.log_min_level, lvl, lvl.label());
                }
            });

        let visible = log
            .logs
            .iter()
            .filter(|e| e.level >= log.log_min_level)
            .count();
        ui.weak(format!("{visible} entries"));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(!log.logs.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                clear_clicked = true;
            }
        });
    });
    if clear_clicked {
        log.logs.clear();
    }

    let min = log.log_min_level;
    egui::ScrollArea::vertical()
        .id_salt("console_scroll")
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let any = log.logs.iter().any(|e| e.level >= min);
            if !any {
                ui.weak("No log entries yet");
            } else {
                for entry in &log.logs {
                    if entry.level < min {
                        continue;
                    }
                    let color = match entry.level {
                        LogLevel::Error => theme::RED,
                        LogLevel::Warn => theme::YELLOW,
                        LogLevel::Notice => theme::GREEN,
                        LogLevel::Debug => theme::TEXT_SUBDUED,
                        LogLevel::Info => theme::TEXT_DEFAULT,
                    };
                    ui.horizontal(|ui| {
                        ui.weak(format!("[{}]", entry.time));
                        ui.colored_label(color, entry.level.label());
                        ui.colored_label(color, &entry.message);
                    });
                }
            }
        });
}
