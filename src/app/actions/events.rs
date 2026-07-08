use std::{cmp::Ordering, time::Instant};

use crate::app::ScopeApp;
use crate::app::state::{AbzZeroingCommandResult, CalibrationCommandResult};
use crate::console::LogLevel;
use crate::source::{
    CatalogCommand, DeviceStatus, ScopeBlock, ScopeMode, SourceEvent, SystemCommand, SystemState,
    VarDescriptor, command_result_text,
};
use crate::variable::InspectorState;
use crate::wave::{
    AcquisitionSettings, WaveControlSettings, WaveState,
    data::{PlotData, relative_time_from_ticks},
};

impl ScopeApp {
    pub(in crate::app) fn poll_events(&mut self) {
        let events: Vec<_> = self.source.events.try_iter().collect();
        for event in events {
            match event {
                SourceEvent::Connected(info) => {
                    self.hardware.connected = true;
                    self.hardware.connecting = false;
                    self.hardware.clear_system_command_state();
                    self.descriptor_catalog_ready = false;
                    self.workspace_watch_restored = false;
                    self.log.push(
                        LogLevel::Info,
                        format!(
                            "HELLO: project={} built={} firmware={} mcu={} wire={} contract={} tick={}Hz caps=0x{:08X} scope={}ch/{}ticks/0x{:X}words",
                            info.project_display_name(),
                            info.build_time_local_text()
                                .unwrap_or_else(|| "not reported".to_owned()),
                            info.firmware_name,
                            info.mcu_model_label(),
                            info.protocol_version,
                            info.contract_version,
                            info.tick_hz,
                            info.capabilities,
                            info.scope_max_ch,
                            info.scope_block_ticks,
                            info.scope_ring_words
                        ),
                    );
                    self.hardware.info = Some(info);
                    self.hardware.device_summary = self.hardware.device_summary_text();
                    self.hardware.performance.set_available(true);
                    self.calibration.reset_session();
                    self.abz_zeroing.reset_session();
                    self.next_dc_voltage_read = Instant::now();
                    self.handle_firmware_project();
                }
                SourceEvent::Disconnected => {
                    self.hardware.connected = false;
                    self.hardware.connecting = false;
                    self.hardware.info = None;
                    self.hardware.status = None;
                    self.hardware.performance.clear();
                    self.hardware.clear_system_command_state();
                    self.calibration.reset_session();
                    self.abz_zeroing.reset_session();
                    self.next_dc_voltage_read = Instant::now();
                    self.wave.active = false;
                    self.wave.restart_pending = None;
                    self.descriptor_catalog_ready = false;
                    // Keep the last catalog and restored refs available for
                    // offline inspection/layout edits. Connected always resets
                    // both reconciliation gates before installing a new catalog.
                    self.handle_firmware_disconnect();
                    self.log.push(LogLevel::Info, "Disconnected".to_owned());
                }
                SourceEvent::Descriptors(descriptors) => {
                    self.log.push(
                        LogLevel::Info,
                        format!("Enumerated {} descriptor(s)", descriptors.len()),
                    );
                    // Descriptors is a complete catalog replacement. Snapshot
                    // the currently reconciled name refs before replacing it so
                    // a repeated catalog cannot silently drop variables.
                    if self.descriptor_catalog_ready {
                        self.workspace = self.snapshot_workspace();
                    }
                    self.workspace_watch_restored = false;
                    self.inspector.set_descriptors(descriptors);
                    self.descriptor_catalog_ready = true;
                    self.restore_workspace_watch_once();
                    self.request_watch_read();
                    self.next_dc_voltage_read = Instant::now();
                    self.calibration.next_read = Instant::now();
                    self.abz_zeroing.next_read = Instant::now();
                }
                SourceEvent::Status(status) => {
                    let now = Instant::now();
                    let previous_state = self
                        .hardware
                        .status
                        .as_ref()
                        .map(|previous| previous.system_state);
                    let previous_cpu1_heartbeat = self
                        .hardware
                        .status
                        .as_ref()
                        .map_or(0, |previous| previous.cpu1_heartbeat);
                    let previous_tick = self
                        .hardware
                        .status
                        .as_ref()
                        .map_or(0, |previous| previous.tick);
                    let cpu1_restarted = self
                        .hardware
                        .status
                        .as_ref()
                        .is_some_and(|previous| cpu1_status_restarted(previous, &status));
                    if cpu1_restarted {
                        self.hardware.clear_system_command_state();
                        self.calibration.reset_session();
                        self.abz_zeroing.reset_session();
                        self.wave.active = false;
                        self.wave.restart_pending = None;
                        self.log.push(
                            LogLevel::Warn,
                            format!(
                                "Viewer2000 CPU1 status restarted: hb {}/{} tick {}/{}; cleared pending host commands",
                                previous_cpu1_heartbeat, status.cpu1_heartbeat, previous_tick, status.tick
                            ),
                        );
                    }
                    let pending_system_start = self
                        .hardware
                        .pending_system_command
                        .as_ref()
                        .is_some_and(|pending| pending.command == SystemCommand::Start);
                    let completed_command = self.hardware.complete_pending_system_command(&status);
                    let completed_calibration =
                        self.calibration.complete_measure_from_status(&status);
                    let completed_abz_zeroing = self.abz_zeroing.complete_from_status(&status);
                    let stop_wave = should_stop_on_system_stop(
                        &self.wave.control,
                        previous_state,
                        status.system_state,
                        &self.wave,
                    );
                    let start_wave = should_capture_on_system_start(
                        &self.wave.control,
                        previous_state,
                        status.system_state,
                        pending_system_start,
                        &self.wave,
                    );
                    let entered_running =
                        system_started_transition(previous_state, status.system_state);
                    let performance = status.performance;
                    self.hardware.status = Some(status);
                    self.hardware.performance.ingest_status(performance);
                    if let Some(completed) = completed_command {
                        let result = command_result_text(completed.result);
                        let level = if completed.result == 0 {
                            LogLevel::Notice
                        } else {
                            LogLevel::Warn
                        };
                        self.log.push(
                            level,
                            format!(
                                "System command {} completed {result} as sequence {}",
                                completed.command.label(),
                                completed.sequence
                            ),
                        );
                        if completed.command == SystemCommand::Start && completed.result == 5 {
                            // CAL_FAILED: the firmware refused to start on an
                            // untrusted current-sensor zero.
                            self.log.push(
                                LogLevel::Warn,
                                "Start refused: Current Zeroing is not trusted.".to_owned(),
                            );
                        }
                        if completed.command == SystemCommand::Start && completed.result == 3 {
                            self.log.push(
                                LogLevel::Warn,
                                "Start refused: ABZ Zeroing is not ready.".to_owned(),
                            );
                            self.abz_zeroing.next_read = Instant::now();
                        }
                    }
                    if let Some(expired) = self.hardware.expire_pending_system_command(now) {
                        match expired.sequence {
                            Some(sequence) => self.log.push(
                                LogLevel::Warn,
                                format!(
                                    "System command {} pending seq {sequence} timed out; released host command state",
                                    expired.command.label()
                                ),
                            ),
                            None => self.log.push(
                                LogLevel::Warn,
                                format!(
                                    "System command {} send timed out; released host command state",
                                    expired.command.label()
                                ),
                            ),
                        }
                    }
                    if let Some(result) = completed_calibration {
                        log_calibration_result(&result, &mut self.log);
                        self.calibration.next_read = Instant::now();
                    }
                    if let Some(result) = completed_abz_zeroing {
                        log_abz_zeroing_result(&result, &mut self.log);
                        self.abz_zeroing.next_read = Instant::now();
                    }
                    if stop_wave {
                        self.stop_acquisition();
                        self.log.push(
                            LogLevel::Info,
                            "Wave stopped because user system stopped".to_owned(),
                        );
                    }
                    if start_wave {
                        self.start_acquisition(ScopeMode::CaptureArmed);
                    }
                    if entered_running {
                        self.request_watch_read();
                        self.log.push(
                            LogLevel::Debug,
                            "User variables reset on START; refreshing watched values".to_owned(),
                        );
                    }
                }
                SourceEvent::ParamsStaged => {
                    self.log
                        .push(LogLevel::Debug, "Parameter writes staged".to_owned());
                }
                SourceEvent::ParamsCommitted { sequence } => {
                    self.log.push(
                        LogLevel::Notice,
                        format!("Parameter commit sequence {sequence}"),
                    );
                    self.request_watch_read();
                }
                SourceEvent::Values {
                    read_sequence,
                    indexes,
                    values,
                } => {
                    self.inspector.update_values(&indexes, values);
                    self.log.push(
                        LogLevel::Debug,
                        format!("Value read sequence {read_sequence}"),
                    );
                }
                SourceEvent::ChannelsBound { bind_sequence } => {
                    self.wave.binding = std::mem::take(&mut self.wave.pending_binding);
                    self.wave.bind_sequence = Some(bind_sequence);
                    self.plot_data.ensure_series(&self.wave.binding);
                    self.log.push(
                        LogLevel::Info,
                        format!("Scope binding accepted as sequence {bind_sequence}"),
                    );
                }
                SourceEvent::SystemCommandAccepted { command, sequence } => {
                    if self
                        .hardware
                        .accept_system_command(command, sequence, Instant::now())
                    {
                        self.log.push(
                            LogLevel::Info,
                            format!(
                                "System command {} accepted as sequence {sequence}",
                                command.label()
                            ),
                        );
                    } else {
                        self.log.push(
                            LogLevel::Warn,
                            format!(
                                "Ignored system command {} ACK sequence {sequence} without matching pending command",
                                command.label()
                            ),
                        );
                    }
                }
                SourceEvent::CalibrationMeasureAccepted { sequence } => {
                    if self.calibration.accept_measure(sequence) {
                        self.log.push(
                            LogLevel::Info,
                            format!("Calibration Measure Zero accepted as sequence {sequence}"),
                        );
                    } else {
                        self.log.push(
                            LogLevel::Warn,
                            format!(
                                "Ignored calibration Measure Zero ACK sequence {sequence} without matching pending command"
                            ),
                        );
                    }
                }
                SourceEvent::CalibrationCommitCompleted { commit_sequence } => {
                    let result = self.calibration.complete_commit(commit_sequence);
                    log_calibration_result(&result, &mut self.log);
                    self.calibration.next_read = Instant::now();
                }
                SourceEvent::CalibrationCommandFailed { command, message } => {
                    let result = self.calibration.fail(command, message);
                    log_calibration_result(&result, &mut self.log);
                    self.calibration.next_read = Instant::now();
                }
                #[cfg(test)]
                SourceEvent::AbzZeroingAccepted { sequence } => {
                    if self.abz_zeroing.accept(sequence) {
                        self.log.push(
                            LogLevel::Info,
                            format!("ABZ Zeroing accepted as sequence {sequence}"),
                        );
                    } else {
                        self.log.push(
                            LogLevel::Warn,
                            format!(
                                "Ignored ABZ Zeroing ACK sequence {sequence} without matching pending command"
                            ),
                        );
                    }
                    self.abz_zeroing.next_read = Instant::now();
                }
                #[cfg(test)]
                SourceEvent::AbzZeroingCommandFailed { message } => {
                    let result = self.abz_zeroing.fail(message);
                    log_abz_zeroing_result(&result, &mut self.log);
                    self.abz_zeroing.next_read = Instant::now();
                }
                SourceEvent::ScopeConfigured { mode } => {
                    self.wave.mode = mode;
                    self.wave.active = mode != ScopeMode::Off;
                    self.log.push(
                        LogLevel::Info,
                        format!("Scope configured as {}", mode_label(mode)),
                    );
                    if mode == ScopeMode::Off
                        && let Some(restart_mode) = self.wave.restart_pending.take()
                    {
                        self.start_acquisition(restart_mode);
                    }
                    if should_force_capture(mode, &self.wave.settings_snapshot) {
                        self.send_catalog(CatalogCommand::ForceCapture);
                    }
                }
                SourceEvent::CaptureForceAccepted {
                    capture_state_sequence,
                } => {
                    self.log.push(
                        LogLevel::Debug,
                        format!(
                            "Auto Capture force accepted for state sequence {capture_state_sequence}"
                        ),
                    );
                }
                SourceEvent::CaptureForceFailed { message } => {
                    self.log.push(
                        LogLevel::Warn,
                        format!("Auto Capture force failed: {message}"),
                    );
                    self.stop_acquisition();
                }
                SourceEvent::CaptureFrame {
                    capture_id,
                    trigger_tick,
                    blocks,
                } => {
                    let tick_hz = self.hardware.info.as_ref().map_or(1, |info| info.tick_hz);
                    let mut frame_blocks = blocks;
                    redraw_capture_frame(
                        &mut self.plot_data,
                        &mut frame_blocks,
                        &self.wave.binding,
                        tick_hz,
                        &self.wave.settings_snapshot,
                        Some(trigger_tick),
                        &mut self.log,
                    );
                    self.rearm_capture(capture_id);
                }
                SourceEvent::Blocks { mode, blocks } => {
                    let tick_hz = self.hardware.info.as_ref().map_or(1, |info| info.tick_hz);
                    self.wave.mode = mode;
                    match mode {
                        ScopeMode::Stream => {
                            for block in blocks {
                                if !block_matches_binding(self.wave.bind_sequence, block.bind_seq) {
                                    self.log.push(
                                        LogLevel::Warn,
                                        format!(
                                            "Discarded block {} with stale bind sequence {}",
                                            block.block_seq, block.bind_seq
                                        ),
                                    );
                                    continue;
                                }
                                if let Err(error) = self.plot_data.append_block(
                                    &block,
                                    &self.wave.binding,
                                    tick_hz,
                                    self.wave.settings_snapshot.prescaler,
                                ) {
                                    self.log.push(LogLevel::Warn, error);
                                }
                            }
                        }
                        ScopeMode::CaptureArmed | ScopeMode::CapturePost => {}
                        ScopeMode::CaptureFrozen => {}
                        ScopeMode::Off | ScopeMode::Unknown(_) => {}
                    }
                }
                SourceEvent::StreamGap { expected, received } => {
                    if self.wave.mode == ScopeMode::Stream {
                        self.plot_data.append_gap(&self.wave.binding);
                    }
                    self.log.push(
                        LogLevel::Warn,
                        format!("Scope block gap: expected {expected}, received {received}"),
                    );
                }
                SourceEvent::PushFrameGap { expected, received } => {
                    self.log.push(
                        LogLevel::Warn,
                        format!("SCI push frame gap: expected {expected}, received {received}"),
                    );
                }
                SourceEvent::DeviceChanged { old_hash, info } => {
                    let new_hash = info.build_hash;
                    self.workspace = self.snapshot_workspace();
                    clear_device_session_state(
                        &mut self.wave,
                        &mut self.plot_data,
                        &mut self.inspector,
                    );
                    self.hardware.info = Some(info);
                    self.hardware.status = None;
                    self.hardware.performance.clear();
                    self.hardware.clear_system_command_state();
                    self.calibration.reset_session();
                    self.abz_zeroing.reset_session();
                    self.next_dc_voltage_read = Instant::now();
                    self.hardware.device_summary = self.hardware.device_summary_text();
                    self.workspace_watch_restored = false;
                    self.descriptor_catalog_ready = false;
                    self.handle_firmware_project();
                    self.log.push(
                        LogLevel::Warn,
                        format!(
                            "Firmware changed 0x{old_hash:08X} -> 0x{new_hash:08X}; re-enumerating"
                        ),
                    );
                }
                SourceEvent::Error(error) => {
                    self.hardware.connecting = false;
                    self.hardware.clear_system_command_state();
                    if let Some(result) = self.calibration.fail_pending(error.clone()) {
                        log_calibration_result(&result, &mut self.log);
                    }
                    if let Some(result) = self.abz_zeroing.fail_pending(error.clone()) {
                        log_abz_zeroing_result(&result, &mut self.log);
                    }
                    self.log.push(LogLevel::Error, error);
                }
                SourceEvent::Log(message) => self.log.push(LogLevel::Info, message),
            }
        }
    }
}

