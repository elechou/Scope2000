use std::collections::{HashMap, VecDeque};

use egui_plot::PlotPoint;

use crate::source::{ScopeBlock, VarDescriptor};

const MIN_LOD_POINTS: usize = 256;
const LOD_BUCKET_SIZE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesBounds {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

/// Circular buffer for one variable's time series. Full-resolution points are
/// retained for CSV export while cached levels are used for rendering.
pub struct TimeSeries {
    pub points: VecDeque<PlotPoint>,
    lod_levels: Vec<Vec<PlotPoint>>,
    bounds: Option<SeriesBounds>,
    max_len: usize,
}

impl TimeSeries {
    pub fn new(max_len: usize) -> Self {
        Self {
            points: VecDeque::with_capacity(max_len),
            lod_levels: Vec::new(),
            bounds: None,
            max_len,
        }
    }

    pub fn push(&mut self, time: f64, value: f64) {
        self.append_samples(&[time], &[value]);
    }

    fn append_samples(&mut self, times: &[f64], values: &[f64]) {
        let count = times.len().min(values.len());
        if self.max_len == 0 {
            self.points.clear();
            self.rebuild_render_cache();
            return;
        }

        if count >= self.max_len {
            self.points.clear();
            let start = count - self.max_len;
            self.points.extend(
                times[start..count]
                    .iter()
                    .zip(&values[start..count])
                    .map(|(&x, &y)| PlotPoint::new(x, y)),
            );
        } else {
            let overflow = self
                .points
                .len()
                .saturating_add(count)
                .saturating_sub(self.max_len);
            if overflow != 0 {
                self.points.drain(..overflow);
            }
            self.points.extend(
                times[..count]
                    .iter()
                    .zip(&values[..count])
                    .map(|(&x, &y)| PlotPoint::new(x, y)),
            );
        }
        self.rebuild_render_cache();
    }

    pub fn push_gap(&mut self) {
        let x = self.points.back().map_or(0.0, |point| point.x);
        self.push(x, f64::NAN);
    }

    pub fn bounds(&self) -> Option<SeriesBounds> {
        self.bounds
    }

    /// Borrow a cached level whose visible point count is appropriate for the
    /// current plot width. Raw points remain available when zoomed in.
    pub fn render_points(&self, x_range: Option<(f64, f64)>, target: usize) -> &[PlotPoint] {
        let target = target.max(MIN_LOD_POINTS);
        let (raw, wrapped) = self.points.as_slices();
        debug_assert!(wrapped.is_empty(), "render cache keeps points contiguous");
        let raw = visible_points(raw, x_range);
        if raw.len() <= target {
            return raw;
        }

        for level in &self.lod_levels {
            let visible = visible_points(level, x_range);
            if visible.len() <= target {
                return visible;
            }
        }

        self.lod_levels
            .last()
            .map_or(raw, |level| visible_points(level, x_range))
    }

    fn rebuild_render_cache(&mut self) {
        self.points.make_contiguous();
        self.bounds = series_bounds(self.points.as_slices().0);
        self.lod_levels.clear();

        loop {
            let (source_len, next) = if let Some(source) = self.lod_levels.last() {
                (source.len(), downsample_min_max(source))
            } else {
                let source = self.points.as_slices().0;
                (source.len(), downsample_min_max(source))
            };
            if source_len <= MIN_LOD_POINTS || next.len() >= source_len {
                break;
            }
            self.lod_levels.push(next);
        }
    }
}

fn series_bounds(points: &[PlotPoint]) -> Option<SeriesBounds> {
    let mut bounds = SeriesBounds {
        x_min: f64::INFINITY,
        x_max: f64::NEG_INFINITY,
        y_min: f64::INFINITY,
        y_max: f64::NEG_INFINITY,
    };
    for point in points {
        if point.x.is_finite() {
            bounds.x_min = bounds.x_min.min(point.x);
            bounds.x_max = bounds.x_max.max(point.x);
        }
        if point.y.is_finite() {
            bounds.y_min = bounds.y_min.min(point.y);
            bounds.y_max = bounds.y_max.max(point.y);
        }
    }
    if !bounds.x_min.is_finite() || !bounds.x_max.is_finite() {
        return None;
    }
    if !bounds.y_min.is_finite() || !bounds.y_max.is_finite() {
        bounds.y_min = 0.0;
        bounds.y_max = 0.0;
    }
    Some(bounds)
}

fn visible_points(points: &[PlotPoint], x_range: Option<(f64, f64)>) -> &[PlotPoint] {
    let Some((left, right)) = x_range else {
        return points;
    };
    let (left, right) = if left <= right {
        (left, right)
    } else {
        (right, left)
    };
    let start = points
        .partition_point(|point| point.x < left)
        .saturating_sub(1);
    let end = points
        .partition_point(|point| point.x <= right)
        .saturating_add(1)
        .min(points.len());
    &points[start.min(end)..end]
}

/// Reduce each time-ordered bucket to its minimum and maximum value, emitted
/// in their original time order. This preserves narrow spikes better than
/// averaging while producing a compact render level.
fn downsample_min_max(points: &[PlotPoint]) -> Vec<PlotPoint> {
    let mut reduced = Vec::with_capacity(points.len().div_ceil(2));
    for chunk in points.chunks(LOD_BUCKET_SIZE) {
        if chunk.len() <= 2 {
            reduced.extend_from_slice(chunk);
            continue;
        }

        let mut min_index = 0;
        let mut max_index = 0;
        for (index, point) in chunk.iter().enumerate().skip(1) {
            if point.y.total_cmp(&chunk[min_index].y).is_lt() {
                min_index = index;
            }
            if point.y.total_cmp(&chunk[max_index].y).is_gt() {
                max_index = index;
            }
        }
        if min_index == max_index {
            reduced.push(chunk[min_index]);
        } else if min_index < max_index {
            reduced.push(chunk[min_index]);
            reduced.push(chunk[max_index]);
        } else {
            reduced.push(chunk[max_index]);
            reduced.push(chunk[min_index]);
        }
    }
    if let Some(&last) = points.last()
        && reduced.last() != Some(&last)
    {
        reduced.push(last);
    }
    reduced
}

/// All time series data, keyed by variable name.
pub struct PlotData {
    pub series: HashMap<String, TimeSeries>,
    pub time_counter: f64,
    pub trigger_time: Option<f64>,
    time_origin_tick: Option<u32>,
    max_points: usize,
}

pub(crate) fn relative_time_from_ticks(tick: u32, origin_tick: u32, tick_hz: u32) -> f64 {
    f64::from(tick.wrapping_sub(origin_tick) as i32) / f64::from(tick_hz.max(1))
}

impl PlotData {
    pub fn new(max_points: usize) -> Self {
        Self {
            series: HashMap::new(),
            time_counter: 0.0,
            trigger_time: None,
            time_origin_tick: None,
            max_points,
        }
    }

    /// Push a single value. Streaming and capture paths use the batched APIs
    /// below so name lookup and render-cache rebuilding are not done per point.
    pub fn push(&mut self, name: &str, time: f64, value: f64) {
        self.series
            .entry(name.to_owned())
            .or_insert_with(|| TimeSeries::new(self.max_points))
            .push(time, value);
        self.time_counter = self.time_counter.max(time);
    }

    pub fn clear(&mut self) {
        self.series.clear();
        self.time_counter = 0.0;
        self.trigger_time = None;
        self.time_origin_tick = None;
    }

    pub fn set_trigger_tick(&mut self, trigger_tick: u32) {
        self.time_origin_tick = Some(trigger_tick);
        self.trigger_time = Some(0.0);
    }

    pub fn ensure_series(&mut self, binding: &[VarDescriptor]) {
        for descriptor in binding {
            self.series
                .entry(descriptor.name.clone())
                .or_insert_with(|| TimeSeries::new(self.max_points));
        }
    }

    pub fn append_gap(&mut self, binding: &[VarDescriptor]) {
        self.ensure_series(binding);
        for descriptor in binding {
            if let Some(series) = self.series.get_mut(&descriptor.name) {
                series.push_gap();
            }
        }
    }

    /// Append already-decoded channel-major values. Each series is looked up
    /// and its render cache is rebuilt once for the entire batch.
    pub fn append_channel_samples(
        &mut self,
        binding: &[VarDescriptor],
        times: &[f64],
        values: &[Vec<f64>],
    ) -> Result<(), String> {
        if binding.len() != values.len() {
            return Err("decoded channel count does not match active binding".to_owned());
        }
        if values.iter().any(|channel| channel.len() != times.len()) {
            return Err("decoded channel length does not match time axis".to_owned());
        }

        self.ensure_series(binding);
        for (descriptor, channel) in binding.iter().zip(values) {
            self.series
                .get_mut(&descriptor.name)
                .expect("binding series was ensured")
                .append_samples(times, channel);
        }
        if let Some(&time) = times.last() {
            self.time_counter = self.time_counter.max(time);
        }
        Ok(())
    }

    pub fn append_block(
        &mut self,
        block: &ScopeBlock,
        binding: &[VarDescriptor],
        tick_hz: u32,
        prescaler: u16,
    ) -> Result<(), String> {
        let errors = self.append_blocks(std::slice::from_ref(block), binding, tick_hz, prescaler);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Decode and append a complete source event as one channel-major batch.
    /// Stream blocks remain ordered and none are discarded here.
    pub fn append_blocks(
        &mut self,
        blocks: &[ScopeBlock],
        binding: &[VarDescriptor],
        tick_hz: u32,
        prescaler: u16,
    ) -> Vec<String> {
        let expected_stride: usize = binding
            .iter()
            .map(|descriptor| descriptor.var.ty.wire_width())
            .sum();
        let total_samples = blocks
            .iter()
            .map(|block| usize::from(block.sample_count))
            .sum();
        let mut times = Vec::with_capacity(total_samples);
        let mut values: Vec<Vec<f64>> = (0..binding.len())
            .map(|_| Vec::with_capacity(total_samples))
            .collect();
        let mut errors = Vec::new();
        let tick_step = u32::from(prescaler.max(1));

        for block in blocks {
            let _reserved_flags = block.flags;
            if binding.len() != usize::from(block.channel_count) {
                errors.push("block channel count does not match active binding".to_owned());
                continue;
            }
            if expected_stride != usize::from(block.stride_octets) {
                errors.push("block stride does not match active binding".to_owned());
                continue;
            }
            if block.samples.len() != expected_stride * usize::from(block.sample_count) {
                errors.push("block payload length is invalid".to_owned());
                continue;
            }

            let origin_tick = *self.time_origin_tick.get_or_insert(block.start_tick);
            let batch_start = times.len();
            let mut decode_error = None;
            for sample_index in 0..usize::from(block.sample_count) {
                let tick = block
                    .start_tick
                    .wrapping_add((sample_index as u32).wrapping_mul(tick_step));
                times.push(relative_time_from_ticks(tick, origin_tick, tick_hz));
                let mut offset = sample_index * expected_stride;
                for (channel, descriptor) in binding.iter().enumerate() {
                    let width = descriptor.var.ty.wire_width();
                    let Some(raw) = descriptor
                        .var
                        .ty
                        .decode(&block.samples[offset..offset + width])
                    else {
                        decode_error = Some("sample type decode failed".to_owned());
                        break;
                    };
                    values[channel].push(raw);
                    offset += width;
                }
                if decode_error.is_some() {
                    break;
                }
            }
            if let Some(error) = decode_error {
                times.truncate(batch_start);
                for channel in &mut values {
                    channel.truncate(batch_start);
                }
                errors.push(error);
            }
        }

        if !times.is_empty()
            && let Err(error) = self.append_channel_samples(binding, &times, &values)
        {
            errors.push(error);
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{ScopeBlock, VarRef, VarType};

    fn descriptor(name: &str, ty: VarType) -> VarDescriptor {
        VarDescriptor {
            name: name.to_owned(),
            var: VarRef { addr: 0, ty },
            kind: 0,
            prescaler: 1,
        }
    }

    #[test]
    fn native_block_decode_preserves_wire_width_until_plot_boundary() {
        let binding = vec![
            descriptor("i16", VarType::I16),
            descriptor("u16", VarType::U16),
            descriptor("i32", VarType::I32),
            descriptor("u32", VarType::U32),
            descriptor("f32", VarType::F32),
        ];
        let mut samples = Vec::new();
        samples.extend_from_slice(&(-2_i16).to_le_bytes());
        samples.extend_from_slice(&7_u16.to_le_bytes());
        samples.extend_from_slice(&(-3_i32).to_le_bytes());
        samples.extend_from_slice(&9_u32.to_le_bytes());
        samples.extend_from_slice(&1.5_f32.to_le_bytes());
        let block = ScopeBlock {
            start_tick: 100,
            block_seq: 1,
            flags: 0,
            sample_count: 1,
            channel_count: binding.len() as u16,
            bind_seq: 3,
            stride_octets: samples.len() as u16,
            samples,
        };
        let mut data = PlotData::new(100);
        data.append_block(&block, &binding, 1_000, 1)
            .expect("append block");
        assert_eq!(data.series["i16"].points[0].x, 0.0);
        assert_eq!(data.series["i16"].points[0].y, -2.0);
        assert_eq!(data.series["u16"].points[0].y, 7.0);
        assert_eq!(data.series["i32"].points[0].y, -3.0);
        assert_eq!(data.series["u32"].points[0].y, 9.0);
        assert_eq!(data.series["f32"].points[0].y, 1.5);
    }

    #[test]
    fn stream_gap_inserts_nan_breaks() {
        let binding = vec![descriptor("signal", VarType::F32)];
        let mut data = PlotData::new(100);
        data.push("signal", 1.0, 2.0);
        data.append_gap(&binding);

        let series = &data.series["signal"];
        assert_eq!(series.points.len(), 2);
        assert!(series.points[1].y.is_nan());
    }

    #[test]
    fn batched_stream_blocks_preserve_order_and_time() {
        let binding = vec![descriptor("signal", VarType::F32)];
        let blocks = vec![
            ScopeBlock {
                start_tick: 100,
                block_seq: 1,
                flags: 0,
                sample_count: 1,
                channel_count: 1,
                bind_seq: 3,
                stride_octets: 4,
                samples: 1.0_f32.to_le_bytes().to_vec(),
            },
            ScopeBlock {
                start_tick: 102,
                block_seq: 2,
                flags: 0,
                sample_count: 1,
                channel_count: 1,
                bind_seq: 3,
                stride_octets: 4,
                samples: 2.0_f32.to_le_bytes().to_vec(),
            },
        ];
        let mut data = PlotData::new(100);

        assert!(data.append_blocks(&blocks, &binding, 100, 1).is_empty());

        let points = &data.series["signal"].points;
        assert_eq!(
            points.iter().map(|point| point.y).collect::<Vec<_>>(),
            vec![1.0, 2.0]
        );
        assert_eq!(
            points.iter().map(|point| point.x).collect::<Vec<_>>(),
            vec![0.0, 0.02]
        );
    }

    #[test]
    fn stream_ring_keeps_only_the_newest_full_resolution_points() {
        let mut series = TimeSeries::new(3);
        series.append_samples(&[0.0, 1.0], &[10.0, 11.0]);
        series.append_samples(&[2.0, 3.0], &[12.0, 13.0]);

        assert_eq!(
            series
                .points
                .iter()
                .map(|point| (point.x, point.y))
                .collect::<Vec<_>>(),
            vec![(1.0, 11.0), (2.0, 12.0), (3.0, 13.0)]
        );
        assert_eq!(series.render_points(None, 256), series.points.as_slices().0);
    }

    #[test]
    fn lod_preserves_spikes_and_raw_points_when_zoomed() {
        let mut series = TimeSeries::new(4096);
        let times: Vec<_> = (0..4096).map(|index| index as f64).collect();
        let mut values = vec![0.0; 4096];
        values[2049] = 100.0;
        series.append_samples(&times, &values);

        assert_eq!(series.points.len(), 4096, "full CSV data must be retained");
        let overview = series.render_points(None, 256);
        assert!(overview.len() <= 257);
        assert!(overview.iter().any(|point| point.y == 100.0));

        let zoomed = series.render_points(Some((2040.0, 2060.0)), 256);
        assert!(zoomed.len() >= 21);
        assert!(zoomed.iter().any(|point| point.y == 100.0));
    }
}
