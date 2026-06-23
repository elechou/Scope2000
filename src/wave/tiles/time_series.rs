use eframe::egui;
use egui::Vec2;
use egui_plot::{Axis, Legend, Line, LineStyle, PlotPoints, VLine};
use egui_tiles::TileId;

use super::MyTilesDelegate;
use crate::theme;
use crate::wave::dnd::VarDragPayload;
use crate::wave::pane::{AxisRange, LegendCorner, TimeAxisMode};
use crate::wave::selection::Selection;

const GRID_SPACING_MIN_PX: f64 = 8.0;
const Y_AXIS_SIDE_MARGIN: f32 = 4.0;
const MIN_X_AXIS_THICKNESS: f32 = 18.0;
const MIN_Y_AXIS_THICKNESS: f32 = 36.0;

fn axis_bounds_for_layout(
    axis_range: &AxisRange,
    auto_bounds: (f64, f64),
    last_bounds: (f64, f64),
) -> (f64, f64) {
    let valid = |(min, max): (f64, f64)| min.is_finite() && max.is_finite() && max > min;

    match axis_range {
        AxisRange::Manual { min, max } if valid((*min, *max)) => (*min, *max),
        AxisRange::Auto if valid(auto_bounds) => auto_bounds,
        _ if valid(last_bounds) => last_bounds,
        _ => (0.0, 1.0),
    }
}

fn default_axis_tick_label(mark: egui_plot::GridMark) -> String {
    let num_decimals = (-mark.step_size.log10().round()).max(0.0) as usize;
    egui::emath::format_with_decimals_in_range(mark.value, num_decimals..=num_decimals)
}

fn estimate_axis_tick_thickness(
    ui: &egui::Ui,
    axis: Axis,
    bounds: (f64, f64),
    axis_pixels: f32,
) -> f32 {
    let (min, max) = bounds;
    let range = max - min;
    if !min.is_finite() || !max.is_finite() || range <= 0.0 || axis_pixels <= 1.0 {
        return 0.0;
    }

    let base_step_size = range.abs() / axis_pixels as f64 * GRID_SPACING_MIN_PX;
    if !base_step_size.is_finite() || base_step_size <= f64::EPSILON {
        return 0.0;
    }

    let steps = egui_plot::log_grid_spacer(10)(egui_plot::GridInput {
        bounds: (min, max),
        base_step_size,
    });

    let label_spacing = match axis {
        Axis::X => egui::Rangef::new(60.0, 80.0),
        Axis::Y => egui::Rangef::new(20.0, 30.0),
    };
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let painter = ui.painter();
    let pixels_per_value = axis_pixels as f64 / range.abs();
    let mut thickness: f32 = 0.0;

    for step in steps {
        let spacing_in_points = (pixels_per_value * step.step_size).abs() as f32;
        if spacing_in_points <= label_spacing.min {
            continue;
        }

        let galley = painter.layout_no_wrap(
            default_axis_tick_label(step),
            font_id.clone(),
            ui.visuals().text_color(),
        );
        let galley_size = match axis {
            Axis::X => galley.size(),
            Axis::Y => galley.size() + 2.0 * Y_AXIS_SIDE_MARGIN * Vec2::X,
        };
        let axis_extent = match axis {
            Axis::X => galley_size.x,
            Axis::Y => galley_size.y,
        };

        if spacing_in_points < axis_extent {
            continue;
        }

        thickness = thickness.max(match axis {
            Axis::X => galley_size.y,
            Axis::Y => galley_size.x,
        });
    }

    thickness
}

