use std::{cmp::Ordering, time::Instant};

use crate::app::ScopeApp;
use crate::console::LogLevel;
use crate::source::{ScopeBlock, ScopeMode, SourceEvent, VarDescriptor, command_result_text};
use crate::variable::InspectorState;
use crate::wave::{AcquisitionSettings, WaveState, data::PlotData};

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
                    self.hardware.version = self.hardware.version_text();
                    self.hardware.performance.set_available(true);
                    self.handle_firmware_project();
                }
                SourceEvent::Disconnected => {
                    self.hardware.connected = false;
                    self.hardware.connecting = false;
                    self.hardware.info = None;
                    self.hardware.status = None;
                    self.hardware.performance.clear();
                    self.hardware.clear_system_command_state();
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
                    self.next_watch_read = Instant::now();
                }
                SourceEvent::Status(status) => {
                    let completed_command = self.hardware.complete_pending_system_command(&status);
                    let previous_state = self
                        .hardware
                        .status
                        .as_ref()
                        .map(|previous| previous.system_state);
                    let entered_running = status.system_state.is_running()
                        && !previous_state.is_some_and(|previous| previous.is_running());
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
                    }
                    if entered_running {
                        self.next_watch_read = Instant::now();
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
                    self.next_watch_read = Instant::now();
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
                    if self.hardware.accept_system_command(command, sequence) {
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
                    self.hardware.version = self.hardware.version_text();
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
                    self.log.push(LogLevel::Error, error);
                }
                SourceEvent::Log(message) => self.log.push(LogLevel::Info, message),
            }
        }
    }
}

fn block_matches_binding(bind_sequence: Option<u16>, block_bind_sequence: u16) -> bool {
    bind_sequence == Some(block_bind_sequence)
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
        plot_data.set_trigger_tick(trigger_tick, tick_hz);
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

    let tick_hz = f64::from(tick_hz.max(1));
    for row in &rows[start..end] {
        let time = f64::from(row.tick) / tick_hz;
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
        assert_eq!(plot_data.trigger_time, Some(1.5));
        assert_eq!(
            series.times.iter().copied().collect::<Vec<_>>(),
            vec![1.0, 2.0]
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
        assert_eq!(series.times.front().copied(), Some(5.0));
        assert_eq!(series.times.back().copied(), Some(24.0));
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
        plot_data.set_trigger_tick(10, 10);
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
