use eframe::egui;

use crate::app::state::{
    CalibrationCommandResult, CalibrationGate, CalibrationHealthLevel, CalibrationSnapshot,
    CalibrationState, UiState, applied_source_label, cal_result_label, cal_state_label,
    store_result_label,
};
use crate::source::command_result_text;
use crate::theme;
use crate::variable::InspectorState;

pub enum CurrentSensorCalibrationAction {
    MeasureZero,
    CommitToFlash,
}

pub fn show(
    ui: &egui::Ui,
    ui_state: &mut UiState,
    calibration: &mut CalibrationState,
    gate: CalibrationGate,
    snapshot: CalibrationSnapshot,
    inspector: &InspectorState,
) -> Option<CurrentSensorCalibrationAction> {
    if !ui_state.show_current_sensor_calibration {
        return None;
    }

    let mut action = None;
    egui::Window::new("Current Zeroing")
        .id(egui::Id::new("current_sensor_calibration_window"))
        .open(&mut ui_state.show_current_sensor_calibration)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ui.ctx().content_rect().center())
        .collapsible(false)
        .resizable(true)
        .default_width(540.0)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(500.0);
            ui.add_space(2.0);
            show_service_notice(ui, snapshot);
            ui.separator();
            show_actions(ui, calibration, gate, &mut action);
            ui.separator();
            show_summary(ui, calibration, gate, snapshot, inspector);
            ui.separator();
            show_channel_table(ui, inspector);
        });

    if calibration.show_commit_confirmation {
        egui::Modal::new("current_sensor_calibration_commit_confirm".into()).show(ui.ctx(), |ui| {
            ui.set_width(360.0);
            ui.add_space(8.0);
            theme::modal_title(ui, "Commit Current Zeroing Reference?");
            ui.add_space(8.0);
            ui.label(
                "Write the latest passing CT zero measurement as the system Golden reference in CPU2 flash.",
            );
            ui.label(
                egui::RichText::new(
                    "Normal startup calibration does not require this operation.",
                )
                .color(theme::TEXT_SUBDUED),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if theme::modal_button(ui, "Commit", theme::YELLOW) {
                    if gate.can_commit {
                        action = Some(CurrentSensorCalibrationAction::CommitToFlash);
                    }
                    calibration.show_commit_confirmation = false;
                }
                if theme::modal_button(ui, "Cancel", theme::WIDGET_BG) {
                    calibration.show_commit_confirmation = false;
                }
            });
            ui.add_space(4.0);
        });
    }

    action
}

fn show_service_notice(ui: &mut egui::Ui, snapshot: CalibrationSnapshot) {
    ui.label(
        egui::RichText::new("Golden Reference Service")
            .strong()
            .color(theme::TEXT_STRONG),
    );
    ui.label(
        "Viewer2000 automatically runs Current Zeroing at every boot. Use this one-time \
         commissioning and service workflow only to establish or update the system Golden \
         reference in flash after initial setup or a significant hardware change.",
    );

    let health = snapshot.health();
    if health.level != CalibrationHealthLevel::Normal {
        ui.add_space(4.0);
        ui.colored_label(
            health_color(health.level),
            format!("⚠ {}: {}", health.label, health.detail),
        );
    }
}

fn show_actions(
    ui: &mut egui::Ui,
    calibration: &mut CalibrationState,
    gate: CalibrationGate,
    action: &mut Option<CurrentSensorCalibrationAction>,
) {
    let button_gap = ui.spacing().item_spacing.x;
    let button_w = ((ui.available_width() - button_gap) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        if theme::action_button_w(ui, "Measure Zero", theme::GREEN, gate.can_measure, button_w) {
            *action = Some(CurrentSensorCalibrationAction::MeasureZero);
        }
        if theme::action_button_w(
            ui,
            "Commit to Flash",
            theme::YELLOW,
            gate.can_commit,
            button_w,
        ) {
            calibration.show_commit_confirmation = true;
        }
    });

    if let Some(pending) = calibration.pending {
        let text = match pending.sequence {
            Some(sequence) => format!("{} pending seq {sequence}", pending.command.label()),
            None => format!("{} sending...", pending.command.label()),
        };
        ui.colored_label(theme::YELLOW, text);
    } else if let Some(reason) = gate.reason {
        ui.colored_label(theme::TEXT_SUBDUED, reason);
    } else if !gate.can_commit {
        ui.colored_label(theme::TEXT_SUBDUED, "Commit requires DONE / OK measurement");
    }
}

