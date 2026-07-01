use thiserror::Error;

use crate::source::{
    DeviceInfo, DeviceStatus, PerformanceSample, ScopeBlock, ScopeMode, SystemState, VarDescriptor,
    VarRef, VarType,
};

pub const WIRE_VERSION: u8 = 10;
pub const VERSION_MAGIC: u8 = 0x50 | WIRE_VERSION;
pub const MAX_PAYLOAD: usize = 1024;
pub const ENUM_MAX_NAME_LEN: usize = 128;

pub mod message {
    pub const HELLO: u8 = 0x01;
    pub const ENUMERATE: u8 = 0x03;
    pub const CAL_WRITE: u8 = 0x10;
    pub const CAL_COMMIT: u8 = 0x11;
    pub const CAL_READ: u8 = 0x12;
    pub const DAQ_CONTROL: u8 = 0x20;
    pub const CAPTURE_REPLAY: u8 = 0x21;
    pub const DAQ_BIND: u8 = 0x22;
    pub const SYSTEM_COMMAND: u8 = 0x30;
    pub const STATUS_PUSH: u8 = 0x41;
    pub const SCOPE_BLOCK_PUSH: u8 = 0x42;
    pub const CAPTURE_BATCH_PUSH: u8 = 0x45;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub message_type: u8,
    pub sequence: u16,
    pub payload: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("COBS frame is malformed")]
    InvalidCobs,
    #[error("frame is too short")]
    FrameTooShort,
    #[error("wire version mismatch: 0x{0:02X}")]
    VersionMismatch(u8),
    #[error("reserved flags are nonzero")]
    InvalidFlags,
    #[error("payload length is invalid")]
    InvalidLength,
    #[error("CRC-32C mismatch")]
    InvalidCrc,
    #[error("message payload is malformed: {0}")]
    MalformedPayload(&'static str),
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, CodecError> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or(CodecError::MalformedPayload("missing u16"))?;
    Ok(u16::from_le_bytes(bytes.try_into().map_err(|_| {
        CodecError::MalformedPayload("invalid u16")
    })?))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, CodecError> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or(CodecError::MalformedPayload("missing u32"))?;
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| {
        CodecError::MalformedPayload("invalid u32")
    })?))
}

fn put_u16(data: &mut Vec<u8>, value: u16) {
    data.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(data: &mut Vec<u8>, value: u32) {
    data.extend_from_slice(&value.to_le_bytes());
}

pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &octet in data {
        crc ^= u32::from(octet);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0x82F6_3B78 } else { 0 };
        }
    }
    crc ^ 0xFFFF_FFFF
}

pub fn cobs_encode(data: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(data.len() + data.len() / 254 + 2);
    encoded.push(0);
    let mut code_index = 0;
    let mut code = 1_u8;
    for &octet in data {
        if octet == 0 {
            encoded[code_index] = code;
            code_index = encoded.len();
            encoded.push(0);
            code = 1;
        } else {
            encoded.push(octet);
            code = code.wrapping_add(1);
            if code == 0xFF {
                encoded[code_index] = code;
                code_index = encoded.len();
                encoded.push(0);
                code = 1;
            }
        }
    }
    encoded[code_index] = code;
    encoded
}

pub fn cobs_decode(data: &[u8]) -> Result<Vec<u8>, CodecError> {
    let mut decoded = Vec::with_capacity(data.len());
    let mut read = 0;
    while read < data.len() {
        let code = data[read];
        if code == 0 {
            return Err(CodecError::InvalidCobs);
        }
        read += 1;
        let count = usize::from(code - 1);
        if read + count > data.len() {
            return Err(CodecError::InvalidCobs);
        }
        decoded.extend_from_slice(&data[read..read + count]);
        read += count;
        if code != 0xFF && read < data.len() {
            decoded.push(0);
        }
    }
    Ok(decoded)
}

pub fn encode_frame(message_type: u8, sequence: u16, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= MAX_PAYLOAD);
    let mut raw = Vec::with_capacity(11 + payload.len());
    raw.push(VERSION_MAGIC);
    raw.push(message_type);
    raw.push(0);
    put_u16(&mut raw, sequence);
    put_u16(&mut raw, payload.len() as u16);
    raw.extend_from_slice(payload);
    let crc = crc32c(&raw);
    put_u32(&mut raw, crc);
    let mut wire = cobs_encode(&raw);
    wire.push(0);
    wire
}

