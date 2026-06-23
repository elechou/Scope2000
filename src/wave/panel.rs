use std::f32::consts::{FRAC_PI_6, TAU};
use std::num::NonZeroU64;

use eframe::egui;
use eframe::egui_wgpu::wgpu::util::DeviceExt as _;
use eframe::egui_wgpu::{self, wgpu};

use crate::source::{ScopeMode, TriggerEdge};
use crate::theme;
use crate::wave::pane::PaneKind;

use super::csv::CsvState;
use super::{
    DEFAULT_TICK_HZ, MAX_RECORD_POINTS_ABSOLUTE, MIN_PRESCALER, MIN_RECORD_POINTS, WaveState,
    effective_tick_hz, format_record_duration, nearest_sampling_prescaler,
    sampling_prescaler_steps,
};

// ---------------------------------------------------------------------------
// Wave section
// ---------------------------------------------------------------------------

/// Actions produced by the wave panel that the caller must handle.
pub enum WaveAction {
    StartStream,
    ArmCapture,
    Stop,
    Restart(ScopeMode),
}

#[derive(Debug, Clone, Copy)]
pub struct WavePermissions {
    pub can_start: bool,
    pub can_edit_variable_refs: bool,
}

/// Collect variable names from TimeSeries panes in the tile tree.
fn collect_pane_vars(tiles: &egui_tiles::Tiles<crate::wave::pane::ViewPane>) -> Vec<String> {
    let mut vars = Vec::new();
    let ids: Vec<_> = tiles.tile_ids().collect();
    for id in ids {
        if let Some(egui_tiles::Tile::Pane(p)) = tiles.get(id) {
            if p.kind != PaneKind::TimeSeries {
                continue;
            }
            for s in &p.series {
                if !vars.contains(&s.var_name) {
                    vars.push(s.var_name.clone());
                }
            }
        }
    }
    vars
}

