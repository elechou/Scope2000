use eframe::egui;

use crate::console::{LogBuffer, LogLevel};
use crate::theme;

/// Show the console log panel.
pub fn show(ui: &mut egui::Ui, log: &mut LogBuffer) {
    theme::section_header(ui, "Console");
    ui.add_space(2.0);

    // Header row: level filter, entry count, clear button
    let mut clear_clicked = false;
    let visible = ui
        .horizontal(|ui| {
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

            let visible = log.visible_entry_count(log.log_min_level);
            ui.weak(format!("{visible} entries"));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!log.logs.is_empty(), egui::Button::new("Clear"))
                    .clicked()
                {
                    clear_clicked = true;
                }
            });
            visible
        })
        .inner;
    if clear_clicked {
        log.clear();
    }
    let visible = if clear_clicked { 0 } else { visible };

    let min = log.log_min_level;
    let row_height = ui.text_style_height(&egui::TextStyle::Body);

    egui::ScrollArea::vertical()
        .id_salt("console_scroll")
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show_rows(ui, row_height, visible.max(1), |ui, row_range| {
            if visible == 0 {
                ui.weak("No log entries yet");
            } else {
                for entry in log
                    .visible_entries(min)
                    .skip(row_range.start)
                    .take(row_range.len())
                {
                    show_entry(ui, entry);
                }
            }
        });
}

fn show_entry(ui: &mut egui::Ui, entry: &crate::console::LogEntry) {
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
