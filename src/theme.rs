use std::num::NonZeroU64;
use std::time::Duration;

use eframe::egui;
use eframe::egui_wgpu::wgpu::util::DeviceExt as _;
use eframe::egui_wgpu::{self, wgpu};
use egui::{Color32, Stroke, Vec2, vec2};

// ---- Color tokens ----

/// Background hierarchy (darkest to lightest).
pub const BG_BODY: Color32 = Color32::from_rgb(13, 16, 17);
pub const BG_PANEL: Color32 = Color32::from_rgb(20, 24, 25);
pub const TAB_BAR: Color32 = Color32::from_rgb(28, 33, 35);
pub const WIDGET_BG: Color32 = Color32::from_rgb(38, 43, 46);
pub const WIDGET_HOVER: Color32 = Color32::from_rgb(49, 56, 59);
pub const WIDGET_ACTIVE: Color32 = Color32::from_rgb(58, 65, 69);

/// Selection.
pub const SELECT_BG: Color32 = Color32::from_rgb(0, 61, 161);
pub const SELECT_STROKE: Color32 = Color32::from_rgb(192, 204, 255);

/// Drag pill colors.
pub const DRAG_PILL_DROPPABLE_FILL: Color32 = Color32::from_rgb(0, 54, 146);
pub const DRAG_PILL_DROPPABLE_STROKE: Color32 = Color32::from_rgb(0, 68, 178);
pub const DRAG_PILL_NONDROPPABLE_FILL: Color32 = Color32::from_rgb(55, 63, 66);
pub const DRAG_PILL_NONDROPPABLE_STROKE: Color32 = Color32::from_rgb(69, 78, 82);

/// Drop target overlay.
pub const DROP_TARGET_STROKE: Color32 = Color32::from_rgb(0, 61, 161);

/// Text hierarchy (dimmest to brightest).
pub const TEXT_SUBDUED: Color32 = Color32::from_rgb(128, 134, 138);
pub const TEXT_DEFAULT: Color32 = Color32::from_rgb(195, 200, 204);
pub const TEXT_STRONG: Color32 = Color32::from_rgb(255, 255, 255);

/// Compact semantic marker for Viewer2000 system variables.
pub const SYSTEM_VARIABLE_BADGE_FILL: Color32 = SELECT_BG;
pub const SYSTEM_VARIABLE_BADGE_TEXT: Color32 = TEXT_STRONG;
pub const SYSTEM_VARIABLE_BADGE_GAP: f32 = 5.0;

/// Semantic colors.
pub const GREEN: Color32 = Color32::from_rgb(40, 167, 69);
pub const RED: Color32 = Color32::from_rgb(220, 53, 69);
#[allow(dead_code)]
pub const YELLOW: Color32 = Color32::from_rgb(255, 200, 40);

/// Control-cycle budget bar. A low-clash spectral palette (RdYlBu-derived)
/// laid out left-to-right across the work segments, with a dim slate for the
/// remaining headroom.
pub const LOAD_ADC: Color32 = Color32::from_rgb(220, 53, 69);
pub const LOAD_CONTROL: Color32 = Color32::from_rgb(255, 200, 40);
pub const LOAD_SCOPE: Color32 = Color32::from_rgb(40, 167, 69);
pub const LOAD_RUNTIME: Color32 = Color32::from_rgb(0, 61, 161);
pub const LOAD_HEADROOM: Color32 = Color32::from_rgb(38, 43, 46);

/// Structure / borders.
pub const SEPARATOR: Color32 = Color32::from_rgb(45, 50, 54);

// ---- Theme application ----

// ---- Icon URIs (embedded SVGs) ----
pub const ICON_TIMESERIES: &str = "bytes://icon_timeseries.svg";
pub const ICON_DATAFRAME: &str = "bytes://icon_dataframe.svg";
pub const ICON_PLUG: &str = "bytes://icon_plug.svg";
pub const ICON_DND_COPY: &str = "bytes://icon_dnd_copy_to.svg";
pub const ICON_DND_MOVE: &str = "bytes://icon_dnd_move_to.svg";
pub const ICON_DND_TRASH: &str = "bytes://icon_dnd_trash.svg";