pub fn show_wave_section(
    ui: &mut egui::Ui,
    wave: &mut WaveState,
    connected: bool,
    tick_hz: Option<u32>,
    tiles: &egui_tiles::Tiles<crate::wave::pane::ViewPane>,
    record_max_points: Option<u16>,
    permissions: WavePermissions,
) -> Option<WaveAction> {
    let mut action = None;
    let tick_hz = effective_tick_hz(tick_hz.unwrap_or(DEFAULT_TICK_HZ));
    let WavePermissions {
        can_start,
        can_edit_variable_refs,
    } = permissions;

    theme::section_header(ui, "Wave");
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        let w = (ui.available_width() - ui.spacing().item_spacing.x * 2.0) / 3.0;
        if theme::action_button_w(
            ui,
            "Stream",
            theme::GREEN,
            connected && can_start && !wave.active,
            w,
        ) {
            action = Some(WaveAction::StartStream);
        }
        if theme::action_button_w(
            ui,
            "Capture",
            theme::SELECT_BG,
            connected && can_start && !wave.active,
            w,
        ) {
            action = Some(WaveAction::ArmCapture);
        }
        if theme::action_button_w(
            ui,
            "Stop",
            theme::RED,
            wave.active || wave.restart_pending.is_some(),
            w,
        ) {
            action = Some(WaveAction::Stop);
        }
    });
    if wave.active {
        ui.colored_label(
            theme::GREEN,
            format!("{} channels, {}", wave.binding.len(), mode_label(wave.mode)),
        );
    } else {
        ui.colored_label(egui::Color32::GRAY, "Wave stopped");
    }

    ui.add_space(2.0);
    ui.separator();

    let mut any_dragging = false;

    show_sampling_controls(ui, wave, tick_hz, record_max_points, &mut any_dragging);

    ui.add_space(2.0);
    ui.separator();

    let pane_vars = collect_pane_vars(tiles);

    ui.add_enabled_ui(can_edit_variable_refs, |ui| {
        ui.horizontal(|ui| {
            ui.label("Trigger");
            let ch_label = match &wave.settings.trigger_source {
                None => "Auto".to_string(),
                Some(name) => name.clone(),
            };
            egui::ComboBox::from_id_salt("trg_ch")
                .width(90.0)
                .selected_text(ch_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut wave.settings.trigger_source, None, "Auto");
                    for (i, name) in pane_vars.iter().enumerate() {
                        if i > 15 {
                            break;
                        }
                        ui.selectable_value(
                            &mut wave.settings.trigger_source,
                            Some(name.clone()),
                            name.as_str(),
                        );
                    }
                });
            egui::ComboBox::from_id_salt("trg_edge")
                .width(45.0)
                .selected_text(trigger_edge_label(wave.settings.trigger_edge))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut wave.settings.trigger_edge, TriggerEdge::Rise, "Rise");
                    ui.selectable_value(&mut wave.settings.trigger_edge, TriggerEdge::Fall, "Fall");
                });
        })
    });

    ui.horizontal(|ui| {
        ui.label("Level");
        let r = ui.add(egui::DragValue::new(&mut wave.settings.trigger_level).speed(0.1));
        any_dragging |= r.dragged();
        ui.label("Hyst");
        let r = ui.add(
            egui::DragValue::new(&mut wave.settings.trigger_hysteresis)
                .range(0.0..=f32::MAX)
                .speed(0.01),
        );
        any_dragging |= r.dragged();
        ui.label("Pre");
        let r = ui.add(
            egui::DragValue::new(&mut wave.settings.pre_trigger_percent)
                .range(0..=100)
                .speed(1.0)
                .suffix("%"),
        );
        any_dragging |= r.dragged();
    });

    wave.settings.clamp();

    if can_start && wave.active && action.is_none() && !any_dragging {
        let settings_changed =
            settings_changed_for_mode(wave.mode, &wave.settings, &wave.settings_snapshot);
        let channels_changed = pane_vars != wave.pane_vars_snapshot;
        if settings_changed || channels_changed {
            action = Some(WaveAction::Restart(restart_entry_mode(wave.mode)));
        }
    }

    action
}

fn settings_changed_for_mode(
    mode: ScopeMode,
    settings: &super::AcquisitionSettings,
    snapshot: &super::AcquisitionSettings,
) -> bool {
    if mode == ScopeMode::Stream {
        let mut settings = settings.clone();
        let mut snapshot = snapshot.clone();
        settings.record_points = 0;
        snapshot.record_points = 0;
        settings != snapshot
    } else {
        settings != snapshot
    }
}

fn restart_entry_mode(mode: ScopeMode) -> ScopeMode {
    match mode {
        ScopeMode::Stream => ScopeMode::Stream,
        ScopeMode::CaptureArmed | ScopeMode::CapturePost | ScopeMode::CaptureFrozen => {
            ScopeMode::CaptureArmed
        }
        ScopeMode::Off | ScopeMode::Unknown(_) => mode,
    }
}