impl<'a> MyTilesDelegate<'a> {
    pub(super) fn time_series_ui(
        &mut self,
        ui: &mut egui::Ui,
        tile_id: TileId,
        pane: &mut crate::wave::pane::ViewPane,
    ) -> egui_tiles::UiResponse {
        let plot_id = egui::Id::new(("scope2000_plot", tile_id));

        if let Some(mut mem) = egui_plot::PlotMemory::load(ui.ctx(), plot_id) {
            mem.hidden_items.clear();
            for s in pane.series.iter().filter(|s| !s.visible) {
                mem.hidden_items.insert(egui::Id::new(s.label()));
            }
            mem.store(ui.ctx(), plot_id);
        }

        let x_origin = match pane.properties.time_axis_mode {
            TimeAxisMode::System => 0.0,
            TimeAxisMode::TriggerRelative => self.data.trigger_time.unwrap_or(0.0),
        };
        let trigger_x = self.data.trigger_time.map(|time| time - x_origin);

        let series_data: Vec<_> = pane
            .series
            .iter()
            .filter_map(|s| {
                self.data.series.get(&s.var_name).map(|ts| {
                    let points: Vec<[f64; 2]> = ts
                        .times
                        .iter()
                        .zip(ts.values.iter())
                        .map(|(&t, &v)| [t - x_origin, v])
                        .collect();
                    (s.label().to_string(), s.color, points, s.width)
                })
            })
            .collect();

        let props = &pane.properties;
        let highlight_var = self.highlight_var;
        let has_time_axis_data = series_data
            .iter()
            .any(|(_, _, points, _)| !points.is_empty());

        ui.style_mut().visuals.extreme_bg_color = props.background;

        let show_grid = props.show_grid;
        let legend_visible = props.legend_visible;
        let legend_corner = props.legend_corner;
        let sync_time_axis = props.sync_time_axis;
        let time_range = props.time_axis_range.clone();
        let scalar_range = props.scalar_axis_range.clone();
        let time_is_auto = matches!(time_range, AxisRange::Auto);
        let scalar_is_auto = matches!(scalar_range, AxisRange::Auto);
        let apply_pending = std::mem::take(&mut pane.properties.axis_apply_pending);

        let (padded_x, padded_y) = {
            let mut x_min = f64::INFINITY;
            let mut x_max = f64::NEG_INFINITY;
            let mut y_min = f64::INFINITY;
            let mut y_max = f64::NEG_INFINITY;
            for (_, _, points, _) in &series_data {
                for &[x, y] in points.iter() {
                    if x < x_min {
                        x_min = x;
                    }
                    if x > x_max {
                        x_max = x;
                    }
                    if y < y_min {
                        y_min = y;
                    }
                    if y > y_max {
                        y_max = y;
                    }
                }
            }
            let pad = |lo: f64, hi: f64| -> (f64, f64) {
                if !lo.is_finite() || !hi.is_finite() {
                    return (0.0, 1.0);
                }
                let range = hi - lo;
                if !range.is_finite() || range <= 0.0 {
                    return (lo - 1.0, hi + 1.0);
                }
                let m = range * 0.05 / 2.0;
                (lo - m, hi + m)
            };
            (pad(x_min, x_max), pad(y_min, y_max))
        };

        let (inner_resp, payload) =
            ui.dnd_drop_zone::<VarDragPayload, _>(egui::Frame::default(), |ui| {
                ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_BODY);

                let complete_size = ui.available_size_before_wrap();
                let x_bounds =
                    axis_bounds_for_layout(&time_range, padded_x, pane.properties.last_bounds_x);
                let y_bounds =
                    axis_bounds_for_layout(&scalar_range, padded_y, pane.properties.last_bounds_y);
                let mut reserved_x = MIN_X_AXIS_THICKNESS;
                let mut reserved_y = MIN_Y_AXIS_THICKNESS;
                for _ in 0..2 {
                    let plot_height = (complete_size.y - reserved_x).max(1.0);
                    reserved_y = reserved_y.max(estimate_axis_tick_thickness(
                        ui,
                        Axis::Y,
                        y_bounds,
                        plot_height,
                    ));
                    let plot_width = (complete_size.x - reserved_y).max(1.0);
                    reserved_x = reserved_x.max(estimate_axis_tick_thickness(
                        ui,
                        Axis::X,
                        x_bounds,
                        plot_width,
                    ));
                }

                let mut plot = egui_plot::Plot::new(tile_id)
                    .id(plot_id)
                    .height(ui.available_height())
                    .allow_drag(true)
                    .allow_zoom(true)
                    .allow_scroll(true)
                    .auto_bounds(egui::Vec2b::new(time_is_auto, scalar_is_auto))
                    .set_margin_fraction(egui::Vec2::ZERO)
                    .y_axis_min_width(reserved_y)
                    .custom_x_axes(vec![
                        egui_plot::AxisHints::new_x().min_thickness(reserved_x),
                    ])
                    .show_grid(show_grid);

                if sync_time_axis
                    && has_time_axis_data
                    && let Some(group_id) = self.time_axis_sync_group
                {
                    plot = plot.link_axis(group_id, egui::Vec2b::new(true, false));
                }

                if time_is_auto {
                    plot = plot.include_x(padded_x.0).include_x(padded_x.1);
                }
                if scalar_is_auto {
                    plot = plot.include_y(padded_y.0).include_y(padded_y.1);
                }

                if legend_visible {
                    let corner = match legend_corner {
                        LegendCorner::LeftTop => egui_plot::Corner::LeftTop,
                        LegendCorner::RightTop => egui_plot::Corner::RightTop,
                        LegendCorner::LeftBottom => egui_plot::Corner::LeftBottom,
                        LegendCorner::RightBottom => egui_plot::Corner::RightBottom,
                    };
                    plot = plot.legend(Legend::default().position(corner));
                }

                let plot_resp = plot.show(ui, |plot_ui| {
                    if apply_pending {
                        plot_ui.set_auto_bounds(egui::Vec2b::new(time_is_auto, scalar_is_auto));
                        if let AxisRange::Manual { min, max } = &time_range {
                            plot_ui.set_plot_bounds_x(*min..=*max);
                        }
                        if let AxisRange::Manual { min, max } = &scalar_range {
                            plot_ui.set_plot_bounds_y(*min..=*max);
                        }
                    }

                    if let Some(x) = trigger_x {
                        plot_ui.vline(
                            VLine::new("Trigger", x)
                                .color(theme::YELLOW)
                                .style(LineStyle::dashed_dense())
                                .width(1.0)
                                .allow_hover(false),
                        );
                    }

                    for (name, color, points, width) in &series_data {
                        if !points.is_empty() {
                            let item_id = egui::Id::new(name.as_str());
                            let is_highlighted = highlight_var
                                .is_some_and(|(tid, vid)| tid == tile_id && vid == item_id);
                            plot_ui.line(
                                Line::new(name.as_str(), PlotPoints::new(points.clone()))
                                    .color(*color)
                                    .width(*width)
                                    .highlight(is_highlighted),
                            );
                        }
                    }
                });

                let bounds = plot_resp.transform.bounds();
                let bounds_x = (bounds.min()[0], bounds.max()[0]);
                let bounds_y = (bounds.min()[1], bounds.max()[1]);

                let plot_clicked = plot_resp.response.clicked();

                (
                    plot_resp.hovered_plot_item,
                    bounds_x,
                    bounds_y,
                    plot_clicked,
                )
            });

