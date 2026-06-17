use std::collections::{HashMap, VecDeque};

use crate::source::{ScopeBlock, VarDescriptor};

/// Circular buffer for one variable's time series.
pub struct TimeSeries {
    pub times: VecDeque<f64>,
    pub values: VecDeque<f64>,
    pub(crate) max_len: usize,
}

impl TimeSeries {
    pub fn new(max_len: usize) -> Self {
        Self {
            times: VecDeque::with_capacity(max_len),
            values: VecDeque::with_capacity(max_len),
            max_len,
        }
    }

    pub fn push(&mut self, time: f64, value: f64) {
        if self.times.len() >= self.max_len {
            self.times.pop_front();
            self.values.pop_front();
        }
        self.times.push_back(time);
        self.values.push_back(value);
    }

    pub fn push_gap(&mut self) {
        let x = self.times.back().copied().unwrap_or(0.0);
        self.push(x, f64::NAN);
    }
}

/// All time series data, keyed by variable name.
pub struct PlotData {
    pub series: HashMap<String, TimeSeries>,
    pub time_counter: f64,
    max_points: usize,
}

impl PlotData {
    pub fn new(max_points: usize) -> Self {
        Self {
            series: HashMap::new(),
            time_counter: 0.0,
            max_points,
        }
    }

    /// Update capacity. Existing series keep data but will shed on next push if over limit.
    pub fn set_max_points(&mut self, new_max: usize) {
        self.max_points = new_max.max(1);
        for ts in self.series.values_mut() {
            ts.max_len = self.max_points;
        }
    }

    /// Push a new value for a named variable.
    pub fn push(&mut self, name: &str, time: f64, value: f64) {
        self.series
            .entry(name.to_string())
            .or_insert_with(|| TimeSeries::new(self.max_points))
            .push(time, value);
        if time > self.time_counter {
            self.time_counter = time;
        }
    }

    pub fn clear(&mut self) {
        self.series.clear();
        self.time_counter = 0.0;
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

    pub fn append_block(
        &mut self,
        block: &ScopeBlock,
        binding: &[VarDescriptor],
        expected_group: u8,
        tick_hz: u32,
        prescaler: u16,
    ) -> Result<(), String> {
        if block.group != u16::from(expected_group) {
            return Err("block group does not match active acquisition".to_owned());
        }
        if binding.len() != usize::from(block.channel_count) {
            return Err("block channel count does not match active binding".to_owned());
        }
        let expected_stride: usize = binding
            .iter()
            .map(|descriptor| descriptor.var.ty.wire_width())
            .sum();
        if expected_stride != usize::from(block.stride_octets) {
            return Err("block stride does not match active binding".to_owned());
        }
        if block.samples.len() != expected_stride * usize::from(block.sample_count) {
            return Err("block payload length is invalid".to_owned());
        }

        self.ensure_series(binding);
        let tick_hz = f64::from(tick_hz.max(1));
        let tick_step = u64::from(prescaler.max(1));
        for sample_index in 0..usize::from(block.sample_count) {
            let mut offset = sample_index * expected_stride;
            let tick = u64::from(block.start_tick) + sample_index as u64 * tick_step;
            let time = tick as f64 / tick_hz;
            for descriptor in binding {
                let width = descriptor.var.ty.wire_width();
                let raw = descriptor
                    .var
                    .ty
                    .decode(&block.samples[offset..offset + width])
                    .ok_or_else(|| "sample type decode failed".to_owned())?;
                self.push(&descriptor.name, time, raw);
                offset += width;
            }
        }
        Ok(())
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
            group: 0,
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
            group: 0,
            sample_count: 1,
            channel_count: binding.len() as u16,
            bind_seq: 3,
            stride_octets: samples.len() as u16,
            samples,
        };
        let mut data = PlotData::new(100);
        data.append_block(&block, &binding, 0, 1_000, 1)
            .expect("append block");
        assert_eq!(data.series["i16"].values[0], -2.0);
        assert_eq!(data.series["u16"].values[0], 7.0);
        assert_eq!(data.series["i32"].values[0], -3.0);
        assert_eq!(data.series["u32"].values[0], 9.0);
        assert_eq!(data.series["f32"].values[0], 1.5);
    }

    #[test]
    fn stream_gap_inserts_nan_breaks() {
        let binding = vec![descriptor("signal", VarType::F32)];
        let mut data = PlotData::new(100);
        data.push("signal", 1.0, 2.0);
        data.append_gap(&binding);

        let series = &data.series["signal"];
        assert_eq!(series.times.len(), 2);
        assert!(series.values[1].is_nan());
    }
}
