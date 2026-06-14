use std::time::Instant;

use crate::app::ScopeApp;
use crate::console::LogLevel;
use crate::source::{ScopeMode, SourceEvent};
use crate::variable::InspectorState;
use crate::wave::{WaveState, data::PlotData};

impl ScopeApp {
    pub(in crate::app) fn poll_events(&mut self) {
        let events: Vec<_> = self.source.events.try_iter().collect();
        for event in events {
            match event {
                SourceEvent::Connected(info) => {
                    self.hardware.connected = true;
                    self.hardware.connecting = false;
                    self.log.push(
                        LogLevel::Info,
                        format!(
                            "HELLO: {} wire={} contract={} tick={}Hz caps=0x{:08X}",
                            info.firmware_name,
                            info.protocol_version,
                            info.contract_version,
                            info.tick_hz,
                            info.capabilities
                        ),
                    );
                    self.hardware.info = Some(info);
                    self.hardware.version = self.hardware.version_text();
                }
                SourceEvent::Disconnected => {
                    self.hardware.connected = false;
                    self.hardware.connecting = false;
                    self.hardware.info = None;
                    self.hardware.status = None;
                    self.wave.active = false;
                    self.wave.restart_pending = None;
                    self.log.push(LogLevel::Info, "Disconnected".to_owned());
                }
                SourceEvent::Descriptors(descriptors) => {
                    self.log.push(
                        LogLevel::Info,
                        format!("Enumerated {} descriptor(s)", descriptors.len()),
                    );
                    self.inspector.set_descriptors(descriptors);
                    self.restore_workspace_watch_once();
                    self.next_watch_read = Instant::now();
                }
                SourceEvent::Status(status) => {
                    self.hardware.status = Some(status);
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
                    mirror_sequence,
                    start,
                    values,
                } => {
                    self.inspector.update_values(start, values);
                    self.log.push(
                        LogLevel::Debug,
                        format!("Value mirror sequence {mirror_sequence}"),
                    );
                }
                SourceEvent::ChannelsBound {
                    group,
                    bind_sequence,
                } => {
                    self.wave.binding = std::mem::take(&mut self.wave.pending_binding);
                    self.wave.bind_sequence = Some(bind_sequence);
                    self.plot_data.ensure_series(&self.wave.binding);
                    self.log.push(
                        LogLevel::Info,
                        format!("Group {group} binding accepted as sequence {bind_sequence}"),
                    );
                }
                SourceEvent::ScopeConfigured { group, mode } => {
                    self.wave.mode = mode;
                    self.wave.active = mode != ScopeMode::Off;
                    self.log.push(
                        LogLevel::Info,
                        format!("Group {group} configured as {}", mode_label(mode)),
                    );
                    if mode == ScopeMode::Off
                        && let Some(restart_mode) = self.wave.restart_pending.take()
                    {
                        self.start_acquisition(restart_mode);
                    }
                }
                SourceEvent::Blocks(blocks) => {
                    let tick_hz = self.hardware.info.as_ref().map_or(1, |info| info.tick_hz);
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
                            self.wave.settings_snapshot.group,
                            tick_hz,
                            self.wave.settings_snapshot.prescaler,
                        ) {
                            self.log.push(LogLevel::Warn, error);
                        }
                    }
                }
                SourceEvent::StreamGap {
                    group,
                    expected,
                    received,
                } => {
                    self.plot_data.append_gap(&self.wave.binding);
                    self.log.push(
                        LogLevel::Warn,
                        format!(
                            "Group {group} block gap: expected {expected}, received {received}"
                        ),
                    );
                }
                SourceEvent::DeviceChanged { old_hash, new_hash } => {
                    clear_device_session_state(
                        &mut self.wave,
                        &mut self.plot_data,
                        &mut self.inspector,
                    );
                    self.workspace_watch_restored = false;
                    self.log.push(
                        LogLevel::Warn,
                        format!(
                            "Firmware changed 0x{old_hash:08X} -> 0x{new_hash:08X}; re-enumerating"
                        ),
                    );
                }
                SourceEvent::Error(error) => {
                    self.hardware.connecting = false;
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
        ScopeMode::Live => "live",
        ScopeMode::SnapshotArmed => "snapshot armed",
        ScopeMode::SnapshotTriggered => "snapshot triggered",
        ScopeMode::SnapshotFrozen => "snapshot frozen",
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
            min: 0.0,
            max: 1.0,
            scale: 1.0,
            offset: 0.0,
            prescaler: 1,
            group: 0,
        }
    }

    #[test]
    fn stale_bind_sequence_is_discarded() {
        assert!(block_matches_binding(Some(7), 7));
        assert!(!block_matches_binding(Some(7), 6));
        assert!(!block_matches_binding(None, 7));
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
        let mut inspector = InspectorState::default();
        inspector.set_descriptors(vec![descriptor]);
        inspector.pinned.push(0);

        clear_device_session_state(&mut wave, &mut plot_data, &mut inspector);

        assert!(!wave.active);
        assert!(wave.binding.is_empty());
        assert!(wave.pending_binding.is_empty());
        assert_eq!(wave.bind_sequence, None);
        assert!(plot_data.series.is_empty());
        assert!(inspector.descriptors.is_empty());
        assert!(inspector.pinned.is_empty());
    }
}