fn log_calibration_result(result: &CalibrationCommandResult, log: &mut crate::console::LogBuffer) {
    match result {
        CalibrationCommandResult::Measure { sequence, result } => {
            let text = command_result_text(*result);
            let level = if *result == 0 {
                LogLevel::Notice
            } else {
                LogLevel::Warn
            };
            log.push(
                level,
                format!("Calibration Measure Zero completed {text} as sequence {sequence}"),
            );
        }
        CalibrationCommandResult::Commit { commit_sequence } => {
            log.push(
                LogLevel::Notice,
                format!("Calibration Commit to Flash stored commit sequence {commit_sequence}"),
            );
        }
        CalibrationCommandResult::Failed { command, message } => {
            log.push(
                LogLevel::Warn,
                format!("Calibration {} failed: {message}", command.label()),
            );
        }
    }
}

fn log_abz_zeroing_result(result: &AbzZeroingCommandResult, log: &mut crate::console::LogBuffer) {
    match result {
        AbzZeroingCommandResult::Completed { sequence, result } => {
            let text = command_result_text(*result);
            let level = if *result == 0 {
                LogLevel::Notice
            } else {
                LogLevel::Warn
            };
            log.push(
                level,
                format!("ABZ Zeroing request completed {text} as sequence {sequence}"),
            );
        }
        AbzZeroingCommandResult::Failed { message } => {
            log.push(
                LogLevel::Warn,
                format!("ABZ Zeroing request failed: {message}"),
            );
        }
    }
}