/// Icon size used in tabs and blueprint entries.
pub const ICON_SIZE: egui::Vec2 = egui::Vec2::new(14.0, 14.0);

// ---- Nerd Font (Font Awesome) glyphs used for row action buttons ----
pub const GLYPH_MINUS: &str = "\u{f068}";
pub const GLYPH_EYE: &str = "\u{f06e}";
pub const GLYPH_EYE_SLASH: &str = "\u{f070}";

// ---- Color swatch (compact color-picker entry) ----
/// Size of the small color swatch square.
pub const COLOR_SWATCH_SIZE: f32 = 14.0;
/// Color picker slider width used inside the swatch popup.
pub const COLOR_SWATCH_POPUP_WIDTH: f32 = 275.0;

/// Paint a 14px color-swatch square at `rect` and return its interaction
/// response. Rounded-rect fill + 1px white-alpha stroke (stronger when hovered
/// to signal interactivity). Caller is responsible for allocation and popup.
pub fn color_swatch_at(
    ui: &egui::Ui,
    rect: egui::Rect,
    resp: &egui::Response,
    color: egui::Color32,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let stroke_color = if resp.hovered() {
        egui::Color32::from_white_alpha(200)
    } else {
        egui::Color32::from_white_alpha(51)
    };
    ui.painter().rect(
        rect,
        3.0,
        color,
        egui::Stroke::new(1.0, stroke_color),
        egui::StrokeKind::Inside,
    );
}

/// Attach a color-picker popup to a swatch response. Returns `true` if the
/// color was modified this frame.
pub fn color_swatch_popup(resp: &egui::Response, color: &mut egui::Color32) -> bool {
    let popup_id = resp.id.with("color_popup");
    let mut changed = false;
    egui::Popup::menu(resp)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().slider_width = COLOR_SWATCH_POPUP_WIDTH;
            if egui::color_picker::color_picker_color32(
                ui,
                color,
                egui::color_picker::Alpha::Opaque,
            ) {
                changed = true;
            }
        });
    changed
}

/// Self-allocating 14px color swatch widget. Opens a color picker popup on
/// click; the edited color is written back into `color` in place. Returns the
/// swatch's response with `.changed()` set when the color was modified.
pub fn color_swatch(ui: &mut egui::Ui, color: &mut egui::Color32) -> egui::Response {
    let size = egui::Vec2::splat(COLOR_SWATCH_SIZE);
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    color_swatch_at(ui, rect, &resp, *color);
    if color_swatch_popup(&resp, color) {
        resp.mark_changed();
    }
    resp
}

