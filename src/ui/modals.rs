use eframe::egui;

use crate::app::state::{HardwareState, UiState};
use crate::source::v2k::transport;
use crate::source::{command_result_text, fault_code_text};
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

/// Connection settings shared by the menu bar and status bar entry points.
/// Returns `true` when the user requests a connection.
pub fn show_connection_settings(
    ui: &egui::Ui,
    hardware: &mut HardwareState,
    ui_state: &mut UiState,
) -> bool {
    if ui_state.show_connection_settings && !hardware.can_configure_connection() {
        ui_state.show_connection_settings = false;
    }
    if !ui_state.show_connection_settings {
        return false;
    }

    let mut connect_clicked = false;
    egui::Modal::new("connection_settings_modal".into()).show(ui.ctx(), |ui| {
        ui.set_width(340.0);
        ui.add_space(8.0);
        theme::modal_title(ui, "Connection Settings");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Transport");
            ui.label(egui::RichText::new("Serial").color(theme::TEXT_SUBDUED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Refresh").clicked() {
                    refresh_serial_ports(hardware);
                }
            });
        });

        ui.label("Port");
        egui::ComboBox::from_id_salt("connection_settings_serial_port")
            .width(ui.available_width())
            .selected_text(if hardware.port.is_empty() {
                "Select serial port"
            } else {
                &hardware.port
            })
            .show_ui(ui, |ui| {
                for port in &hardware.serial_ports {
                    ui.selectable_value(&mut hardware.port, port.clone(), port);
                }
            });

        ui.label("Baud rate");
        egui::ComboBox::from_id_salt("connection_settings_baud")
            .width(ui.available_width())
            .selected_text(hardware.baud.to_string())
            .show_ui(ui, |ui| {
                for baud in [115_200, 230_400, 460_800, 921_600, 1_500_000, 3_125_000] {
                    ui.selectable_value(&mut hardware.baud, baud, baud.to_string());
                }
            });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!hardware.port.is_empty(), |ui| {
                if theme::modal_button(ui, "Connect", theme::GREEN) {
                    connect_clicked = true;
                }
            });
            if theme::modal_button(ui, "Close", theme::WIDGET_BG) {
                ui_state.show_connection_settings = false;
            }
        });
        ui.add_space(4.0);
    });

    if connect_clicked {
        ui_state.show_connection_settings = false;
    }
    connect_clicked
}

fn refresh_serial_ports(hardware: &mut HardwareState) {
    hardware.serial_ports = transport::available_serial_ports();
    if !hardware.port.is_empty() && !hardware.serial_ports.contains(&hardware.port) {
        hardware.serial_ports.insert(0, hardware.port.clone());
    }
}

/// Movable, non-modal Viewer2000 device information window.
pub fn show_device_info_window(
    ui: &egui::Ui,
    hardware: &HardwareState,
    descriptor_count: usize,
    ui_state: &mut UiState,
) {
    if !ui_state.show_device_info_window {
        return;
    }

    const WINDOW_MARGIN: f32 = 8.0;
    // The top and bottom panels have already been allocated, so this bottom
    // edge sits above the status bar and automatically tracks its height.
    let content_rect = ui.available_rect_before_wrap();
    let default_pos = egui::pos2(
        content_rect.right() - WINDOW_MARGIN,
        content_rect.bottom() - WINDOW_MARGIN,
    );

    egui::Window::new("Device Information")
        .id(egui::Id::new("device_info_window"))
        .open(&mut ui_state.show_device_info_window)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .default_pos(default_pos)
        .collapsible(false)
        .resizable(false)
        .movable(true)
        .show(ui.ctx(), |ui| {
            ui.set_min_width(380.0);
            if let Some(info) = &hardware.info {
                ui.monospace(format!("project {}", info.project_display_name()));
                ui.monospace(info.build_time_display_text());
                ui.monospace(format!("firmware {}", info.firmware_name));
                ui.monospace(format!("mcu {}", info.mcu_model_label()));
                ui.monospace(format!(
                    "wire {} contract {}",
                    info.protocol_version, info.contract_version
                ));
                ui.monospace(format!("build 0x{:08X}", info.build_hash));
                ui.monospace(format!("tick {} Hz", info.tick_hz));
                ui.monospace(format!(
                    "descriptors {descriptor_count}/{}",
                    info.descriptor_count
                ));
                ui.monospace(format!("capabilities 0x{:08X}", info.capabilities));
            } else {
                ui.monospace("No Viewer2000 session");
            }

            if let Some(status) = &hardware.status {
                ui.separator();
                ui.monospace(format!(
                    "state={}({}) fault={}({}) flags=0x{:04X}",
                    status.system_state,
                    status.system_state.wire_value(),
                    fault_code_text(status.fault_code),
                    status.fault_code,
                    status.status_flags
                ));
                ui.monospace(format!("tick={}", status.tick));
                ui.monospace(format!(
                    "hb={}/{}",
                    status.cpu1_heartbeat, status.cpu2_heartbeat
                ));
                ui.monospace(format!(
                    "cal seq={} result={} fail={}",
                    status.applied_seq, status.calibration_result, status.calibration_fail_index
                ));
                ui.monospace(format!(
                    "scope={} flags=0x{:02X}",
                    hardware.scope_mode_label(),
                    status.scope_flags,
                ));
                ui.monospace(format!(
                    "cmd ack={} result={}({})",
                    status.command_ack_seq.unwrap_or_default(),
                    command_result_text(status.command_result.unwrap_or_default()),
                    status.command_result.unwrap_or_default()
                ));
                if let Some(text) = hardware.pending_system_command_text() {
                    ui.monospace(text);
                }
                if let Some(text) = hardware.last_system_command_text() {
                    ui.monospace(text);
                }
            }
        });
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
