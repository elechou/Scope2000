use eframe::egui;

use crate::app::state::{
    AbzZeroingHealthLevel, AbzZeroingSnapshot, UiState, abz_zeroing_block_label,
    abz_zeroing_result_label, abz_zeroing_state_label,
};
use crate::theme;

pub fn show(ui: &egui::Ui, ui_state: &mut UiState, snapshot: Option<AbzZeroingSnapshot>) {
    if !ui_state.show_abz_zeroing {
        return;
    }
    egui::Window::new("ABZ Zeroing")
        .id(egui::Id::new("abz_zeroing_window"))
        .open(&mut ui_state.show_abz_zeroing)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ui.ctx().content_rect().center())
        .collapsible(false)
        .resizable(true)
        .default_width(460.0)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(420.0);
            ui.add_space(2.0);
            show_status_notice(ui, snapshot);
            ui.separator();
            show_summary(ui, snapshot);
        });
}

fn show_status_notice(ui: &mut egui::Ui, snapshot: Option<AbzZeroingSnapshot>) {
    ui.label(
        egui::RichText::new("Angle Reference Service")
            .strong()
            .color(theme::TEXT_STRONG),
    );

    let Some(snapshot) = snapshot else {
        ui.colored_label(theme::TEXT_SUBDUED, "No ABZ Zeroing status is available.");
        return;
    };

    let health = snapshot.health();
    ui.colored_label(
        health_color(health.level),
        format!("{}: {}", health.label, health.detail),
    );
}

fn show_summary(ui: &mut egui::Ui, snapshot: Option<AbzZeroingSnapshot>) {
    let snapshot = snapshot.unwrap_or_default();
    egui::Grid::new("abz_zeroing_summary")
        .num_columns(2)
        .spacing(egui::vec2(18.0, 4.0))
        .striped(true)
        .show(ui, |ui| {
            summary_row(
                ui,
                "Ready",
                ready_label(snapshot.ready),
                if snapshot.ready == Some(1) {
                    theme::GREEN
                } else {
                    theme::YELLOW
                },
            );
            summary_row(
                ui,
                "State",
                abz_zeroing_state_label(snapshot.state),
                state_color(snapshot.state),
            );
            summary_row(
                ui,
                "Result",
                abz_zeroing_result_label(snapshot.result),
                result_color(snapshot.result),
            );
            summary_row(
                ui,
                "Block Reason",
                abz_zeroing_block_label(snapshot.block_reason),
                block_color(snapshot.block_reason),
            );
            if snapshot.npe_z_good.is_some() || snapshot.npe_z_rejects.is_some() {
                summary_row(
                    ui,
                    "NPE Z Events",
                    format!(
                        "good {} seen {} rejects {}",
                        u16_text(snapshot.npe_z_good),
                        u32_text(snapshot.npe_z_seen),
                        u32_text(snapshot.npe_z_rejects)
                    ),
                    theme::TEXT_DEFAULT,
                );
            }
            if snapshot.eqep2_index_count.is_some()
                || snapshot.eqep2_index_latch.is_some()
                || snapshot.eqep2_raw_count.is_some()
            {
                summary_row(
                    ui,
                    "eQEP2 Index",
                    format!(
                        "count {} latch {} raw {}",
                        u32_text(snapshot.eqep2_index_count),
                        u16_text(snapshot.eqep2_index_latch),
                        u16_text(snapshot.eqep2_raw_count)
                    ),
                    theme::TEXT_DEFAULT,
                );
            }
            if snapshot.npe_first_latch.is_some()
                || snapshot.npe_last_latch.is_some()
                || snapshot.npe_last_reject_latch.is_some()
            {
                summary_row(
                    ui,
                    "NPE Latches",
                    format!(
                        "first {} last {} reject {}",
                        u16_text(snapshot.npe_first_latch),
                        u16_text(snapshot.npe_last_latch),
                        u16_text(snapshot.npe_last_reject_latch)
                    ),
                    theme::TEXT_DEFAULT,
                );
            }
            if snapshot.npe_dir_changes.is_some()
                || snapshot.npe_dir_resets.is_some()
                || snapshot.npe_error_resets.is_some()
            {
                summary_row(
                    ui,
                    "NPE Resets",
                    format!(
                        "dir changes {} dir resets {} error resets {}",
                        u32_text(snapshot.npe_dir_changes),
                        u32_text(snapshot.npe_dir_resets),
                        u32_text(snapshot.npe_error_resets)
                    ),
                    reset_color(snapshot),
                );
            }
            if snapshot.eqep2_status.is_some()
                || snapshot.eqep2_error_flags.is_some()
                || snapshot.npe_last_error_flags.is_some()
            {
                summary_row(
                    ui,
                    "eQEP2 Flags",
                    format!(
                        "status {} errors {} last {}",
                        hex_u16_text(snapshot.eqep2_status),
                        hex_u32_text(snapshot.eqep2_error_flags),
                        hex_u32_text(snapshot.npe_last_error_flags)
                    ),
                    error_flag_color(snapshot),
                );
            }
            if snapshot.eqep2_index_event.is_some() || snapshot.eqep2_dir_change.is_some() {
                summary_row(
                    ui,
                    "eQEP2 Latches",
                    format!(
                        "index event {} dir change {}",
                        u16_text(snapshot.eqep2_index_event),
                        u16_text(snapshot.eqep2_dir_change)
                    ),
                    theme::TEXT_DEFAULT,
                );
            }
        });
}