        let (hovered_item, bounds_x, bounds_y, plot_clicked) = inner_resp.inner;
        pane.properties.last_bounds_x = bounds_x;
        pane.properties.last_bounds_y = bounds_y;

        let mem_auto = egui_plot::PlotMemory::load(ui.ctx(), plot_id)
            .map(|m| m.auto_bounds)
            .unwrap_or(egui::Vec2b::TRUE);
        pane.properties.time_axis_range = if mem_auto.x {
            AxisRange::Auto
        } else {
            AxisRange::Manual {
                min: bounds_x.0,
                max: bounds_x.1,
            }
        };
        pane.properties.scalar_axis_range = if mem_auto.y {
            AxisRange::Auto
        } else {
            AxisRange::Manual {
                min: bounds_y.0,
                max: bounds_y.1,
            }
        };

        if let Some(mem) = egui_plot::PlotMemory::load(ui.ctx(), plot_id) {
            for s in pane.series.iter_mut() {
                let id = egui::Id::new(s.label());
                s.visible = !mem.hidden_items.contains(&id);
            }
        }

        if let Some(id) = hovered_item {
            self.hovered_plot_var = Some((tile_id, id));
        }

        if self.can_edit_variable_refs
            && inner_resp.response.contains_pointer()
            && egui::DragAndDrop::has_payload_of_type::<VarDragPayload>(ui.ctx())
        {
            self.drop_hover_tile = Some(tile_id);
        }

        if self.can_edit_variable_refs
            && let Some(dropped) = payload
            && let Some(fb) = self.apply_drop(ui.ctx(), tile_id, pane, &dropped)
        {
            self.drop_feedback = Some(fb);
        }

        if plot_clicked && !super::is_pane_selected(self.selection, tile_id) {
            *self.selection = Selection::Pane(tile_id);
        }

        egui_tiles::UiResponse::None
    }
}