pub fn decode_raw(raw: &[u8]) -> Result<Frame, CodecError> {
    if raw.len() < 11 {
        return Err(CodecError::FrameTooShort);
    }
    if raw[0] != VERSION_MAGIC {
        return Err(CodecError::VersionMismatch(raw[0]));
    }
    if raw[2] != 0 {
        return Err(CodecError::InvalidFlags);
    }
    let payload_len = usize::from(read_u16(raw, 5)?);
    if payload_len > MAX_PAYLOAD || raw.len() != 7 + payload_len + 4 {
        return Err(CodecError::InvalidLength);
    }
    let expected = read_u32(raw, raw.len() - 4)?;
    if crc32c(&raw[..raw.len() - 4]) != expected {
        return Err(CodecError::InvalidCrc);
    }
    Ok(Frame {
        message_type: raw[1],
        sequence: read_u16(raw, 3)?,
        payload: raw[7..7 + payload_len].to_vec(),
    })
}

#[derive(Default)]
pub struct FrameDecoder {
    encoded: Vec<u8>,
    discard_until_delimiter: bool,
}

impl FrameDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<Frame, CodecError>> {
        let mut frames = Vec::new();
        for &octet in bytes {
            if octet == 0 {
                if !self.discard_until_delimiter && !self.encoded.is_empty() {
                    let result = cobs_decode(&self.encoded).and_then(|raw| decode_raw(&raw));
                    frames.push(result);
                }
                self.encoded.clear();
                self.discard_until_delimiter = false;
            } else if !self.discard_until_delimiter {
                if self.encoded.len() < MAX_PAYLOAD + 16 {
                    self.encoded.push(octet);
                } else {
                    self.encoded.clear();
                    self.discard_until_delimiter = true;
                }
            }
        }
        frames
    }
}

pub fn parse_hello(payload: &[u8]) -> Result<DeviceInfo, CodecError> {
    if payload.len() < 84 {
        return Err(CodecError::MalformedPayload(
            "HELLO is shorter than current layout",
        ));
    }
    fn fixed_string(bytes: &[u8]) -> String {
        let name_len = bytes
            .iter()
            .position(|&value| value == 0)
            .unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..name_len]).into_owned()
    }

    let firmware_name = fixed_string(&payload[12..28]);
    let project_name = fixed_string(&payload[36..68]);
    Ok(DeviceInfo {
        protocol_version: read_u16(payload, 0)?,
        contract_version: read_u16(payload, 2)?,
        build_hash: read_u32(payload, 4)?,
        descriptor_count: read_u16(payload, 8)?,
        firmware_name,
        tick_hz: read_u32(payload, 28)?,
        capabilities: read_u32(payload, 32)?,
        project_name,
        build_time_utc: read_u32(payload, 68)?,
        mcu_model: read_u16(payload, 72)?,
        scope_max_ch: read_u16(payload, 74)?,
        scope_block_ticks: read_u16(payload, 76)?,
        scope_ring_words: read_u32(payload, 80)?,
    })
}

pub fn parse_status(payload: &[u8]) -> Result<DeviceStatus, CodecError> {
    if payload.len() < 96 {
        return Err(CodecError::MalformedPayload(
            "STATUS is shorter than current layout",
        ));
    }
    let performance = parse_status_performance(payload)?;
    Ok(DeviceStatus {
        system_state: SystemState::from_wire(read_u16(payload, 0)?),
        fault_code: read_u16(payload, 2)?,
        status_flags: read_u16(payload, 4)?,
        tick: read_u32(payload, 6)?,
        cpu1_heartbeat: read_u32(payload, 10)?,
        cpu2_heartbeat: read_u32(payload, 14)?,
        applied_seq: read_u32(payload, 18)?,
        calibration_result: read_u16(payload, 22)?,
        calibration_fail_index: read_u16(payload, 24)?,
        build_hash: read_u32(payload, 26)?,
        scope_mode: ScopeMode::from_wire(payload[30]),
        scope_flags: payload[31],
        // Always present: parse_status already requires the full current layout
        // (>= 84), and a shorter STATUS never reaches here because HELLO rejects a
        // mismatched contract_ver. The Option stays for cross-source uniformity
        // (e.g. a future non-native bridge that lacks these fields).
        command_ack_seq: Some(read_u32(payload, 34)?),
        command_result: Some(read_u16(payload, 38)?),
        performance,
        scope_state_seq: read_u16(payload, 84)?,
        scope_frozen_count: read_u16(payload, 86)?,
        scope_trigger_tick: read_u32(payload, 88)?,
        scope_bind_seq: read_u16(payload, 92)?,
    })
}