fn show_sampling_controls(
    ui: &mut egui::Ui,
    wave: &mut WaveState,
    tick_hz: u32,
    record_max_points: Option<u16>,
    any_dragging: &mut bool,
) {
    let record_max_points = record_max_points
        .unwrap_or(MAX_RECORD_POINTS_ABSOLUTE)
        .clamp(MIN_RECORD_POINTS, MAX_RECORD_POINTS_ABSOLUTE);
    wave.settings.clamp_record_points(Some(record_max_points));
    ui.horizontal(|ui| {
        ui.label("Sampling");
        let steps = sampling_steps_for_ui(tick_hz, wave.settings.prescaler);
        let current_idx = steps
            .iter()
            .position(|&prescaler| prescaler == wave.settings.prescaler)
            .unwrap_or(0);
        let mut step_idx = current_idx as i32;
        let max_step_idx = steps.len().saturating_sub(1) as i32;
        let format_steps = steps.clone();
        let parse_steps = steps.clone();
        let response = ui.add(
            egui::DragValue::new(&mut step_idx)
                .range(0..=max_step_idx)
                .speed(0.08)
                .custom_formatter(move |value, _| {
                    let idx = sampling_step_index(value, format_steps.len());
                    format_sampling_interval_us(prescaler_interval_us(format_steps[idx], tick_hz))
                })
                .custom_parser(move |text| {
                    parse_sampling_interval_us(text).map(|interval_us| {
                        let prescaler =
                            nearest_sampling_prescaler(tick_hz, interval_us, &parse_steps);
                        parse_steps
                            .iter()
                            .position(|&step| step == prescaler)
                            .unwrap_or(0) as f64
                    })
                })
                .update_while_editing(false),
        );
        if response.changed() {
            let idx = sampling_step_index(f64::from(step_idx), steps.len());
            wave.settings.prescaler = steps[idx];
        }
        *any_dragging |= response.dragged();
        ui.weak(format_rate(wave.settings.sample_rate_hz(tick_hz)));
    });
    ui.horizontal(|ui| {
        ui.label("Record");
        let response = ui.add(
            egui::DragValue::new(&mut wave.settings.record_points)
                .range(MIN_RECORD_POINTS..=record_max_points)
                .speed(100.0)
                .update_while_editing(false)
                .suffix(" pts"),
        );
        *any_dragging |= response.dragged();
        ui.weak(format_record_duration(
            wave.settings.record_duration_seconds(tick_hz),
        ));
    });
}

fn sampling_steps_for_ui(tick_hz: u32, current_prescaler: u16) -> Vec<u16> {
    let mut steps = sampling_prescaler_steps(tick_hz);
    let current_prescaler = current_prescaler.clamp(MIN_PRESCALER, super::MAX_PRESCALER);
    if !steps.contains(&current_prescaler) {
        steps.push(current_prescaler);
        steps.sort_unstable();
    }
    steps
}

fn sampling_step_index(value: f64, len: usize) -> usize {
    (value.round() as isize).clamp(0, len.saturating_sub(1) as isize) as usize
}

fn prescaler_interval_us(prescaler: u16, tick_hz: u32) -> f64 {
    f64::from(prescaler.max(1)) * 1_000_000.0 / f64::from(effective_tick_hz(tick_hz))
}

fn format_sampling_interval_us(interval_us: f64) -> String {
    if interval_us >= 1_000_000.0 {
        format_compact_time(interval_us / 1_000_000.0, "s")
    } else if interval_us >= 1_000.0 {
        format_compact_time(interval_us / 1_000.0, "ms")
    } else {
        format_compact_time(interval_us, "us")
    }
}

fn format_compact_time(value: f64, unit: &str) -> String {
    let decimals = if value >= 100.0 || value.fract() == 0.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        3
    };
    let mut text = format!("{value:.decimals$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    format!("{text} {unit}")
}

fn parse_sampling_interval_us(text: &str) -> Option<f64> {
    let text = text.trim().to_ascii_lowercase();
    let (number, multiplier) = if let Some(value) = text.strip_suffix("ms") {
        (value.trim(), 1_000.0)
    } else if let Some(value) = text.strip_suffix("us") {
        (value.trim(), 1.0)
    } else if let Some(value) = text.strip_suffix('s') {
        (value.trim(), 1_000_000.0)
    } else {
        (text.as_str(), 1.0)
    };
    number.parse::<f64>().ok().map(|value| value * multiplier)
}

fn format_rate(hz: f64) -> String {
    if hz >= 1_000_000.0 {
        format!("{:.3} MHz", hz / 1_000_000.0)
    } else if hz >= 1_000.0 {
        format!("{:.3} kHz", hz / 1_000.0)
    } else {
        format!("{:.3} Hz", hz)
    }
}