pub fn setup_fonts_and_icons(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
    ctx.include_bytes(
        ICON_TIMESERIES,
        include_bytes!("../assets/icons/view_timeseries.svg"),
    );
    ctx.include_bytes(
        ICON_DATAFRAME,
        include_bytes!("../assets/icons/view_dataframe.svg"),
    );
    ctx.include_bytes(ICON_PLUG, include_bytes!("../assets/icons/plug.svg"));
    ctx.include_bytes(
        ICON_DND_COPY,
        include_bytes!("../assets/icons/dnd_copy_to.svg"),
    );
    ctx.include_bytes(
        ICON_DND_MOVE,
        include_bytes!("../assets/icons/dnd_move_to.svg"),
    );
    ctx.include_bytes(
        ICON_DND_TRASH,
        include_bytes!("../assets/icons/dnd_trash.svg"),
    );

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/font/Inter-Medium.ttf"
        ))),
    );
    fonts.font_data.insert(
        "Hack".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/font/Inter-Medium.ttf"
        ))),
    );
    fonts.font_data.insert(
        "NerdSymbols".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/font/SymbolsNerdFontMono-Regular.ttf"
        ))),
    );
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .insert(0, "Inter".to_owned());
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .push("NerdSymbols".to_owned());
    fonts
        .families
        .get_mut(&egui::FontFamily::Monospace)
        .unwrap()
        .insert(0, "Hack".to_owned());
    fonts
        .families
        .get_mut(&egui::FontFamily::Monospace)
        .unwrap()
        .push("NerdSymbols".to_owned());
    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    // ---- Backgrounds ----
    visuals.panel_fill = BG_BODY;
    visuals.window_fill = TAB_BAR;
    visuals.faint_bg_color = BG_PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(5, 5, 5);

    // ---- Selection ----
    visuals.selection.bg_fill = SELECT_BG;
    visuals.selection.stroke = Stroke::new(2.0, SELECT_STROKE);

    // ---- Shadows ----
    let shadow = egui::Shadow {
        blur: 20,
        offset: [0, 5],
        spread: 0,
        color: Color32::from_black_alpha(96),
    };
    visuals.popup_shadow = shadow;
    visuals.window_shadow = shadow;

    // ---- Window / menu chrome ----
    visuals.window_stroke = Stroke::NONE;
    visuals.window_corner_radius = egui::CornerRadius::same(6);
    visuals.menu_corner_radius = egui::CornerRadius::same(6);
    visuals.indent_has_left_vline = false;
    visuals.striped = false;
    visuals.collapsing_header_frame = false;

    // ---- Error / warning ----
    visuals.error_fg_color = Color32::from_rgb(171, 1, 22);
    visuals.warn_fg_color = Color32::from_rgb(255, 122, 12);

    // ---- Widget visuals (per-state) ----

    // Non-interactive (labels, separators)
    visuals.widgets.noninteractive.bg_fill = BG_BODY;
    visuals.widgets.noninteractive.weak_bg_fill = BG_BODY;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, SEPARATOR);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_SUBDUED);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);

    // Inactive (buttons at rest)
    visuals.widgets.inactive.bg_fill = WIDGET_BG;
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_DEFAULT);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);

    // Hovered
    visuals.widgets.hovered.bg_fill = WIDGET_HOVER;
    visuals.widgets.hovered.weak_bg_fill = WIDGET_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_STRONG);
    visuals.widgets.hovered.expansion = 0.0;
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);

    // Active (pressed)
    visuals.widgets.active.bg_fill = WIDGET_ACTIVE;
    visuals.widgets.active.weak_bg_fill = WIDGET_ACTIVE;
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.widgets.active.fg_stroke = Stroke::new(2.0, TEXT_STRONG);
    visuals.widgets.active.expansion = 0.0;
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);

    // Open (e.g. combo-box while expanded)
    visuals.widgets.open.bg_fill = WIDGET_HOVER;
    visuals.widgets.open.weak_bg_fill = WIDGET_HOVER;
    visuals.widgets.open.bg_stroke = Stroke::NONE;
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, TEXT_STRONG);
    visuals.widgets.open.expansion = 0.0;
    visuals.widgets.open.corner_radius = egui::CornerRadius::same(6);

    ctx.set_visuals(visuals);

    // ---- Spacing ----
    let mut style = (*ctx.global_style()).clone();
    // egui's multi-pass layout warns when widget ids shift between passes.
    // Grids legitimately trigger a 2nd pass when their column widths settle,
    // which shifts auto-generated ids for downstream widgets. That's expected
    // behavior, not a bug — mute the noise.
    #[cfg(debug_assertions)]
    {
        style.debug.warn_if_rect_changes_id = false;
    }
    style.spacing.item_spacing = vec2(8.0, 6.0);
    style.spacing.button_padding = vec2(4.0, 1.0);
    style.spacing.indent = 14.0;
    style.spacing.combo_width = 8.0;
    style.spacing.scroll.bar_width = 6.0;
    style.spacing.scroll.bar_inner_margin = 2.0;
    style.spacing.scroll.bar_outer_margin = 2.0;
    style.spacing.tooltip_width = 600.0;
    style.spacing.interact_size.y = 18.0;
    ctx.set_global_style(style);
}