fn block_matches_binding(bind_sequence: Option<u16>, block_bind_sequence: u16) -> bool {
    bind_sequence == Some(block_bind_sequence)
}

fn system_stopped_transition(
    previous_state: Option<SystemState>,
    current_state: SystemState,
) -> bool {
    previous_state.is_some_and(|previous| previous.is_running()) && !current_state.is_running()
}

fn system_started_transition(
    previous_state: Option<SystemState>,
    current_state: SystemState,
) -> bool {
    current_state.is_running() && !previous_state.is_some_and(|previous| previous.is_running())
}

fn should_capture_on_system_start(
    control: &WaveControlSettings,
    previous_state: Option<SystemState>,
    current_state: SystemState,
    pending_system_start: bool,
    wave: &WaveState,
) -> bool {
    control.capture_on_system_start
        && pending_system_start
        && system_started_transition(previous_state, current_state)
        && !wave_has_active_or_pending_stop_target(wave)
}

fn should_stop_on_system_stop(
    control: &WaveControlSettings,
    previous_state: Option<SystemState>,
    current_state: SystemState,
    wave: &WaveState,
) -> bool {
    control.stop_on_system_stop
        && system_stopped_transition(previous_state, current_state)
        && wave_has_active_or_pending_stop_target(wave)
}