fn show_summary(
    ui: &mut egui::Ui,
    calibration: &CalibrationState,
    gate: CalibrationGate,
    snapshot: CalibrationSnapshot,
    inspector: &InspectorState,
) {
    let state = snapshot.state;
    let result = snapshot.result;
    let applied_src = snapshot.applied_source;
    let store_valid = snapshot.store_valid;
    let store_result = snapshot.store_result;
    let store_seq = snapshot.store_sequence;
    let settle_max = value_u16(inspector, "v2k_cal.settle_max");
    let settle_ch = value_u16(inspector, "v2k_cal.settle_ch");

    egui::Grid::new("current_sensor_calibration_summary")
        .num_columns(2)
        .spacing(egui::vec2(18.0, 4.0))
        .striped(true)
        .show(ui, |ui| {
            summary_row(
                ui,
                "Measurement",
                cal_state_label(state),
                state_color(state),
            );
            summary_row(
                ui,
                "Measurement Result",
                cal_result_label(result),
                result_color(result),
            );
            summary_row(
                ui,
                "Applied Source",
                applied_source_label(applied_src),
                if result == Some(1) && applied_src == Some(1) {
                    theme::RED
                } else {
                    theme::TEXT_DEFAULT
                },
            );
            summary_row(
                ui,
                "Golden Record",
                match store_valid {
                    Some(1) => format!(
                        "valid seq {}",
                        store_seq.map_or("-".to_owned(), |v| v.to_string())
                    ),
                    Some(0) => "none".to_owned(),
                    Some(other) => format!("valid={other}"),
                    None => "unknown".to_owned(),
                },
                if store_valid == Some(1) {
                    theme::GREEN
                } else {
                    theme::TEXT_SUBDUED
                },
            );
            summary_row(
                ui,
                "Flash Result",
                store_result_text(inspector, store_result),
                store_result_color(inspector, store_result),
            );
            summary_row(
                ui,
                "Settle Delta",
                format!(
                    "max {} ch {}",
                    settle_max.map_or("-".to_owned(), |v| v.to_string()),
                    settle_ch.map_or("-".to_owned(), |v| v.to_string())
                ),
                theme::TEXT_DEFAULT,
            );
            if let Some(text) = last_result_text(calibration) {
                summary_row(
                    ui,
                    "Last Command",
                    text,
                    if gate.can_measure {
                        theme::TEXT_DEFAULT
                    } else {
                        theme::TEXT_SUBDUED
                    },
                );
            }
        });
}

fn show_channel_table(ui: &mut egui::Ui, inspector: &InspectorState) {
    ui.label(egui::RichText::new("CT Offset Diagnostics").strong());
    ui.add_space(2.0);
    egui::Grid::new("current_sensor_calibration_channels")
        .num_columns(4)
        .spacing(egui::vec2(24.0, 4.0))
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Channel");
            ui.strong("Measured");
            ui.strong("Golden (CPU2 Flash)");
            ui.strong("Noise p-p");
            ui.end_row();
            for channel in 1..=6 {
                ui.monospace(format!("CT{channel}"));
                ui.monospace(value_u16_text(
                    inspector,
                    &format!("v2k_cal.zero_meas.ct{channel}"),
                ));
                ui.monospace(value_u16_text(
                    inspector,
                    &format!("v2k_cal.zero_stored.ct{channel}"),
                ));
                ui.monospace(value_u16_text(
                    inspector,
                    &format!("v2k_cal.noise_pp.ct{channel}"),
                ));
                ui.end_row();
            }
        });

    if inspector.index_by_name("v2k_cal.zero_stored.ct1").is_none() {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Stored Golden offsets are not exposed by this Viewer2000 catalog.",
            )
            .color(theme::TEXT_SUBDUED),
        );
    }
}

fn summary_row(ui: &mut egui::Ui, label: &str, value: String, color: egui::Color32) {
    ui.label(egui::RichText::new(label).color(theme::TEXT_SUBDUED));
    ui.colored_label(color, value);
    ui.end_row();
}

fn value_u16(inspector: &InspectorState, name: &str) -> Option<u16> {
    inspector
        .value_by_name(name)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u16)
}

fn value_u16_text(inspector: &InspectorState, name: &str) -> String {
    value_u16(inspector, name).map_or("-".to_owned(), |value| value.to_string())
}

fn state_color(value: Option<u16>) -> egui::Color32 {
    match value {
        Some(1) => theme::YELLOW,
        Some(2) => theme::GREEN,
        _ => theme::TEXT_DEFAULT,
    }
}

fn result_color(value: Option<u16>) -> egui::Color32 {
    match value {
        Some(1) => theme::GREEN,
        Some(0) | None => theme::TEXT_SUBDUED,
        _ => theme::RED,
    }
}

fn health_color(level: CalibrationHealthLevel) -> egui::Color32 {
    match level {
        CalibrationHealthLevel::Normal => theme::TEXT_SUBDUED,
        CalibrationHealthLevel::Warning => theme::YELLOW,
        CalibrationHealthLevel::Error => theme::RED,
    }
}

fn store_result_text(inspector: &InspectorState, value: Option<u16>) -> String {
    if inspector.index_by_name("v2k_cal.store_result").is_none() {
        "UNAVAILABLE".to_owned()
    } else {
        store_result_label(value)
    }
}

fn store_result_color(inspector: &InspectorState, value: Option<u16>) -> egui::Color32 {
    if inspector.index_by_name("v2k_cal.store_result").is_none() {
        return theme::TEXT_SUBDUED;
    }
    match value {
        Some(1) => theme::GREEN,
        Some(0) | None => theme::TEXT_SUBDUED,
        Some(_) => theme::RED,
    }
}

fn last_result_text(calibration: &CalibrationState) -> Option<String> {
    match calibration.last_result.as_ref()? {
        CalibrationCommandResult::Measure { sequence, result } => Some(format!(
            "Measure Zero {} (seq {sequence})",
            command_result_text(*result)
        )),
        CalibrationCommandResult::Commit { commit_sequence } => {
            Some(format!("Commit to Flash OK (seq {commit_sequence})"))
        }
        CalibrationCommandResult::Failed { command, message } => {
            Some(format!("{} failed: {message}", command.label()))
        }
    }
}
