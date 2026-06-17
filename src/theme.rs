use eframe::egui;
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

/// Semantic colors.
pub const GREEN: Color32 = Color32::from_rgb(40, 167, 69);
pub const RED: Color32 = Color32::from_rgb(220, 53, 69);
#[allow(dead_code)]
pub const YELLOW: Color32 = Color32::from_rgb(255, 200, 40);

/// Structure / borders.
pub const SEPARATOR: Color32 = Color32::from_rgb(45, 50, 54);

// ---- Theme application ----

// ---- Icon URIs (embedded SVGs) ----
pub const ICON_TIMESERIES: &str = "bytes://icon_timeseries.svg";
pub const ICON_DATAFRAME: &str = "bytes://icon_dataframe.svg";
pub const ICON_PLUG: &str = "bytes://icon_plug.svg";
pub const ICON_DND_COPY: &str = "bytes://icon_dnd_copy_to.svg";
pub const ICON_DND_MOVE: &str = "bytes://icon_dnd_move_to.svg";

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
    let desired = Vec2::new(ui.max_rect().width(), 24.0);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        let paint_rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range())
            .expand2(vec2(f32::from(SIDE_PANEL_INNER_MARGIN_X), 0.0));
        let mut painter = ui.painter().clone();
        painter.set_clip_rect(paint_rect);
        painter.rect_filled(paint_rect, 0.0, TAB_BAR);
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        painter.text(
            paint_rect.left_center() + vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            title,
            font_id,
            TEXT_STRONG,
        );
    }

    response
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

/// Styled action button (Execute, Stop, Run, etc).
pub fn action_button(ui: &mut egui::Ui, text: &str, color: Color32, enabled: bool) -> bool {
    let size = action_button_size(ui, text, 0.0);
    action_button_response_sized(ui, text, color, enabled, size).clicked()
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