fn wave_has_active_or_pending_stop_target(wave: &WaveState) -> bool {
    wave.active || wave.restart_pending.is_some() || !wave.pending_binding.is_empty()
}

fn cpu1_status_restarted(previous: &DeviceStatus, current: &DeviceStatus) -> bool {
    current.cpu1_heartbeat < previous.cpu1_heartbeat && current.tick < previous.tick
}

fn should_force_capture(mode: ScopeMode, settings: &AcquisitionSettings) -> bool {
    mode == ScopeMode::CaptureArmed && settings.trigger_source.is_none()
}

fn redraw_capture_frame(
    plot_data: &mut PlotData,
    blocks: &mut [ScopeBlock],
    binding: &[VarDescriptor],
    tick_hz: u32,
    settings: &AcquisitionSettings,
    trigger_tick: Option<u32>,
    log: &mut crate::console::LogBuffer,
) {
    sort_blocks_by_start_tick(blocks);
    plot_data.clear();
    if let Some(trigger_tick) = trigger_tick {
        plot_data.set_trigger_tick(trigger_tick);
    }
    plot_data.ensure_series(binding);
    if let Some(trigger_tick) = trigger_tick {
        match append_requested_capture_window(
            plot_data,
            blocks,
            binding,
            tick_hz,
            settings,
            trigger_tick,
        ) {
            Ok(count) => {
                log.push(
                    LogLevel::Info,
                    format!("Capture frame complete: {count} sample(s)"),
                );
                return;
            }
            Err(error) => log.push(LogLevel::Warn, error),
        }
    }
    for block in blocks.iter() {
        if let Err(error) = plot_data.append_block(block, binding, tick_hz, settings.prescaler) {
            log.push(LogLevel::Warn, error);
        }
    }
    log.push(
        LogLevel::Info,
        format!("Capture frame complete: {} block(s)", blocks.len()),
    );
}

