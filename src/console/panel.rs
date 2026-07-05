use eframe::egui;

use crate::console::{LogBuffer, LogLevel};
use crate::theme;

/// Show the console log panel.
pub fn show(ui: &mut egui::Ui, log: &mut LogBuffer) {
    theme::section_header(ui, "Console");
    ui.add_space(2.0);

    // Header row: level filter, entry count, clear button
    let mut clear_clicked = false;
    let visible = ui
        .horizontal(|ui| {
            show_level_filter(ui, log);

            let visible = log.visible_entry_count(log.log_min_level);
            ui.weak(format!("{visible} entries"));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!log.logs.is_empty(), egui::Button::new("Clear"))
                    .clicked()
                {
                    clear_clicked = true;
                }
            });
            visible
        })
        .inner;
    if clear_clicked {
        log.clear();
    }
    let visible = if clear_clicked { 0 } else { visible };

    let min = log.log_min_level;
    let row_height = console_row_height(ui);

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt("console_scroll")
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show_rows(ui, row_height, visible.max(1), |ui, row_range| {
                if visible == 0 {
                    show_empty_entry(ui, row_height);
                } else {
                    for entry in log
                        .visible_entries(min)
                        .skip(row_range.start)
                        .take(row_range.len())
                    {
                        show_entry(ui, entry, row_height);
                    }
                }
            });
    });
}

fn show_empty_entry(ui: &mut egui::Ui, row_height: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_height),
        egui::Sense::hover(),
    );
    paint_entry_text(ui, rect, [("No log entries yet", theme::TEXT_SUBDUED)]);
}

fn show_entry(ui: &mut egui::Ui, entry: &crate::console::LogEntry, row_height: f32) {
    let color = match entry.level {
        LogLevel::Error => theme::RED,
        LogLevel::Warn => theme::YELLOW,
        LogLevel::Notice => theme::GREEN,
        LogLevel::Debug => theme::TEXT_SUBDUED,
        LogLevel::Info => theme::TEXT_DEFAULT,
    };
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_height),
        egui::Sense::hover(),
    );
    let time = format!("[{}]", entry.time);
    paint_entry_text(
        ui,
        rect,
        [
            (time.as_str(), theme::TEXT_SUBDUED),
            (entry.level.label(), color),
            (entry.message.as_str(), color),
        ],
    );
}

fn paint_entry_text<'a>(
    ui: &egui::Ui,
    rect: egui::Rect,
    segments: impl IntoIterator<Item = (&'a str, egui::Color32)>,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }

    let mut painter = ui.painter().clone();
    painter.set_clip_rect(rect.intersect(ui.clip_rect()));
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let mut x = rect.left();
    for (text, color) in segments {
        let galley = painter.layout_no_wrap(text.to_owned(), font_id.clone(), color);
        let pos = snap_pos_to_pixels(ui, egui::pos2(x, rect.center().y - galley.size().y * 0.5));
        painter.galley(pos, galley.clone(), color);
        x += galley.size().x + 8.0;
    }
}

fn console_row_height(ui: &egui::Ui) -> f32 {
    ceil_to_pixels(
        ui,
        ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y,
    )
}

const LOG_LEVEL_FILTER_WIDTH: f32 = 64.0;
const LOG_LEVEL_ROW_HEIGHT: f32 = 20.0;
const LOG_LEVEL_MENU_GAP: f32 = 2.0;
const LOG_LEVELS: [LogLevel; 5] = [
    LogLevel::Debug,
    LogLevel::Info,
    LogLevel::Notice,
    LogLevel::Warn,
    LogLevel::Error,
];

fn show_level_filter(ui: &mut egui::Ui, log: &mut LogBuffer) {
    let button_id = ui.make_persistent_id("log_level_filter");
    let popup_id = button_id.with("popup");
    if console_panel_is_resizing(ui) {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }

    let size = egui::vec2(LOG_LEVEL_FILTER_WIDTH, ui.spacing().interact_size.y);
    let (_, rect) = ui.allocate_space(size);
    let response = ui.interact(rect, button_id, egui::Sense::click());
    let is_open = egui::Popup::is_id_open(ui.ctx(), popup_id);

    paint_level_filter_button(ui, rect, &response, is_open, log.log_min_level.label());

    let menu_pos = level_menu_position(ui, &response);
    egui::Popup::menu(&response)
        .id(popup_id)
        .anchor(menu_pos)
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .gap(0.0)
        .frame(egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(0)))
        .width(LOG_LEVEL_FILTER_WIDTH)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.set_min_size(egui::vec2(LOG_LEVEL_FILTER_WIDTH, level_menu_height()));
            ui.spacing_mut().item_spacing.y = 0.0;
            for lvl in LOG_LEVELS {
                show_level_menu_row(ui, popup_id, log, lvl);
            }
        });
}

