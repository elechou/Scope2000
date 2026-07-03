use eframe::egui;

use crate::app::state::{CalibrationHealthLevel, CalibrationSnapshot, HardwareState, UiState};
use crate::console::{LogBuffer, LogLevel};
use crate::theme;

pub enum StatusBarAction {
    Connect,
    CancelConnect,
    Disconnect,
}

/// Show the bottom status bar.
pub fn show(
    ui: &mut egui::Ui,
    hardware: &mut HardwareState,
    ui_state: &mut UiState,
    log: &mut LogBuffer,
    calibration: CalibrationSnapshot,
) -> Option<StatusBarAction> {
    // Auto-dismiss promoted status messages after 5 seconds.
    if let Some(ref msg) = log.status_message
        && matches!(
            msg.level,
            LogLevel::Notice | LogLevel::Warn | LogLevel::Error
        )
        && msg.timestamp.elapsed() > std::time::Duration::from_secs(5)
    {
        log.status_message = None;
    }

    let mut connect_clicked = false;
    let mut cancel_connect_clicked = false;
    let mut disconnect_clicked = false;
    let mut dismiss_status = false;
    egui::Panel::bottom("status_bar")
        .frame(theme::status_bar_frame())
        .show_separator_line(false)
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                // Plug icon button (connect/disconnect) -- disabled while connecting
                let is_connecting = hardware.connecting;
                let tint = if hardware.connected {
                    theme::GREEN
                } else if is_connecting {
                    theme::YELLOW
                } else {
                    theme::TEXT_SUBDUED
                };
                let plug_btn = ui.add(egui::Button::image(
                    egui::Image::new(theme::ICON_PLUG)
                        .fit_to_exact_size(egui::vec2(16.0, 16.0))
                        .tint(tint),
                ));
                if plug_btn.clicked() {
                    if hardware.connected {
                        if hardware.is_running() {
                            ui_state.stop_warning_action = Some("Disconnect");
                        } else {
                            disconnect_clicked = true;
                        }
                    } else if is_connecting {
                        cancel_connect_clicked = true;
                    } else {
                        connect_clicked = true;
                    }
                }
                let hover = if hardware.connected {
                    "Disconnect"
                } else if is_connecting {
                    "Cancel"
                } else {
                    "Connect"
                };
                plug_btn.on_hover_text(hover);

                let endpoint_color = if hardware.connected {
                    theme::GREEN
                } else if hardware.connecting {
                    theme::YELLOW
                } else {
                    theme::TEXT_SUBDUED
                };
                let endpoint_text = egui::RichText::new(hardware.endpoint_label())
                    .color(endpoint_color)
                    .monospace();
                let connection_settings = if hardware.can_configure_connection() {
                    ui.add(egui::Button::new(endpoint_text).frame(false))
                        .on_hover_text("Connection settings")
                } else {
                    ui.label(endpoint_text)
                };
                if connection_settings.clicked() {
                    ui_state.show_connection_settings = true;
                }

                if hardware.connecting {
                    ui.spinner();
                    ui.label(egui::RichText::new("Connecting").color(theme::YELLOW));
                }

                ui.separator();

                let calibration_health = calibration.health();
                let calibration_color = match calibration_health.level {
                    CalibrationHealthLevel::Normal => theme::TEXT_SUBDUED,
                    CalibrationHealthLevel::Warning => theme::YELLOW,
                    CalibrationHealthLevel::Error => theme::RED,
                };
                let calibration_text = if calibration_health.level == CalibrationHealthLevel::Normal
                {
                    "Current Sensor"
                } else {
                    "⚠ Current Sensor"
                };
                let calibration_status = ui.add(
                    egui::Button::new(
                        egui::RichText::new(calibration_text).color(calibration_color),
                    )
                    .frame(false),
                );
                if calibration_status.clicked() {
                    ui_state.show_device_info_window = true;
                }
                calibration_status.on_hover_text(format!(
                    "{}\n{}",
                    calibration_health.label, calibration_health.detail
                ));

                ui.separator();

                // Status message (latest Warn/Error from console)
                if let Some(ref msg) = log.status_message {
                    let color = match msg.level {
                        LogLevel::Error => theme::RED,
                        LogLevel::Warn => theme::YELLOW,
                        LogLevel::Notice => theme::GREEN,
                        LogLevel::Info => theme::GREEN,
                        _ => theme::TEXT_DEFAULT,
                    };
                    if ui
                        .colored_label(color, &msg.text)
                        .on_hover_text("Click to dismiss")
                        .clicked()
                    {
                        dismiss_status = true;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(ref info) = hardware.device_summary {
                        let device_info = ui.add(
                            egui::Button::new(egui::RichText::new(info).color(theme::TEXT_SUBDUED))
                                .frame(false),
                        );
                        if device_info.clicked() {
                            ui_state.show_device_info_window = !ui_state.show_device_info_window;
                        }
                        let hover = hardware
                            .device_info_hover_text()
                            .unwrap_or_else(|| "Device Information".to_owned());
                        device_info.on_hover_text(hover);
                        ui.separator();
                    }
                });
            });
        });
    if dismiss_status {
        log.status_message = None;
    }
    if connect_clicked {
        Some(StatusBarAction::Connect)
    } else if cancel_connect_clicked {
        Some(StatusBarAction::CancelConnect)
    } else if disconnect_clicked {
        Some(StatusBarAction::Disconnect)
    } else {
        None
    }
}
