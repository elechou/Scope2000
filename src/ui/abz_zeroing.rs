use eframe::egui;

use crate::app::state::{
    AbzZeroingHealthLevel, AbzZeroingSnapshot, SrmOpenLoopGate, SrmOpenLoopSnapshot,
    SrmOpenLoopState, UiState, abz_zeroing_block_label, abz_zeroing_result_label,
    abz_zeroing_state_label,
};
use crate::theme;

pub enum AbzZeroingAction {
    RunSrmOpenLoopAbz,
}

pub fn show(
    ui: &egui::Ui,
    ui_state: &mut UiState,
    srm_open_loop: &mut SrmOpenLoopState,
    snapshot: Option<AbzZeroingSnapshot>,
    srm_snapshot: Option<SrmOpenLoopSnapshot>,
    srm_gate: SrmOpenLoopGate,
) -> Option<AbzZeroingAction> {
    if !ui_state.show_abz_zeroing {
        return None;
    }
    let mut action = None;
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
            ui.separator();
            show_srm_open_loop(ui, srm_open_loop, srm_snapshot, srm_gate);
        });

    if srm_open_loop.show_run_confirmation {
        egui::Modal::new("srm_open_loop_abz_confirm".into()).show(ui.ctx(), |ui| {
            ui.set_width(380.0);
            ui.add_space(8.0);
            theme::modal_title(ui, "Start SRM Open-loop ABZ?");
            ui.add_space(8.0);
            ui.label(
                "Scope2000 will Start Viewer2000, keep the powered SRM open-loop ABZ run active, then Stop Viewer2000 when ABZ Zeroing reports ready.",
            );
            ui.label(
                egui::RichText::new("If ABZ Ready stays unknown or not ready, the powered run continues until Stop, Fault, or disconnect.")
                    .color(theme::TEXT_SUBDUED),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if theme::modal_button(ui, "Start", theme::YELLOW) {
                    if srm_gate.can_run {
                        action = Some(AbzZeroingAction::RunSrmOpenLoopAbz);
                    }
                    srm_open_loop.show_run_confirmation = false;
                }
                if theme::modal_button(ui, "Cancel", theme::WIDGET_BG) {
                    srm_open_loop.show_run_confirmation = false;
                }
            });
            ui.add_space(4.0);
        });
    }

    action
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

fn show_srm_open_loop(
    ui: &mut egui::Ui,
    srm_open_loop: &mut SrmOpenLoopState,
    snapshot: Option<SrmOpenLoopSnapshot>,
    gate: SrmOpenLoopGate,
) {
    ui.label(
        egui::RichText::new("SRM Open-loop ABZ")
            .strong()
            .color(theme::TEXT_STRONG),
    );

    let run_w = ui.available_width();
    if theme::action_button_w(ui, "Start", theme::YELLOW, gate.can_run, run_w) {
        srm_open_loop.show_run_confirmation = true;
    }

    if let Some(text) = srm_workflow_text(srm_open_loop) {
        ui.colored_label(theme::YELLOW, text);
    } else if let Some(reason) = gate.reason {
        ui.colored_label(theme::TEXT_SUBDUED, reason);
    }

    let Some(snapshot) = snapshot else {
        return;
    };

    ui.add_space(4.0);
    egui::Grid::new("srm_open_loop_summary")
        .num_columns(2)
        .spacing(egui::vec2(18.0, 4.0))
        .striped(true)
        .show(ui, |ui| {
            summary_row(
                ui,
                "DC Voltage",
                voltage_text(snapshot.dc_v),
                theme::TEXT_DEFAULT,
            );
            summary_row(
                ui,
                "Peak Duty",
                duty_text(snapshot.peak_duty),
                theme::TEXT_DEFAULT,
            );
            summary_row(ui, "Ticks", u32_text(snapshot.ticks), theme::TEXT_DEFAULT);
            summary_row(
                ui,
                "eQEP Errors",
                hex_u32_text(snapshot.eqep_errors),
                if snapshot.eqep_errors.unwrap_or_default() == 0 {
                    theme::TEXT_DEFAULT
                } else {
                    theme::RED
                },
            );
        });
}

fn srm_workflow_text(srm_open_loop: &SrmOpenLoopState) -> Option<String> {
    match srm_open_loop.phase {
        crate::app::state::SrmOpenLoopPhase::Idle => None,
        crate::app::state::SrmOpenLoopPhase::StartingSystem => {
            Some("SRM Open-loop ABZ: starting Viewer2000...".to_owned())
        }
        crate::app::state::SrmOpenLoopPhase::RequestingOpenLoop => {
            Some("SRM Open-loop ABZ: requesting powered run...".to_owned())
        }
        crate::app::state::SrmOpenLoopPhase::Running => {
            Some("SRM Open-loop ABZ: powered run active".to_owned())
        }
        crate::app::state::SrmOpenLoopPhase::StoppingSystem => {
            Some("SRM Open-loop ABZ: stopping Viewer2000...".to_owned())
        }
    }
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

fn voltage_text(value: Option<f64>) -> String {
    value.map_or("-".to_owned(), |value| format!("{value:.3} V"))
}

fn duty_text(value: Option<f64>) -> String {
    value.map_or("-".to_owned(), |value| format!("{:.2}%", value * 100.0))
}

fn hex_u16_text(value: Option<u16>) -> String {
    value.map_or("-".to_owned(), |value| format!("0x{value:04X}"))
}

fn hex_u32_text(value: Option<u32>) -> String {
    value.map_or("-".to_owned(), |value| format!("0x{value:08X}"))
}