fn parse_status_performance(payload: &[u8]) -> Result<Option<PerformanceSample>, CodecError> {
    let sequence = read_u32(payload, 42)?;
    let sequence_end = read_u32(payload, 80)?;
    if sequence == 0 || sequence != sequence_end {
        return Ok(None);
    }
    let sample = PerformanceSample {
        sequence,
        cycle_budget: read_u32(payload, 46)?,
        load_average: read_u32(payload, 50)?,
        load_peak: read_u32(payload, 54)?,
        control_at_peak: read_u32(payload, 58)?,
        scope_at_peak: read_u32(payload, 62)?,
        latency_at_peak: read_u16(payload, 66)?,
        peak_tick: read_u32(payload, 68)?,
        violations: read_u32(payload, 72)?,
        overflows: read_u32(payload, 76)?,
    };
    if sample.cycle_budget == 0
        || sample.load_average > sample.load_peak
        || sample.adc_at_peak() > sample.load_peak
        || sample.control_at_peak > sample.load_peak - sample.adc_at_peak()
        || sample.scope_at_peak > sample.load_peak - sample.adc_at_peak() - sample.control_at_peak
    {
        return Ok(None);
    }
    Ok(Some(sample))
}

pub fn parse_descriptors(payload: &[u8]) -> Result<(u16, u16, Vec<VarDescriptor>), CodecError> {
    if payload.len() < 6 {
        return Err(CodecError::MalformedPayload("ENUM header"));
    }
    let total = read_u16(payload, 0)?;
    let start = read_u16(payload, 2)?;
    let count = usize::from(payload[4]);
    if payload[5] != 0 {
        return Err(CodecError::MalformedPayload("ENUM reserved"));
    }
    let mut descriptors = Vec::with_capacity(count);
    let mut offset = 6;
    for _ in 0..count {
        if payload.len() < offset + 12 {
            return Err(CodecError::MalformedPayload("ENUM entry header"));
        }
        let addr = read_u32(payload, offset)?;
        let ty = VarType::from_wire(read_u16(payload, offset + 4)?)
            .ok_or(CodecError::MalformedPayload("unknown variable type"))?;
        let kind = read_u16(payload, offset + 6)?;
        let prescaler = read_u16(payload, offset + 8)?;
        let name_len = usize::from(payload[offset + 10]);
        if payload[offset + 11] != 0 {
            return Err(CodecError::MalformedPayload("ENUM entry reserved"));
        }
        if name_len == 0 || name_len > ENUM_MAX_NAME_LEN {
            return Err(CodecError::MalformedPayload("ENUM name length"));
        }
        offset += 12;
        let name = payload
            .get(offset..offset + name_len)
            .ok_or(CodecError::MalformedPayload("ENUM name"))?;
        if !name.iter().all(|value| (0x20..=0x7e).contains(value)) {
            return Err(CodecError::MalformedPayload("ENUM name ASCII"));
        }
        descriptors.push(VarDescriptor {
            name: std::str::from_utf8(name)
                .map_err(|_| CodecError::MalformedPayload("ENUM name UTF-8"))?
                .to_owned(),
            var: VarRef { addr, ty },
            kind,
            prescaler,
        });
        offset += name_len;
    }
    if offset != payload.len() {
        return Err(CodecError::MalformedPayload("trailing ENUM data"));
    }
    Ok((total, start, descriptors))
}

#[derive(Debug, Clone, Copy)]
pub struct Ack {
    pub status: u8,
    pub echoed_type: u8,
    pub data: u32,
}

pub fn parse_ack(payload: &[u8]) -> Result<Ack, CodecError> {
    if payload.len() < 8 {
        return Err(CodecError::MalformedPayload("ACK"));
    }
    Ok(Ack {
        status: payload[0],
        echoed_type: payload[1],
        data: read_u32(payload, 4)?,
    })
}

#[derive(Debug)]
pub struct BlockBatch {
    pub overrun_count: u16,
    pub blocks: Vec<ScopeBlock>,
}

#[derive(Debug)]
pub struct CaptureBatch {
    pub capture_id: u16,
    pub total_blocks: u16,
    pub first_block_index: u16,
    pub is_replay: bool,
    pub remaining_hint: u16,
    pub trigger_tick: u32,
    pub blocks: Vec<ScopeBlock>,
}