fn sort_blocks_by_start_tick(blocks: &mut [ScopeBlock]) {
    blocks.sort_by(|left, right| compare_wrapped_tick(left.start_tick, right.start_tick));
}

struct CaptureRow {
    tick: u32,
    values: Vec<f64>,
}

fn append_requested_capture_window(
    plot_data: &mut PlotData,
    blocks: &[ScopeBlock],
    binding: &[VarDescriptor],
    tick_hz: u32,
    settings: &AcquisitionSettings,
    trigger_tick: u32,
) -> Result<usize, String> {
    let rows = decode_capture_rows(blocks, binding, settings.prescaler)?;
    let Some(trigger_index) = rows.iter().position(|row| row.tick == trigger_tick) else {
        return Err("capture trigger tick was not present in drained blocks".to_owned());
    };
    let requested = usize::from(settings.record_points);
    if requested == 0 {
        return Err("capture record point count is zero".to_owned());
    }
    let pre = capture_pre_samples(requested, settings.pre_trigger_percent);
    if trigger_index < pre {
        return Err(format!(
            "capture frame has only {trigger_index} pre-trigger sample(s), expected {pre}"
        ));
    }
    let start = trigger_index - pre;
    let end = start.saturating_add(requested);
    if end > rows.len() {
        return Err(format!(
            "capture frame has {} sample(s) after trimming start, expected {requested}",
            rows.len().saturating_sub(start)
        ));
    }

    for row in &rows[start..end] {
        let time = relative_time_from_ticks(row.tick, trigger_tick, tick_hz);
        for (descriptor, value) in binding.iter().zip(row.values.iter()) {
            plot_data.push(&descriptor.name, time, *value);
        }
    }
    Ok(requested)
}