/// 2x faster than egui's default `animation_time` (0.1s). Used only where
/// we pre-tick an animation to override the global rate for a specific id.
pub const FAST_ANIMATION_TIME: f32 = 0.05;
const SIDE_PANEL_INNER_MARGIN_X: i8 = 8;
const SIDE_PANEL_INNER_MARGIN_Y: i8 = 0;
const SECTION_HEADER_HEIGHT: f32 = 24.0;
const SYSTEM_HEADER_STRIPE_WIDTH: f32 = 16.0;
const SYSTEM_HEADER_STRIPE_PERIOD: f32 = SYSTEM_HEADER_STRIPE_WIDTH * 2.0;
const SYSTEM_HEADER_STRIPE_SPEED: f32 = 54.0;
const SYSTEM_HEADER_COS_30: f32 = 0.866_025_4;
const SYSTEM_HEADER_SIN_30: f32 = 0.5;

/// Pre-advance the expansion animation for an egui `Panel` at 2x speed.
/// Call this just before `Panel::...show_animated_inside(...)` — egui's
/// internal `animate_bool_responsive` will then see ~zero elapsed time
/// and leave our advanced value alone. Hovers and other animations keep
/// the default `animation_time`.
pub fn pretick_panel_animation(ctx: &egui::Context, panel_id: &str, is_expanded: bool) {
    ctx.animate_bool_with_time(
        egui::Id::new(panel_id).with("animation"),
        is_expanded,
        FAST_ANIMATION_TIME,
    );
}

// ---- Helper widgets ----

/// Full-width section header bar with dark background.
/// Paints edge-to-edge by bleeding across the side-panel frame margins.
pub fn section_header(ui: &mut egui::Ui, title: &str) -> egui::Response {
    section_header_colored(ui, title, TAB_BAR)
}

pub fn section_header_colored(ui: &mut egui::Ui, title: &str, fill: Color32) -> egui::Response {
    let (rect, response) = allocate_section_header(ui);

    if ui.is_rect_visible(rect) {
        let paint_rect = section_header_paint_rect(ui, rect);
        let mut painter = ui.painter().clone();
        painter.set_clip_rect(paint_rect);
        painter.rect_filled(paint_rect, 0.0, fill);
        let luminance =
            (u32::from(fill.r()) * 299 + u32::from(fill.g()) * 587 + u32::from(fill.b()) * 114)
                / 1000;
        let text_color = if luminance >= 160 {
            Color32::BLACK
        } else {
            TEXT_STRONG
        };
        paint_section_header_title(ui, &painter, paint_rect, title, text_color, false);
    }

    response
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemHeaderStatus {
    Idle,
    Running,
    Fault,
}

impl SystemHeaderStatus {
    fn accent(self) -> Option<Color32> {
        match self {
            Self::Idle => None,
            Self::Running => Some(YELLOW),
            Self::Fault => Some(RED),
        }
    }
}

pub fn init_system_header_renderer(cc: &eframe::CreationContext<'_>) -> bool {
    let Some(wgpu_render_state) = cc.wgpu_render_state.as_ref() else {
        return false;
    };

    let device = &wgpu_render_state.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("system_header_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("system_header_shader.wgsl").into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("system_header_bind_group_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(SystemHeaderUniforms::BYTE_SIZE as u64),
            },
            count: None,
        }],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("system_header_pipeline_layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("system_header_pipeline"),
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

    let initial_uniforms = SystemHeaderUniforms::default();
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("system_header_uniform_buffer"),
        contents: &initial_uniforms.to_bytes(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("system_header_bind_group"),
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
        .insert(SystemHeaderRenderResources {
            pipeline,
            bind_group,
            uniform_buffer,
        });

    true
}

pub fn system_section_header(
    ui: &mut egui::Ui,
    title: &str,
    status: SystemHeaderStatus,
    gpu_ready: bool,
) -> egui::Response {
    let Some(accent) = status.accent() else {
        return section_header(ui, title);
    };

    let (rect, response) = allocate_section_header(ui);
    if ui.is_rect_visible(rect) {
        let paint_rect = section_header_paint_rect(ui, rect);
        let phase = ui.input(|input| input.time as f32) * SYSTEM_HEADER_STRIPE_SPEED;
        ui.ctx().request_repaint_after(Duration::from_millis(16));

        if gpu_ready {
            paint_system_header_gpu(ui, paint_rect, phase, accent);
        } else {
            let mut painter = ui.painter().clone();
            painter.set_clip_rect(paint_rect);
            paint_system_header_cpu(&painter, paint_rect, phase, accent);
        }

        let mut painter = ui.painter().clone();
        painter.set_clip_rect(paint_rect);
        paint_section_header_title(ui, &painter, paint_rect, title, TEXT_STRONG, true);
    }

    response
}

fn allocate_section_header(ui: &mut egui::Ui) -> (egui::Rect, egui::Response) {
    let desired = Vec2::new(ui.max_rect().width(), SECTION_HEADER_HEIGHT);
    ui.allocate_exact_size(desired, egui::Sense::hover())
}

fn section_header_paint_rect(ui: &egui::Ui, rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range())
        .expand2(vec2(f32::from(SIDE_PANEL_INNER_MARGIN_X), 0.0))
}