fn mode_label(mode: ScopeMode) -> &'static str {
    match mode {
        ScopeMode::Off => "off",
        ScopeMode::Stream => "stream",
        ScopeMode::CaptureArmed => "capture",
        ScopeMode::CapturePost => "capture post",
        ScopeMode::CaptureFrozen => "capture frozen",
        ScopeMode::Unknown(_) => "unknown",
    }
}

fn trigger_edge_label(edge: TriggerEdge) -> &'static str {
    match edge {
        TriggerEdge::Rise => "Rise",
        TriggerEdge::Fall => "Fall",
    }
}

// ---------------------------------------------------------------------------
// CSV Export section
// ---------------------------------------------------------------------------

const GLYPH_SETTINGS: &str = "\u{eb52}";
const CSV_BUTTON_RADIUS: u8 = 6;
const CSV_BUTTON_STROKE_ALPHA: u8 = 34;
const CSV_FLOW_MIN_COLS: usize = 48;
const CSV_FLOW_MAX_COLS: usize = 160;
const CSV_FLOW_MIN_ROWS: usize = 28;
const CSV_FLOW_MAX_ROWS: usize = 96;

/// Actions produced by the CSV export panel.
pub enum CsvAction {
    QuickSnapshot,
    SaveWithDialog,
}

pub fn init_csv_button_renderer(cc: &eframe::CreationContext<'_>) -> bool {
    let Some(wgpu_render_state) = cc.wgpu_render_state.as_ref() else {
        return false;
    };

    let device = &wgpu_render_state.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("csv_button_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("csv_button_shader.wgsl").into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("csv_button_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(CsvButtonUniforms::BYTE_SIZE as u64),
            },
            count: None,
        }],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("csv_button_pipeline_layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("csv_button_pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu_render_state.target_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let initial_uniforms = CsvButtonUniforms::default();
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("csv_button_uniform_buffer"),
        contents: &initial_uniforms.to_bytes(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("csv_button_bind_group"),
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buffer.as_entire_binding(),
        }],
    });

    wgpu_render_state
        .renderer
        .write()
        .callback_resources
        .insert(CsvButtonRenderResources {
            pipeline,
            bind_group,
            uniform_buffer,
        });

    true
}

pub fn show_csv_export(
    ui: &mut egui::Ui,
    csv: &mut CsvState,
    has_data: bool,
    filename_preview: Option<&str>,
) -> Option<CsvAction> {
    let mut action = None;

    theme::section_header(ui, "CSV Export");
    ui.add_space(4.0);

    let saving = csv.save_rx.is_some();

    // ---- Button row: [Save Data / Ultra Fast Snapshot (wide)] [gear (icon)] ----
    let gear_rect = ui
        .horizontal(|ui| {
            let spacing = ui.spacing().item_spacing.x;
            let gear_w = theme::ACTION_BUTTON_HEIGHT + 4.0;
            let save_w = ui.available_width() - gear_w - spacing;

            if csv.ultra_fast {
                let phase = &mut csv.rainbow_phase;
                if triangle_flow_button(
                    ui,
                    "Ultra Fast Snapshot",
                    has_data && !saving,
                    save_w,
                    phase,
                    csv.gpu_mask_ready,
                ) {
                    action = Some(CsvAction::QuickSnapshot);
                }
            } else if theme::action_button_w(
                ui,
                "Save Data",
                theme::SELECT_BG,
                has_data && !saving,
                save_w,
            ) {
                action = Some(CsvAction::SaveWithDialog);
            }

            // Settings gear button
            let resp =
                theme::action_button_response_w(ui, GLYPH_SETTINGS, theme::WIDGET_BG, true, gear_w);
            if resp.clicked() {
                csv.show_settings = !csv.show_settings;
            }
            resp.rect
        })
        .inner;

    // ---- Filename preview (only in ultra-fast mode) ----
    if csv.ultra_fast
        && let Some(preview) = filename_preview
    {
        ui.label(egui::RichText::new(format!("{preview}.csv")).color(theme::TEXT_SUBDUED));
    }

    // ---- Settings popup + Overwrite modal ----
    show_csv_settings_popup(ui, csv, gear_rect);

    action
}