fn summary_row(ui: &mut egui::Ui, label: &str, value: String, color: egui::Color32) {
    ui.label(egui::RichText::new(label).color(theme::TEXT_SUBDUED));
    ui.colored_label(color, value);
    ui.end_row();
}

fn ready_label(value: Option<u16>) -> String {
    match value {
        Some(0) => "NO".to_owned(),
        Some(1) => "YES".to_owned(),
        Some(other) => format!("READY {other}"),
        None => "UNKNOWN".to_owned(),
    }
}

fn state_color(value: Option<u16>) -> egui::Color32 {
    match value {
        Some(1) => theme::YELLOW,
        Some(2) => theme::GREEN,
        Some(3) => theme::RED,
        _ => theme::TEXT_DEFAULT,
    }
}

fn result_color(value: Option<u16>) -> egui::Color32 {
    match value {
        Some(1) => theme::GREEN,
        Some(2) => theme::RED,
        Some(3) => theme::YELLOW,
        Some(0) | None => theme::TEXT_SUBDUED,
        Some(_) => theme::RED,
    }
}

fn block_color(value: Option<u16>) -> egui::Color32 {
    match value {
        Some(0) => theme::TEXT_SUBDUED,
        Some(_) => theme::RED,
        None => theme::TEXT_SUBDUED,
    }
}

fn reset_color(snapshot: AbzZeroingSnapshot) -> egui::Color32 {
    if snapshot.npe_error_resets.unwrap_or_default() != 0 {
        theme::RED
    } else if snapshot.npe_dir_resets.unwrap_or_default() != 0 {
        theme::YELLOW
    } else {
        theme::TEXT_DEFAULT
    }
}

fn error_flag_color(snapshot: AbzZeroingSnapshot) -> egui::Color32 {
    if snapshot.eqep2_error_flags.unwrap_or_default() != 0
        || snapshot.npe_last_error_flags.unwrap_or_default() != 0
    {
        theme::RED
    } else {
        theme::TEXT_DEFAULT
    }
}

fn health_color(level: AbzZeroingHealthLevel) -> egui::Color32 {
    match level {
        AbzZeroingHealthLevel::Normal => theme::TEXT_SUBDUED,
        AbzZeroingHealthLevel::Warning => theme::YELLOW,
        AbzZeroingHealthLevel::Error => theme::RED,
    }
}

fn u16_text(value: Option<u16>) -> String {
    value.map_or("-".to_owned(), |value| value.to_string())
}

fn u32_text(value: Option<u32>) -> String {
    value.map_or("-".to_owned(), |value| value.to_string())
}

fn hex_u16_text(value: Option<u16>) -> String {
    value.map_or("-".to_owned(), |value| format!("0x{value:04X}"))
}

fn hex_u32_text(value: Option<u32>) -> String {
    value.map_or("-".to_owned(), |value| format!("0x{value:08X}"))
}