fn paint_section_header_title(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    title: &str,
    color: Color32,
    shadow: bool,
) {
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let pos = rect.left_center() + vec2(8.0, 0.0);
    if shadow {
        painter.text(
            pos + vec2(0.0, 1.0),
            egui::Align2::LEFT_CENTER,
            title,
            font_id.clone(),
            Color32::from_black_alpha(180),
        );
    }
    painter.text(pos, egui::Align2::LEFT_CENTER, title, font_id, color);
}

fn paint_system_header_gpu(ui: &egui::Ui, rect: egui::Rect, phase: f32, accent: Color32) {
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
        rect,
        SystemHeaderPaintCallback {
            width: rect.width(),
            height: rect.height(),
            phase,
            pixels_per_point: ui.ctx().pixels_per_point(),
            accent,
        },
    ));
}

fn paint_system_header_cpu(painter: &egui::Painter, rect: egui::Rect, phase: f32, accent: Color32) {
    painter.rect_filled(rect, 0.0, Color32::BLACK);

    let phase = phase.rem_euclid(SYSTEM_HEADER_STRIPE_PERIOD);
    let min_coord = -rect.height() * SYSTEM_HEADER_SIN_30;
    let max_coord = rect.width() * SYSTEM_HEADER_COS_30;
    let mut coord = phase
        + (min_coord / SYSTEM_HEADER_STRIPE_PERIOD).floor() * SYSTEM_HEADER_STRIPE_PERIOD
        - SYSTEM_HEADER_STRIPE_PERIOD;

    while coord <= max_coord + SYSTEM_HEADER_STRIPE_PERIOD {
        let x0_top = rect.left() + coord / SYSTEM_HEADER_COS_30;
        let x1_top = rect.left() + (coord + SYSTEM_HEADER_STRIPE_WIDTH) / SYSTEM_HEADER_COS_30;
        let x1_bottom = rect.left()
            + (coord + SYSTEM_HEADER_STRIPE_WIDTH + rect.height() * SYSTEM_HEADER_SIN_30)
                / SYSTEM_HEADER_COS_30;
        let x0_bottom =
            rect.left() + (coord + rect.height() * SYSTEM_HEADER_SIN_30) / SYSTEM_HEADER_COS_30;

        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x0_top, rect.top()),
                egui::pos2(x1_top, rect.top()),
                egui::pos2(x1_bottom, rect.bottom()),
                egui::pos2(x0_bottom, rect.bottom()),
            ],
            accent,
            Stroke::NONE,
        ));
        coord += SYSTEM_HEADER_STRIPE_PERIOD;
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct SystemHeaderUniforms {
    params0: [f32; 4],
    params1: [f32; 4],
}

impl Default for SystemHeaderUniforms {
    fn default() -> Self {
        Self {
            params0: [0.0; 4],
            params1: [0.0; 4],
        }
    }
}

impl SystemHeaderUniforms {
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

struct SystemHeaderRenderResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
}

impl SystemHeaderRenderResources {
    fn prepare(&self, queue: &wgpu::Queue, uniforms: SystemHeaderUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, &uniforms.to_bytes());
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.draw(0..6, 0..1);
    }
}

struct SystemHeaderPaintCallback {
    width: f32,
    height: f32,
    phase: f32,
    pixels_per_point: f32,
    accent: Color32,
}