fn capture_pre_samples(record_points: usize, pre_trigger_percent: u8) -> usize {
    let pre = record_points * usize::from(pre_trigger_percent.min(100)) / 100;
    let post = record_points.saturating_sub(pre);
    if post == 0 {
        record_points.saturating_sub(1)
    } else {
        pre
    }
}

fn decode_capture_rows(
    blocks: &[ScopeBlock],
    binding: &[VarDescriptor],
    prescaler: u16,
) -> Result<Vec<CaptureRow>, String> {
    let expected_stride: usize = binding
        .iter()
        .map(|descriptor| descriptor.var.ty.wire_width())
        .sum();
    let tick_step = u32::from(prescaler.max(1));
    let mut rows = Vec::new();
    for block in blocks {
        if binding.len() != usize::from(block.channel_count) {
            return Err("block channel count does not match active binding".to_owned());
        }
        if expected_stride != usize::from(block.stride_octets) {
            return Err("block stride does not match active binding".to_owned());
        }
        if block.samples.len() != expected_stride * usize::from(block.sample_count) {
            return Err("block payload length is invalid".to_owned());
        }

        for sample_index in 0..usize::from(block.sample_count) {
            let mut offset = sample_index * expected_stride;
            let tick = block
                .start_tick
                .wrapping_add((sample_index as u32).wrapping_mul(tick_step));
            let mut values = Vec::with_capacity(binding.len());
            for descriptor in binding {
                let width = descriptor.var.ty.wire_width();
                let value = descriptor
                    .var
                    .ty
                    .decode(&block.samples[offset..offset + width])
                    .ok_or_else(|| "sample type decode failed".to_owned())?;
                values.push(value);
                offset += width;
            }
            rows.push(CaptureRow { tick, values });
        }
    }
    Ok(rows)
}

fn compare_wrapped_tick(left: u32, right: u32) -> Ordering {
    (left.wrapping_sub(right) as i32).cmp(&0)
}

fn clear_device_session_state(
    wave: &mut WaveState,
    plot_data: &mut PlotData,
    inspector: &mut InspectorState,
) {
    wave.clear_binding();
    plot_data.clear();
    inspector.clear();
}