fn parse_scope_blocks(
    payload: &[u8],
    count: usize,
    mut offset: usize,
) -> Result<Vec<ScopeBlock>, CodecError> {
    let mut blocks = Vec::with_capacity(count);
    for _ in 0..count {
        if payload.len() < offset + 16 {
            return Err(CodecError::MalformedPayload("block header"));
        }
        let flags = read_u16(payload, offset + 6)?;
        let sample_count = read_u16(payload, offset + 8)?;
        let stride = read_u16(payload, offset + 14)?;
        let sample_octets = usize::from(sample_count) * usize::from(stride);
        let end = offset + 16 + sample_octets;
        if end > payload.len() {
            return Err(CodecError::MalformedPayload("block samples"));
        }
        blocks.push(ScopeBlock {
            start_tick: read_u32(payload, offset)?,
            block_seq: read_u16(payload, offset + 4)?,
            flags,
            sample_count,
            channel_count: read_u16(payload, offset + 10)?,
            bind_seq: read_u16(payload, offset + 12)?,
            stride_octets: stride,
            samples: payload[offset + 16..end].to_vec(),
        });
        offset = end;
    }
    if offset != payload.len() {
        return Err(CodecError::MalformedPayload("trailing block data"));
    }
    Ok(blocks)
}

pub fn parse_block_batch(payload: &[u8]) -> Result<BlockBatch, CodecError> {
    if payload.len() < 8 {
        return Err(CodecError::MalformedPayload("SCOPE_BLOCK_PUSH header"));
    }
    let count = usize::from(payload[0]);
    if payload[1] != 0 || read_u16(payload, 6)? != 0 {
        return Err(CodecError::MalformedPayload("SCOPE_BLOCK_PUSH reserved"));
    }
    let _remaining_hint = read_u16(payload, 4)?;
    let blocks = parse_scope_blocks(payload, count, 8)?;
    Ok(BlockBatch {
        overrun_count: read_u16(payload, 2)?,
        blocks,
    })
}

pub fn parse_capture_batch(payload: &[u8]) -> Result<CaptureBatch, CodecError> {
    if payload.len() < 16 {
        return Err(CodecError::MalformedPayload("CAPTURE_BATCH_PUSH header"));
    }
    let count = usize::from(payload[6]);
    let flags = payload[7];
    if flags & !0x01 != 0 || read_u16(payload, 10)? != 0 {
        return Err(CodecError::MalformedPayload("CAPTURE_BATCH_PUSH reserved"));
    }
    let blocks = parse_scope_blocks(payload, count, 16)?;
    Ok(CaptureBatch {
        capture_id: read_u16(payload, 0)?,
        total_blocks: read_u16(payload, 2)?,
        first_block_index: read_u16(payload, 4)?,
        is_replay: flags & 0x01 != 0,
        remaining_hint: read_u16(payload, 8)?,
        trigger_tick: read_u32(payload, 12)?,
        blocks,
    })
}

pub fn hello_request() -> Vec<u8> {
    Vec::new()
}

pub fn enum_request(start: u16, max_count: u8) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4);
    put_u16(&mut payload, start);
    payload.push(max_count);
    payload.push(0);
    payload
}

pub fn cal_write_request(writes: &[(VarRef, u32)]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 + writes.len() * 12);
    payload.push(writes.len() as u8);
    payload.push(0);
    for (var, value_bits) in writes {
        put_u32(&mut payload, var.addr);
        put_u32(&mut payload, *value_bits);
        put_u16(&mut payload, var.ty as u16);
        put_u16(&mut payload, 0);
    }
    payload
}

pub fn cal_read_request(reads: &[VarRef]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 + reads.len() * 8);
    payload.push(reads.len() as u8);
    payload.push(0);
    for var in reads {
        put_u32(&mut payload, var.addr);
        put_u16(&mut payload, var.ty as u16);
        put_u16(&mut payload, 0);
    }
    payload
}

pub fn daq_bind_request(channels: &[VarRef]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 + channels.len() * 8);
    payload.push(channels.len() as u8);
    payload.push(0);
    for channel in channels {
        put_u32(&mut payload, channel.addr);
        put_u16(&mut payload, channel.ty as u16);
        put_u16(&mut payload, 0);
    }
    payload
}

