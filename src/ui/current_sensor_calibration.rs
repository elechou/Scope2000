use eframe::egui;

use crate::app::state::{
    CalibrationCommandResult, CalibrationGate, CalibrationState, UiState, applied_source_label,
    cal_result_label, cal_state_label, store_result_label,
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
    inspector: &InspectorState,
) -> Option<CurrentSensorCalibrationAction> {
    if !ui_state.show_current_sensor_calibration {
        return None;
    }

    let mut action = None;
    egui::Window::new("Current Sensor Calibration")
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
            show_actions(ui, calibration, gate, &mut action);
            ui.separator();
            show_summary(ui, calibration, gate, inspector);
            ui.separator();
            show_channel_table(ui, inspector);
        });

    if calibration.show_commit_confirmation {
        egui::Modal::new("current_sensor_calibration_commit_confirm".into()).show(ui.ctx(), |ui| {
            ui.set_width(360.0);
            ui.add_space(8.0);
            theme::modal_title(ui, "Commit Current Sensor Calibration?");
            ui.add_space(8.0);
            ui.label("Persist the current passing CT zero measurement to CPU2 flash.");
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
    inspector: &InspectorState,
) {
    let state = value_u16(inspector, "v2k_cal.state");
    let result = value_u16(inspector, "v2k_cal.result");
    let applied_src = value_u16(inspector, "v2k_cal.applied_src");
    let store_valid = value_u16(inspector, "v2k_cal.store_valid");
    let store_result = value_u16(inspector, "v2k_cal.store_result");
    let store_seq = value_u32(inspector, "v2k_cal.store_seq");
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
            summary_row(ui, "Result", cal_result_label(result), result_color(result));
            summary_row(
                ui,
                "Applied Source",
                applied_source_label(applied_src),
                theme::TEXT_DEFAULT,
            );
            summary_row(
                ui,
                "Stored Record",
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
                "Store Result",
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
    ui.label(egui::RichText::new("CT Raw Diagnostics").strong());
    ui.add_space(2.0);
    egui::Grid::new("current_sensor_calibration_channels")
        .num_columns(3)
        .spacing(egui::vec2(24.0, 4.0))
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Channel");
            ui.strong("Zero");
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
                    &format!("v2k_cal.noise_pp.ct{channel}"),
                ));
                ui.end_row();
            }
        });
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

fn value_u32(inspector: &InspectorState, name: &str) -> Option<u32> {
    inspector
        .value_by_name(name)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u32)
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