impl egui_wgpu::CallbackTrait for SystemHeaderPaintCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = resources.get::<SystemHeaderRenderResources>() else {
            return Vec::new();
        };
        resources.prepare(
            queue,
            SystemHeaderUniforms {
                params0: [self.width, self.height, self.phase, self.pixels_per_point],
                params1: color_to_shader_rgba(self.accent),
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
        let Some(resources) = resources.get::<SystemHeaderRenderResources>() else {
            return;
        };
        resources.paint(render_pass);
    }
}

fn color_to_shader_rgba(color: Color32) -> [f32; 4] {
    [
        f32::from(color.r()) / 255.0,
        f32::from(color.g()) / 255.0,
        f32::from(color.b()) / 255.0,
        f32::from(color.a()) / 255.0,
    ]
}

/// Unified action button height across side-panel sections.
pub const ACTION_BUTTON_HEIGHT: f32 = 22.0;

fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}

fn interactive_button_fill(base: Color32, enabled: bool, response: &egui::Response) -> Color32 {
    if !enabled {
        return WIDGET_BG;
    }
    if response.is_pointer_button_down_on() {
        if base == WIDGET_BG {
            WIDGET_ACTIVE
        } else {
            mix_color(base, Color32::BLACK, 0.12)
        }
    } else if response.hovered() {
        if base == WIDGET_BG {
            WIDGET_HOVER
        } else {
            mix_color(base, Color32::WHITE, 0.12)
        }
    } else {
        base
    }
}

fn action_text_color(base: Color32, enabled: bool, response: &egui::Response) -> Color32 {
    if !enabled {
        TEXT_SUBDUED
    } else if base == WIDGET_BG {
        if response.hovered() || response.is_pointer_button_down_on() {
            TEXT_STRONG
        } else {
            TEXT_DEFAULT
        }
    } else {
        Color32::WHITE
    }
}

fn action_button_response_sized(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    enabled: bool,
    size: egui::Vec2,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);

    if ui.is_rect_visible(rect) {
        let fill = interactive_button_fill(color, enabled, &response);
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(6), fill);

        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let text_pos = rect.center()
            + egui::vec2(
                0.0,
                if enabled && response.is_pointer_button_down_on() {
                    0.5
                } else {
                    0.0
                },
            );
        ui.painter().text(
            text_pos,
            egui::Align2::CENTER_CENTER,
            text,
            font_id,
            action_text_color(color, enabled, &response),
        );
    }

    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));

    response
}

fn action_button_size(ui: &egui::Ui, text: &str, min_width: f32) -> egui::Vec2 {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font_id, Color32::WHITE);
    let padding = ui.spacing().button_padding;
    egui::vec2(
        (galley.size().x + padding.x * 2.0).max(min_width),
        ACTION_BUTTON_HEIGHT,
    )
}

/// Styled action button with fixed width. Text is centered both axes.
pub fn action_button_response_w(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    enabled: bool,
    width: f32,
) -> egui::Response {
    action_button_response_sized(
        ui,
        text,
        color,
        enabled,
        egui::vec2(width, ACTION_BUTTON_HEIGHT),
    )
}

/// Styled action button with fixed width. Text is centered both axes.
pub fn action_button_w(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    enabled: bool,
    width: f32,
) -> bool {
    action_button_response_w(ui, text, color, enabled, width).clicked()
}

/// Small button with an SVG icon and fixed width.
pub fn icon_button(ui: &mut egui::Ui, icon_uri: &str, text: &str, width: f32) -> egui::Response {
    let button = egui::Button::image_and_text(
        egui::Image::new(icon_uri)
            .fit_to_exact_size(ICON_SIZE)
            .tint(TEXT_DEFAULT),
        text,
    )
    .min_size(egui::vec2(width, 0.0));
    ui.add(button)
}

/// Side panel frame (no separator, body bg).
pub fn side_panel_frame() -> egui::Frame {
    egui::Frame {
        inner_margin: egui::Margin::symmetric(SIDE_PANEL_INNER_MARGIN_X, SIDE_PANEL_INNER_MARGIN_Y),
        fill: BG_BODY,
        stroke: Stroke::NONE,
        ..Default::default()
    }
}