fn mode_label(mode: ScopeMode) -> &'static str {
    match mode {
        ScopeMode::Off => "off",
        ScopeMode::Stream => "stream",
        ScopeMode::CaptureArmed => "capture armed",
        ScopeMode::CapturePost => "capture post",
        ScopeMode::CaptureFrozen => "capture frozen",
        ScopeMode::Unknown(_) => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{VarDescriptor, VarRef, VarType};

    fn descriptor(name: &str) -> VarDescriptor {
        VarDescriptor {
            name: name.to_owned(),
            var: VarRef {
                addr: 0,
                ty: VarType::F32,
            },
            kind: 0x0002,
            prescaler: 1,
        }
    }

    fn f32_block(start_tick: u32, block_seq: u16, bind_seq: u16, value: f32) -> ScopeBlock {
        ScopeBlock {
            start_tick,
            block_seq,
            flags: 0,
            sample_count: 1,
            channel_count: 1,
            bind_seq,
            stride_octets: 4,
            samples: value.to_le_bytes().to_vec(),
        }
    }

    fn status(tick: u32, cpu1_heartbeat: u32) -> DeviceStatus {
        DeviceStatus {
            system_state: SystemState::Idle,
            fault_code: 0,
            status_flags: 0,
            tick,
            cpu1_heartbeat,
            cpu2_heartbeat: 0,
            applied_seq: 0,
            calibration_result: 0,
            calibration_fail_index: 0,
            build_hash: 0,
            scope_mode: ScopeMode::Off,
            scope_flags: 0,
            command_ack_seq: Some(0),
            command_result: Some(0),
            performance: None,
            scope_state_seq: 0,
            scope_frozen_count: 0,
            scope_trigger_tick: 0,
            scope_bind_seq: 0,
        }
    }

    fn f32_block_samples(
        start_tick: u32,
        block_seq: u16,
        bind_seq: u16,
        values: &[f32],
    ) -> ScopeBlock {
        let mut samples = Vec::new();
        for value in values {
            samples.extend_from_slice(&value.to_le_bytes());
        }
        ScopeBlock {
            start_tick,
            block_seq,
            flags: 0,
            sample_count: values.len() as u16,
            channel_count: 1,
            bind_seq,
            stride_octets: 4,
            samples,
        }
    }

    #[test]
    fn stale_bind_sequence_is_discarded() {
        assert!(block_matches_binding(Some(7), 7));
        assert!(!block_matches_binding(Some(7), 6));
        assert!(!block_matches_binding(None, 7));
    }

    #[test]
    fn system_stopped_transition_only_fires_when_leaving_running() {
        assert!(system_stopped_transition(
            Some(SystemState::Running),
            SystemState::Idle
        ));
        assert!(system_stopped_transition(
            Some(SystemState::Running),
            SystemState::Fault
        ));
        assert!(!system_stopped_transition(
            Some(SystemState::Running),
            SystemState::Running
        ));
        assert!(!system_stopped_transition(
            Some(SystemState::Idle),
            SystemState::Idle
        ));
        assert!(!system_stopped_transition(None, SystemState::Idle));
    }

    #[test]
    fn capture_follow_system_start_requires_setting_pending_start_and_idle_wave() {
        let control = WaveControlSettings::default();
        assert!(should_capture_on_system_start(
            &control,
            Some(SystemState::Idle),
            SystemState::Running,
            true,
            &WaveState::default()
        ));

        assert!(!should_capture_on_system_start(
            &control,
            Some(SystemState::Idle),
            SystemState::Running,
            false,
            &WaveState::default()
        ));

        let disabled = WaveControlSettings {
            capture_on_system_start: false,
            ..WaveControlSettings::default()
        };
        assert!(!should_capture_on_system_start(
            &disabled,
            Some(SystemState::Idle),
            SystemState::Running,
            true,
            &WaveState::default()
        ));

        let active = WaveState {
            active: true,
            ..WaveState::default()
        };
        assert!(!should_capture_on_system_start(
            &control,
            Some(SystemState::Idle),
            SystemState::Running,
            true,
            &active
        ));
    }

    #[test]
    fn stop_follow_system_stop_requires_setting_and_active_or_pending_wave() {
        let control = WaveControlSettings::default();
        let active = WaveState {
            active: true,
            ..WaveState::default()
        };
        assert!(should_stop_on_system_stop(
            &control,
            Some(SystemState::Running),
            SystemState::Idle,
            &active
        ));

        let disabled = WaveControlSettings {
            stop_on_system_stop: false,
            ..WaveControlSettings::default()
        };
        assert!(!should_stop_on_system_stop(
            &disabled,
            Some(SystemState::Running),
            SystemState::Idle,
            &active
        ));
        assert!(!should_stop_on_system_stop(
            &control,
            Some(SystemState::Running),
            SystemState::Idle,
            &WaveState::default()
        ));
    }

    #[test]
    fn cpu1_status_restart_requires_heartbeat_and_tick_rollback() {
        let previous = status(20_000, 100_000);

        assert!(cpu1_status_restarted(&previous, &status(100, 500)));
        assert!(!cpu1_status_restarted(&previous, &status(20_100, 500)));
        assert!(!cpu1_status_restarted(&previous, &status(100, 100_100)));
        assert!(!cpu1_status_restarted(&previous, &status(20_100, 100_100)));
    }

    #[test]
    fn wave_stop_target_includes_active_restart_and_pending_start() {
        let descriptor = descriptor("signal");
        assert!(!wave_has_active_or_pending_stop_target(
            &WaveState::default()
        ));

        let mut active = WaveState {
            active: true,
            ..WaveState::default()
        };
        assert!(wave_has_active_or_pending_stop_target(&active));

        active.active = false;
        active.restart_pending = Some(ScopeMode::CaptureArmed);
        assert!(wave_has_active_or_pending_stop_target(&active));

        active.restart_pending = None;
        active.pending_binding = vec![descriptor];
        assert!(wave_has_active_or_pending_stop_target(&active));
    }

    #[test]
    fn capture_blocks_sort_by_wrapped_tick() {
        let mut blocks = vec![
            f32_block(10, 2, 1, 2.0),
            f32_block(u32::MAX - 5, 1, 1, 1.0),
            f32_block(20, 3, 1, 3.0),
        ];

        sort_blocks_by_start_tick(&mut blocks);

        assert_eq!(
            blocks
                .iter()
                .map(|block| block.start_tick)
                .collect::<Vec<_>>(),
            vec![u32::MAX - 5, 10, 20]
        );
    }

    #[test]
    fn auto_capture_forces_every_armed_generation_but_explicit_trigger_does_not() {
        let auto = AcquisitionSettings::default();
        assert!(should_force_capture(ScopeMode::CaptureArmed, &auto));
        assert!(should_force_capture(ScopeMode::CaptureArmed, &auto));
        assert!(!should_force_capture(ScopeMode::CapturePost, &auto));

        let mut explicit = auto;
        explicit.trigger_source = Some("speed".to_owned());
        assert!(!should_force_capture(ScopeMode::CaptureArmed, &explicit));
    }

    #[test]
    fn capture_frame_redraw_replaces_existing_plot_data() {
        let descriptor = descriptor("signal");
        let binding = vec![descriptor.clone()];
        let mut plot_data = PlotData::new(100);
        plot_data.push(&descriptor.name, 99.0, 9.0);
        let mut blocks = vec![f32_block(20, 2, 3, 2.0), f32_block(10, 1, 3, 1.0)];
        let mut log = crate::console::LogBuffer::default();

        redraw_capture_frame(
            &mut plot_data,
            &mut blocks,
            &binding,
            10,
            &AcquisitionSettings::default(),
            Some(15),
            &mut log,
        );

        let series = &plot_data.series["signal"];
        assert_eq!(plot_data.trigger_time, Some(0.0));
        assert_eq!(
            series.times.iter().copied().collect::<Vec<_>>(),
            vec![-0.5, 0.5]
        );
        assert_eq!(
            series.values.iter().copied().collect::<Vec<_>>(),
            vec![1.0, 2.0]
        );
    }

    #[test]
    fn capture_frame_redraw_trims_to_requested_points_around_trigger() {
        let descriptor = descriptor("signal");
        let binding = vec![descriptor.clone()];
        let mut plot_data = PlotData::new(100);
        let mut blocks = vec![
            f32_block_samples(0, 1, 3, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]),
            f32_block_samples(
                10,
                2,
                3,
                &[10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0],
            ),
            f32_block_samples(20, 3, 3, &[20.0, 21.0, 22.0, 23.0, 24.0]),
        ];
        let settings = AcquisitionSettings {
            record_points: 20,
            pre_trigger_percent: 50,
            ..AcquisitionSettings::default()
        };
        let mut log = crate::console::LogBuffer::default();

        redraw_capture_frame(
            &mut plot_data,
            &mut blocks,
            &binding,
            1,
            &settings,
            Some(15),
            &mut log,
        );

        let series = &plot_data.series["signal"];
        assert_eq!(series.times.len(), 20);
        assert_eq!(series.times.front().copied(), Some(-10.0));
        assert_eq!(series.times.back().copied(), Some(9.0));
        assert_eq!(series.values.front().copied(), Some(5.0));
        assert_eq!(series.values.back().copied(), Some(24.0));
    }

    #[test]
    fn device_change_clears_descriptors_bindings_and_plot_data() {
        let descriptor = descriptor("motor.speed");
        let mut wave = WaveState {
            active: true,
            binding: vec![descriptor.clone()],
            pending_binding: vec![descriptor.clone()],
            bind_sequence: Some(3),
            ..WaveState::default()
        };
        let mut plot_data = PlotData::new(100);
        plot_data.push(&descriptor.name, 1.0, 2.0);
        plot_data.set_trigger_tick(10);
        let mut inspector = InspectorState::default();
        inspector.set_descriptors(vec![descriptor]);
        inspector.pinned.push(0);

        clear_device_session_state(&mut wave, &mut plot_data, &mut inspector);

        assert!(!wave.active);
        assert!(wave.binding.is_empty());
        assert!(wave.pending_binding.is_empty());
        assert_eq!(wave.bind_sequence, None);
        assert!(plot_data.series.is_empty());
        assert_eq!(plot_data.trigger_time, None);
        assert!(inspector.descriptors.is_empty());
        assert!(inspector.pinned.is_empty());
    }
}