fn show_csv_settings_popup(ui: &egui::Ui, csv: &mut CsvState, anchor: egui::Rect) {
    if !csv.show_settings {
        return;
    }

    let popup_id = egui::Id::new("csv_settings_popup");
    let pos = egui::pos2(anchor.right() + 4.0, anchor.top());

    let resp = egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .shadow(egui::Shadow {
                    blur: 40,
                    offset: [0, 10],
                    spread: 0,
                    color: egui::Color32::from_black_alpha(192),
                })
                .show(ui, |ui| {
                    ui.set_width(400.0);

                    theme::modal_title(ui, "CSV Export Settings");
                    ui.add_space(8.0);

                    ui.checkbox(&mut csv.ultra_fast, "Ultra Fast Snapshot");
                    ui.add_space(4.0);

                    let editable = csv.ultra_fast;

                    // Directory
                    ui.horizontal(|ui| {
                        ui.label("Dir ");
                        ui.add_enabled(
                            editable,
                            egui::TextEdit::singleline(&mut csv.snapshot_dir)
                                .desired_width(ui.available_width() - 30.0)
                                .hint_text("output directory"),
                        );
                        ui.add_enabled_ui(editable, |ui| {
                            if ui.small_button("...").clicked()
                                && let Some(dir) = rfd::FileDialog::new().pick_folder()
                            {
                                csv.snapshot_dir = dir.to_string_lossy().to_string();
                            }
                        });
                    });

                    // Filename template
                    ui.horizontal(|ui| {
                        ui.label("File");
                        ui.add_enabled(
                            editable,
                            egui::TextEdit::singleline(&mut csv.filename_template)
                                .desired_width(ui.available_width())
                                .hint_text("wave_{$DateTime}"),
                        );
                    });

                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(
                            "Tokens: {$Date} {$Time} {$DateTime} {$var} {$var:.2f}",
                        )
                        .color(theme::TEXT_SUBDUED),
                    );
                });
        });

    // Close when clicking outside the popup (but not on the gear button itself)
    let popup_rect = resp.response.rect;
    if ui.ctx().input(|i| i.pointer.any_pressed())
        && !popup_rect.contains(
            ui.ctx()
                .input(|i| i.pointer.interact_pos().unwrap_or_default()),
        )
        && !anchor.contains(
            ui.ctx()
                .input(|i| i.pointer.interact_pos().unwrap_or_default()),
        )
    {
        csv.show_settings = false;
    }
}

// ---- RGB triangle button with flowing bottom-band sampling ----

/// Button whose fill follows an RGB triangle color space.
fn triangle_flow_button(
    ui: &mut egui::Ui,
    text: &str,
    enabled: bool,
    width: f32,
    phase: &mut f32,
    gpu_mask_ready: bool,
) -> bool {
    let height = theme::ACTION_BUTTON_HEIGHT;
    let rounding = egui::CornerRadius::same(CSV_BUTTON_RADIUS);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, height),
        egui::Sense::click() | egui::Sense::hover(),
    );

    if ui.is_rect_visible(rect) {
        if enabled {
            if response.hovered() {
                *phase += 0.0375;
                ui.ctx().request_repaint();
            }

            if gpu_mask_ready {
                paint_triangle_flow_gpu(ui, rect, f32::from(CSV_BUTTON_RADIUS), *phase);
            } else {
                paint_triangle_flow_rect(ui, rect, f32::from(CSV_BUTTON_RADIUS), *phase);
            }
        } else {
            ui.painter().rect_filled(rect, rounding, theme::WIDGET_BG);
        }

        let text_color = if enabled {
            egui::Color32::WHITE
        } else {
            theme::TEXT_SUBDUED
        };
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let text_pos = rect.center()
            + egui::vec2(
                0.0,
                if response.is_pointer_button_down_on() {
                    0.5
                } else {
                    0.0
                },
            );
        ui.painter().text(
            text_pos + egui::vec2(0.0, 1.0),
            egui::Align2::CENTER_CENTER,
            text,
            font_id.clone(),
            egui::Color32::from_black_alpha(140),
        );
        ui.painter().text(
            text_pos,
            egui::Align2::CENTER_CENTER,
            text,
            font_id,
            text_color,
        );
    }

    enabled && response.clicked()
}