fn paint_level_filter_button(
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    is_open: bool,
    text: &str,
) {
    let visuals = if is_open {
        &ui.visuals().widgets.open
    } else {
        ui.style().interact(response)
    };
    let rect = rect.expand(visuals.expansion);
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let margin = ui.spacing().button_padding;
    let inner = rect.shrink2(margin);
    let icon_size = egui::Vec2::splat(ui.spacing().icon_width);
    let icon_rect = egui::Align2::RIGHT_CENTER.align_size_within_rect(icon_size, inner);
    let text_right = icon_rect.left() - ui.spacing().icon_spacing;
    let text_clip = egui::Rect::from_min_max(
        egui::pos2(inner.left(), rect.top()),
        egui::pos2(text_right, rect.bottom()),
    );

    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font_id, visuals.text_color());
    let text_pos = egui::pos2(inner.left(), rect.center().y - galley.size().y * 0.5);
    let mut clipped = ui.painter().clone();
    clipped.set_clip_rect(text_clip);
    clipped.galley(text_pos, galley, visuals.text_color());

    let triangle = egui::Rect::from_center_size(
        icon_rect.center(),
        egui::vec2(icon_rect.width() * 0.7, icon_rect.height() * 0.45),
    );
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            triangle.left_top(),
            triangle.right_top(),
            triangle.center_bottom(),
        ],
        visuals.fg_stroke.color,
        egui::Stroke::NONE,
    ));
}

fn show_level_menu_row(
    ui: &mut egui::Ui,
    popup_id: egui::Id,
    log: &mut LogBuffer,
    level: LogLevel,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(LOG_LEVEL_FILTER_WIDTH, LOG_LEVEL_ROW_HEIGHT),
        egui::Sense::click(),
    );
    if response.clicked() {
        log.log_min_level = level;
        egui::Popup::close_id(ui.ctx(), popup_id);
    }

    if !ui.is_rect_visible(rect) {
        return;
    }

    let selected = log.log_min_level == level;
    let fill = if selected {
        theme::SELECT_BG
    } else if response.hovered() {
        theme::WIDGET_HOVER
    } else {
        egui::Color32::TRANSPARENT
    };
    if fill != egui::Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 0.0, fill);
    }

    let text_color = if selected || response.hovered() {
        theme::TEXT_STRONG
    } else {
        theme::TEXT_DEFAULT
    };
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(level.label().to_owned(), font_id, text_color);
    let text_pos = snap_pos_to_pixels(
        ui,
        egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5),
    );
    ui.painter().galley(text_pos, galley, text_color);
}

fn level_menu_position(ui: &egui::Ui, response: &egui::Response) -> egui::Pos2 {
    let mut button_rect = response.interact_rect;
    if let Some(to_global) = response.ctx.layer_transform_to_global(response.layer_id) {
        button_rect = to_global * button_rect;
    }

    let content = ui.ctx().content_rect();
    snap_pos_to_pixels(
        ui,
        egui::pos2(
            button_rect.left(),
            (button_rect.top() - LOG_LEVEL_MENU_GAP - level_menu_height()).max(content.top()),
        ),
    )
}

fn level_menu_height() -> f32 {
    LOG_LEVEL_ROW_HEIGHT * LOG_LEVELS.len() as f32
}

fn snap_pos_to_pixels(ui: &egui::Ui, pos: egui::Pos2) -> egui::Pos2 {
    let pixels_per_point = ui.ctx().pixels_per_point();
    egui::pos2(
        (pos.x * pixels_per_point).round() / pixels_per_point,
        (pos.y * pixels_per_point).round() / pixels_per_point,
    )
}

fn ceil_to_pixels(ui: &egui::Ui, value: f32) -> f32 {
    let pixels_per_point = ui.ctx().pixels_per_point();
    (value * pixels_per_point).ceil() / pixels_per_point
}

fn console_panel_is_resizing(ui: &egui::Ui) -> bool {
    ui.ctx()
        .read_response(egui::Id::new("console_panel").with("__resize"))
        .is_some_and(|response| response.dragged())
}