pub fn daq_control_request(config: &crate::source::ScopeConfig) -> Vec<u8> {
    let mut payload = Vec::with_capacity(24);
    put_u16(&mut payload, u16::from(config.mode.wire_value()));
    put_u16(&mut payload, config.trigger_slot);
    put_u32(&mut payload, config.trigger_level.to_bits());
    put_u32(&mut payload, config.trigger_hysteresis.to_bits());
    put_u16(
        &mut payload,
        match config.trigger_edge {
            crate::source::TriggerEdge::Rise => 0,
            crate::source::TriggerEdge::Fall => 1,
        },
    );
    put_u16(&mut payload, u16::from(config.pre_trigger_percent));
    put_u16(&mut payload, config.prescaler);
    put_u16(&mut payload, config.record_points);
    put_u16(&mut payload, config.ack_capture_id);
    put_u16(&mut payload, config.flags);
    payload
}

pub fn capture_replay_request(capture_id: u16, first_block_index: u16, max_blocks: u8) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8);
    put_u16(&mut payload, capture_id);
    put_u16(&mut payload, first_block_index);
    payload.push(max_blocks);
    payload.extend_from_slice(&[0, 0, 0]);
    payload
}

pub fn system_command_request(command: crate::source::SystemCommand) -> Vec<u8> {
    let code = match command {
        crate::source::SystemCommand::Start => 1,
        crate::source::SystemCommand::Stop => 2,
        crate::source::SystemCommand::ClearFault => 3,
    };
    let mut payload = Vec::with_capacity(8);
    put_u16(&mut payload, code);
    put_u16(&mut payload, 0);
    put_u32(&mut payload, 0);
    payload
}