fn paint_triangle_flow_gpu(ui: &egui::Ui, rect: egui::Rect, radius: f32, phase: f32) {
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
        rect,
        CsvButtonPaintCallback {
            width: rect.width(),
            height: rect.height(),
            radius,
            phase,
            pixels_per_point: ui.ctx().pixels_per_point(),
        },
    ));
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(CSV_BUTTON_RADIUS),
        egui::Stroke::new(
            1.0,
            egui::Color32::from_white_alpha(CSV_BUTTON_STROKE_ALPHA),
        ),
        egui::StrokeKind::Inside,
    );
}

fn paint_triangle_flow_rect(ui: &egui::Ui, rect: egui::Rect, radius: f32, phase: f32) {
    let painter = ui.painter();
    let pixels_per_point = ui.ctx().pixels_per_point();
    let cols = ((rect.width() * pixels_per_point * 0.55).round() as usize)
        .clamp(CSV_FLOW_MIN_COLS, CSV_FLOW_MAX_COLS);
    let rows = ((rect.height() * pixels_per_point * 1.9).round() as usize)
        .clamp(CSV_FLOW_MIN_ROWS, CSV_FLOW_MAX_ROWS);
    let mut mesh = egui::Mesh::default();

    for row in 0..=rows {
        let fy = row as f32 / rows as f32;
        let y = egui::lerp(rect.top()..=rect.bottom(), fy);
        let inset = rounded_rect_horizontal_inset(rect.height(), radius, y - rect.top());
        let left = rect.left() + inset;
        let right = rect.right() - inset;

        for col in 0..=cols {
            let tx = col as f32 / cols as f32;
            let x = egui::lerp(left..=right, tx);
            let fx = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let color = triangle_flow_color_at(fx, fy, phase);

            mesh.colored_vertex(egui::pos2(x, y), color);
        }
    }

    let stride = (cols + 1) as u32;
    for row in 0..rows as u32 {
        for col in 0..cols as u32 {
            let tl = row * stride + col;
            let tr = tl + 1;
            let bl = tl + stride;
            let br = bl + 1;
            mesh.add_triangle(tl, bl, tr);
            mesh.add_triangle(bl, tr, br);
        }
    }

    painter.add(egui::Shape::mesh(mesh));
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(CSV_BUTTON_RADIUS),
        egui::Stroke::new(
            1.0,
            egui::Color32::from_white_alpha(CSV_BUTTON_STROKE_ALPHA),
        ),
        egui::StrokeKind::Inside,
    );
}

fn rounded_rect_horizontal_inset(height: f32, radius: f32, y: f32) -> f32 {
    let r = radius.min(height * 0.5);
    if y < r {
        let dy = r - y;
        r - (r * r - dy * dy).max(0.0).sqrt()
    } else if y > height - r {
        let dy = y - (height - r);
        r - (r * r - dy * dy).max(0.0).sqrt()
    } else {
        0.0
    }
}