/// Status bar frame (tab-bar bg, thin top border).
pub fn status_bar_frame() -> egui::Frame {
    egui::Frame {
        inner_margin: egui::Margin::symmetric(8, 2),
        fill: TAB_BAR,
        stroke: Stroke::NONE,
        ..Default::default()
    }
}

/// Menu bar frame (body bg, distinct from tab-bar-colored status bar).
pub fn menu_bar_frame() -> egui::Frame {
    egui::Frame {
        inner_margin: egui::Margin::symmetric(6, 2),
        fill: BG_BODY,
        stroke: Stroke::NONE,
        ..Default::default()
    }
}

/// Custom paint function for CollapsingHeader / CollapsingState toggle arrows.
///
/// Uses Nerd Font Octicons chevrons (`\u{f47c}` down, `\u{f460}` right).
/// In practice we only render the right-chevron and rotate it by
/// `openness * 90°` so the transition between states is a smooth rotation
/// animation (identical visual to the down-chevron when fully open).
/// The glyph is painted at a fixed font size regardless of hover state, so
/// there is no scaling on hover.
pub fn collapsing_arrow_icon(ui: &mut egui::Ui, openness: f32, response: &egui::Response) {
    let color = ui.style().interact(response).fg_stroke.color;
    let font_id = egui::FontId::proportional(12.0);
    let galley = ui
        .painter()
        .layout_no_wrap("\u{f460}".to_owned(), font_id, color);
    let size = galley.size();
    let pos = response.rect.center() - size * 0.5;
    let angle = openness * std::f32::consts::FRAC_PI_2;
    ui.painter().add(
        egui::epaint::TextShape::new(pos, galley, color)
            .with_angle_and_anchor(angle, egui::Align2::CENTER_CENTER),
    );
}

/// Large bold title for modal / popup headers (replaces native title bars).
pub fn modal_title(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).strong().size(16.0));
}

pub fn system_variable_badge_size() -> egui::Vec2 {
    egui::vec2(25.0, 14.0)
}

pub fn paint_system_variable_badge(ui: &egui::Ui, left_center: egui::Pos2, alpha: f32) -> f32 {
    let size = system_variable_badge_size();
    let rect = egui::Rect::from_min_size(
        egui::pos2(left_center.x, left_center.y - size.y * 0.5),
        size,
    );
    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, 3.0, SYSTEM_VARIABLE_BADGE_FILL.gamma_multiply(alpha));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "SYS",
            egui::FontId::proportional(9.0),
            SYSTEM_VARIABLE_BADGE_TEXT.gamma_multiply(alpha),
        );
    }
    size.x + SYSTEM_VARIABLE_BADGE_GAP
}

pub fn system_variable_badge(ui: &mut egui::Ui, alpha: f32) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(system_variable_badge_size(), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, 3.0, SYSTEM_VARIABLE_BADGE_FILL.gamma_multiply(alpha));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "SYS",
            egui::FontId::proportional(9.0),
            SYSTEM_VARIABLE_BADGE_TEXT.gamma_multiply(alpha),
        );
    }
    response
}

/// Semantic action button for modal footers.
/// Normal = `WIDGET_BG`, dangerous = `RED`, recommended = `GREEN`.
pub fn modal_button(ui: &mut egui::Ui, text: &str, color: Color32) -> bool {
    let size = action_button_size(ui, text, 60.0);
    action_button_response_sized(ui, text, color, true, size).clicked()
}

/// Dock toggle button (Nerd Font glyph with active/inactive tint).
pub fn dock_toggle(ui: &mut egui::Ui, glyph: &str, active: bool, tooltip: &str) -> egui::Response {
    let color = if active { TEXT_STRONG } else { TEXT_SUBDUED };
    let btn = egui::Button::new(egui::RichText::new(glyph).size(16.0).color(color))
        .min_size(egui::vec2(22.0, 20.0));
    ui.add(btn).on_hover_text(tooltip)
}