pub fn parse_cal_values(payload: &[u8]) -> Result<(u32, Vec<u32>), CodecError> {
    if payload.len() < 8 {
        return Err(CodecError::MalformedPayload("CAL_READ header"));
    }
    let count = usize::from(payload[4]);
    if payload.len() != 8 + count * 4 {
        return Err(CodecError::MalformedPayload("CAL_READ values"));
    }
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        values.push(read_u32(payload, 8 + index * 4)?);
    }
    Ok((read_u32(payload, 0)?, values))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    fn decode_hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16)
                    .expect("valid hex")
            })
            .collect()
    }

    fn load_vector_frame(name: &str) -> Frame {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/vectors")
            .join(name);
        let text = fs::read_to_string(path).expect("read vector");
        let raw = text
            .lines()
            .find_map(|line| line.strip_prefix("raw: "))
            .map(decode_hex)
            .expect("raw line");
        decode_raw(&raw).expect("valid vector")
    }

    fn descriptor_entry(name: &str, ty: VarType, kind: u16, addr: u32) -> Vec<u8> {
        assert!(!name.is_empty());
        assert!(name.len() <= ENUM_MAX_NAME_LEN);
        let mut entry = Vec::new();
        put_u32(&mut entry, addr);
        put_u16(&mut entry, ty as u16);
        put_u16(&mut entry, kind);
        put_u16(&mut entry, 1);
        entry.push(name.len() as u8);
        entry.push(0);
        entry.extend_from_slice(name.as_bytes());
        entry
    }

    #[test]
    fn golden_vectors_conform() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vectors");
        let mut count = 0;
        for entry in fs::read_dir(root).expect("vector directory") {
            let path = entry.expect("vector entry").path();
            if path.extension().and_then(|value| value.to_str()) != Some("txt") {
                continue;
            }
            count += 1;
            let text = fs::read_to_string(&path).expect("read vector");
            let raw = text
                .lines()
                .find_map(|line| line.strip_prefix("raw: "))
                .map(decode_hex)
                .expect("raw line");
            let wire = text
                .lines()
                .find_map(|line| line.strip_prefix("wire: "))
                .map(decode_hex)
                .expect("wire line");
            let is_negative = path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with("neg_"));
            let raw_result = decode_raw(&raw);
            let mut decoder = FrameDecoder::default();
            let wire_result = decoder.push(&wire);
            if is_negative {
                assert!(raw_result.is_err(), "{}", path.display());
                assert!(
                    wire_result.first().is_some_and(Result::is_err),
                    "{}",
                    path.display()
                );
            } else {
                let frame = raw_result.expect("valid raw vector");
                assert_eq!(
                    encode_frame(frame.message_type, frame.sequence, &frame.payload),
                    wire,
                    "{}",
                    path.display()
                );
                assert_eq!(wire_result.len(), 1, "{}", path.display());
                assert_eq!(
                    wire_result[0].as_ref().expect("valid wire vector"),
                    &frame,
                    "{}",
                    path.display()
                );
            }
        }
        assert_eq!(count, 27);
    }

    #[test]
    fn decoder_handles_split_and_joined_frames() {
        let first = encode_frame(message::HELLO, 1, &[]);
        let second = encode_frame(message::STATUS_PUSH, 2, &[]);
        let mut decoder = FrameDecoder::default();
        assert!(decoder.push(&first[..3]).is_empty());
        let mut tail = first[3..].to_vec();
        tail.extend_from_slice(&second);
        let frames = decoder.push(&tail);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].as_ref().expect("first").sequence, 1);
        assert_eq!(frames[1].as_ref().expect("second").sequence, 2);
    }

    #[test]
    fn decoder_resynchronizes_after_oversized_garbage() {
        let mut decoder = FrameDecoder::default();
        let mut garbage = vec![0x55; MAX_PAYLOAD + 32];
        garbage.push(0);
        garbage.extend_from_slice(&encode_frame(message::STATUS_PUSH, 7, &[]));
        let frames = decoder.push(&garbage);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_ref().expect("resynchronized").sequence, 7);
    }

    #[test]
    fn stream_and_capture_batches_parse_current_headers() {
        let mut stream_payload = vec![1, 0];
        stream_payload.extend_from_slice(&7_u16.to_le_bytes());
        stream_payload.extend_from_slice(&5_u16.to_le_bytes());
        stream_payload.extend_from_slice(&0_u16.to_le_bytes());
        stream_payload.extend_from_slice(&1200_u32.to_le_bytes());
        stream_payload.extend_from_slice(&9_u16.to_le_bytes());
        stream_payload.extend_from_slice(&0_u16.to_le_bytes());
        stream_payload.extend_from_slice(&1_u16.to_le_bytes());
        stream_payload.extend_from_slice(&1_u16.to_le_bytes());
        stream_payload.extend_from_slice(&2_u16.to_le_bytes());
        stream_payload.extend_from_slice(&4_u16.to_le_bytes());
        stream_payload.extend_from_slice(&0.5_f32.to_le_bytes());

        let stream = parse_block_batch(&stream_payload).expect("parse stream batch");

        assert_eq!(stream.overrun_count, 7);
        assert_eq!(stream.blocks.len(), 1);
        assert_eq!(stream.blocks[0].start_tick, 1200);

        let mut capture_payload = Vec::new();
        capture_payload.extend_from_slice(&22_u16.to_le_bytes());
        capture_payload.extend_from_slice(&4_u16.to_le_bytes());
        capture_payload.extend_from_slice(&2_u16.to_le_bytes());
        capture_payload.extend_from_slice(&[1, 1]);
        capture_payload.extend_from_slice(&0_u16.to_le_bytes());
        capture_payload.extend_from_slice(&0_u16.to_le_bytes());
        capture_payload.extend_from_slice(&1234_u32.to_le_bytes());
        capture_payload.extend_from_slice(&1200_u32.to_le_bytes());
        capture_payload.extend_from_slice(&9_u16.to_le_bytes());
        capture_payload.extend_from_slice(&0_u16.to_le_bytes());
        capture_payload.extend_from_slice(&1_u16.to_le_bytes());
        capture_payload.extend_from_slice(&1_u16.to_le_bytes());
        capture_payload.extend_from_slice(&2_u16.to_le_bytes());
        capture_payload.extend_from_slice(&4_u16.to_le_bytes());
        capture_payload.extend_from_slice(&0.5_f32.to_le_bytes());

        let capture = parse_capture_batch(&capture_payload).expect("parse capture batch");

        assert_eq!(capture.capture_id, 22);
        assert_eq!(capture.total_blocks, 4);
        assert_eq!(capture.first_block_index, 2);
        assert!(capture.is_replay);
        assert_eq!(capture.trigger_tick, 1234);
        assert_eq!(capture.blocks.len(), 1);
        assert_eq!(capture.blocks[0].start_tick, 1200);
    }

    #[test]
    fn status_parses_viewer2000_system_state() {
        let frame = load_vector_frame("status_resp.txt");

        let status = parse_status(&frame.payload).expect("parse status");

        assert_eq!(status.system_state, SystemState::Running);
        assert!(status.system_state.is_running());
        let performance = status.performance.expect("performance sample");
        assert_eq!(performance.sequence, 3);
        assert_eq!(performance.cycle_budget, 10_000);
        assert_eq!(performance.load_peak, 7_300);
        assert_eq!(performance.control_at_peak, 1_600);
        assert_eq!(performance.scope_at_peak, 900);
        assert_eq!(performance.latency_at_peak, 40);
        assert_eq!(performance.runtime_at_peak(), 4_760);
        assert_eq!(status.scope_state_seq, 22);
        assert_eq!(status.scope_frozen_count, 4);
        assert_eq!(status.scope_trigger_tick, 1234);
        assert_eq!(status.scope_bind_seq, 3);
    }

    #[test]
    fn hello_vector_matches_supported_contract() {
        let frame = load_vector_frame("hello_resp.txt");

        let info = parse_hello(&frame.payload).expect("parse HELLO");

        assert_eq!(info.protocol_version, u16::from(WIRE_VERSION));
        assert_eq!(
            info.contract_version,
            super::super::EXPECTED_CONTRACT_VERSION
        );
        assert_eq!(info.firmware_name, "viewer2000");
        assert_eq!(info.tick_hz, 20_000);
        assert_eq!(info.project_name, "phase4-demo");
        assert_eq!(info.build_time_utc, 1_781_913_600);
        assert_eq!(info.mcu_model, 1);
        assert_eq!(info.scope_max_ch, 16);
        assert_eq!(info.scope_block_ticks, 10);
        assert_eq!(info.scope_ring_words, 0x7000);
    }

    #[test]
    fn hello_parser_rejects_missing_scope_resource_tail() {
        let payload = vec![0_u8; 72];

        assert!(parse_hello(&payload).is_err());
    }

    #[test]
    fn descriptors_parse_baked_paths_and_kind_flags() {
        let mut payload = Vec::new();
        put_u16(&mut payload, 2);
        put_u16(&mut payload, 0);
        payload.extend_from_slice(&[2, 0]);
        payload.extend(descriptor_entry("pi.Kp", VarType::F32, 0x0007, 0xB002));
        payload.extend(descriptor_entry(
            "trace.err[0]",
            VarType::F32,
            0x0002,
            0xB02A,
        ));

        let (total, start, descriptors) = parse_descriptors(&payload).expect("parse descriptors");

        assert_eq!((total, start), (2, 0));
        assert_eq!(descriptors[0].name, "pi.Kp");
        assert!(descriptors[0].is_parameter());
        assert!(descriptors[0].is_scope());
        assert!(descriptors[0].is_user());
        assert_eq!(descriptors[1].name, "trace.err[0]");
        assert!(!descriptors[1].is_parameter());
        assert!(descriptors[1].is_scope());
        assert!(!descriptors[1].is_user());
    }

    #[test]
    fn enum_vector_distinguishes_user_and_system_descriptors() {
        let frame = load_vector_frame("enum_resp_2entries.txt");

        let (_, _, descriptors) = parse_descriptors(&frame.payload).expect("parse ENUM");

        assert!(descriptors[0].is_user());
        assert!(descriptors[0].is_parameter());
        assert!(descriptors[0].is_scope());
        assert!(!descriptors[1].is_user());
        assert!(descriptors[1].is_scope());
    }

    #[test]
    fn daq_control_encodes_fixed_hysteresis_field() {
        let payload = daq_control_request(&crate::source::ScopeConfig {
            mode: ScopeMode::CaptureArmed,
            trigger_slot: 1,
            trigger_level: 2.5,
            trigger_hysteresis: 0.05,
            trigger_edge: crate::source::TriggerEdge::Rise,
            pre_trigger_percent: 30,
            prescaler: 1,
            record_points: 1_000,
            ack_capture_id: crate::source::NO_CAPTURE_ACK,
            flags: 0,
        });

        assert_eq!(payload.len(), 24);
        assert_eq!(
            read_u32(&payload, 8).expect("hysteresis bits"),
            0.05_f32.to_bits()
        );
        assert_eq!(read_u16(&payload, 18).expect("record points"), 1_000);
        assert_eq!(read_u16(&payload, 20).expect("ack capture id"), 0xFFFF);

        assert_eq!(
            capture_replay_request(22, 2, 4),
            vec![22, 0, 2, 0, 4, 0, 0, 0]
        );
    }
}