fn triangle_flow_color_at(fx: f32, fy: f32, phase: f32) -> egui::Color32 {
    let (base_x, base_y) = triangle_window_coords(fx, fy);
    let bottom_band = smoothstep(0.10, 0.96, fy);
    let cover_band = smoothstep(0.16, 1.0, fy);
    let wave_x = (phase * 2.4 + fy * 5.4 + fx * TAU * 1.55).sin();
    let wave_y = (phase * 1.65 - fx * TAU * 1.10).cos();
    let drift = (phase * 0.95 + fx * TAU * 0.75).sin();

    let sample_x = (base_x + bottom_band * (0.135 * wave_x + 0.045 * wave_y + 0.020 * drift))
        .clamp(-0.20, 1.20);
    let flow_floor = 0.90 + 0.05 * drift + 0.03 * wave_y;
    let sample_y = egui::lerp(
        (base_y + bottom_band * 0.035 * wave_y).clamp(-0.10, 1.10)..=flow_floor.clamp(0.82, 0.98),
        cover_band * 0.42,
    );

    triangle_color_at(sample_x, sample_y)
}

fn triangle_window_coords(fx: f32, fy: f32) -> (f32, f32) {
    let cx = fx - 0.5;
    let cy = fy - 0.5;
    let cos_a = FRAC_PI_6.cos();
    let sin_a = FRAC_PI_6.sin();
    let rx = cx * cos_a + cy * sin_a;
    let ry = -cx * sin_a + cy * cos_a;
    let x = 0.50 + rx * 0.72;
    let y = 0.58 + ry * 0.78;
    (x, y)
}

fn triangle_color_at(u: f32, v: f32) -> egui::Color32 {
    const SOURCES: &[(f32, f32, [f32; 3])] = &[
        (0.50, 0.06, [1.0, 0.0, 0.0]),
        (0.10, 0.92, [0.0, 1.0, 0.0]),
        (0.90, 0.92, [0.0, 0.0, 1.0]),
    ];

    let mut rgb = [0.0_f32; 3];
    let mut weight_sum = 0.0_f32;

    for &(cx, cy, source_rgb) in SOURCES {
        let dx = u - cx;
        let dy = v - cy;
        let dist_sq = dx * dx + dy * dy;
        let weight = 1.0 / (dist_sq + 0.010);

        for idx in 0..3 {
            rgb[idx] += weight * source_rgb[idx];
        }
        weight_sum += weight;
    }

    for channel in &mut rgb {
        *channel = (*channel / weight_sum).powf(0.85).clamp(0.0, 1.0);
    }

    egui::Color32::from_rgb(
        (rgb[0] * 255.0) as u8,
        (rgb[1] * 255.0) as u8,
        (rgb[2] * 255.0) as u8,
    )
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct CsvButtonUniforms {
    params0: [f32; 4],
    params1: [f32; 4],
}

impl Default for CsvButtonUniforms {
    fn default() -> Self {
        Self {
            params0: [0.0; 4],
            params1: [0.0; 4],
        }
    }
}

impl CsvButtonUniforms {
    const BYTE_SIZE: usize = 32;

    fn to_bytes(self) -> [u8; Self::BYTE_SIZE] {
        let values = [
            self.params0[0],
            self.params0[1],
            self.params0[2],
            self.params0[3],
            self.params1[0],
            self.params1[1],
            self.params1[2],
            self.params1[3],
        ];
        let mut bytes = [0_u8; Self::BYTE_SIZE];
        for (idx, value) in values.into_iter().enumerate() {
            let start = idx * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

struct CsvButtonRenderResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
}

impl CsvButtonRenderResources {
    fn prepare(&self, queue: &wgpu::Queue, uniforms: CsvButtonUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, &uniforms.to_bytes());
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.draw(0..6, 0..1);
    }
}

struct CsvButtonPaintCallback {
    width: f32,
    height: f32,
    radius: f32,
    phase: f32,
    pixels_per_point: f32,
}

impl egui_wgpu::CallbackTrait for CsvButtonPaintCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = resources.get::<CsvButtonRenderResources>() else {
            return Vec::new();
        };
        resources.prepare(
            queue,
            CsvButtonUniforms {
                params0: [self.width, self.height, self.radius, self.phase],
                params1: [self.pixels_per_point, 0.0, 0.0, 0.0],
            },
        );
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = resources.get::<CsvButtonRenderResources>() else {
            return;
        };
        resources.paint(render_pass);
    }
}
