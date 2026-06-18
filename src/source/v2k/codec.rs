use thiserror::Error;

use crate::source::{
    DeviceInfo, DeviceStatus, ScopeBlock, ScopeMode, VarDescriptor, VarRef, VarType,
};

pub const WIRE_VERSION: u8 = 5;
pub const VERSION_MAGIC: u8 = 0x50 | WIRE_VERSION;
pub const MAX_PAYLOAD: usize = 1024;

pub mod message {
    pub const HELLO: u8 = 0x01;
    pub const STATUS: u8 = 0x02;
    pub const ENUMERATE: u8 = 0x03;
    pub const CAL_WRITE: u8 = 0x10;
    pub const CAL_COMMIT: u8 = 0x11;
    pub const CAL_READ: u8 = 0x12;
    pub const DAQ_CONTROL: u8 = 0x20;
    pub const BLOCK_REQUEST: u8 = 0x21;
    pub const DAQ_BIND: u8 = 0x22;
    pub const SYSTEM_COMMAND: u8 = 0x30;
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
    if payload.len() < 28 {
        return Err(CodecError::MalformedPayload(
            "HELLO is shorter than current layout",
        ));
    }
    let name_bytes = &payload[12..28];
    let name_len = name_bytes
        .iter()
        .position(|&value| value == 0)
        .unwrap_or(name_bytes.len());
    let firmware_name = String::from_utf8_lossy(&name_bytes[..name_len]).into_owned();
    Ok(DeviceInfo {
        protocol_version: read_u16(payload, 0)?,
        contract_version: read_u16(payload, 2)?,
        build_hash: read_u32(payload, 4)?,
        descriptor_count: read_u16(payload, 8)?,
        firmware_name,
        tick_hz: if payload.len() >= 32 {
            read_u32(payload, 28)?
        } else {
            0
        },
        capabilities: if payload.len() >= 36 {
            read_u32(payload, 32)?
        } else {
            0
        },
    })
}

pub fn parse_status(payload: &[u8]) -> Result<DeviceStatus, CodecError> {
    if payload.len() < 42 {
        return Err(CodecError::MalformedPayload(
            "STATUS is shorter than current layout",
        ));
    }
    Ok(DeviceStatus {
        system_state: read_u16(payload, 0)?,
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
        command_ack_seq: (payload.len() >= 42)
            .then(|| read_u32(payload, 34))
            .transpose()?,
        command_result: (payload.len() >= 42)
            .then(|| read_u16(payload, 38))
            .transpose()?,
    })
}

pub fn parse_descriptors(payload: &[u8]) -> Result<(u16, u16, Vec<VarDescriptor>), CodecError> {
    if payload.len() < 6 {
        return Err(CodecError::MalformedPayload("ENUM header"));
    }
    let total = read_u16(payload, 0)?;
    let start = read_u16(payload, 2)?;
    let count = usize::from(payload[4]);
    if payload.len() != 6 + count * 28 {
        return Err(CodecError::MalformedPayload("ENUM entry count"));
    }
    let mut descriptors = Vec::with_capacity(count);
    for index in 0..count {
        let entry = &payload[6 + index * 28..6 + (index + 1) * 28];
        let name_len = entry[..16]
            .iter()
            .position(|&value| value == 0)
            .unwrap_or(16);
        let ty = VarType::from_wire(read_u16(entry, 16)?)
            .ok_or(CodecError::MalformedPayload("unknown variable type"))?;
        descriptors.push(VarDescriptor {
            name: String::from_utf8_lossy(&entry[..name_len]).into_owned(),
            var: VarRef {
                addr: read_u32(entry, 20)?,
                ty,
            },
            kind: read_u16(entry, 18)?,
            prescaler: read_u16(entry, 24)?,
        });
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
    pub mode: ScopeMode,
    pub overrun_count: u16,
    pub remaining_hint: u16,
    pub trigger_tick: Option<u32>,
    pub blocks: Vec<ScopeBlock>,
}

pub fn parse_block_batch(payload: &[u8]) -> Result<BlockBatch, CodecError> {
    if payload.len() < 12 {
        return Err(CodecError::MalformedPayload("BLOCK_DATA header"));
    }
    let count = usize::from(payload[0]);
    let mode = ScopeMode::from_wire(payload[1]);
    let trigger_tick = read_u32(payload, 8)?;
    let mut offset = 12;
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
    Ok(BlockBatch {
        mode,
        overrun_count: read_u16(payload, 4)?,
        remaining_hint: read_u16(payload, 6)?,
        trigger_tick: (mode == ScopeMode::CaptureFrozen).then_some(trigger_tick),
        blocks,
    })
}

pub fn hello_request() -> Vec<u8> {
    Vec::new()
}

pub fn status_request() -> Vec<u8> {
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

pub fn cal_read_request(start: u16, count: u8) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4);
    put_u16(&mut payload, start);
    payload.push(count);
    payload.push(0);
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
    let mut payload = Vec::with_capacity(20);
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
    payload
}

pub fn block_request(max_blocks: u8) -> Vec<u8> {
    vec![max_blocks, 0]
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

pub fn parse_cal_values(payload: &[u8]) -> Result<(u32, u16, Vec<u32>), CodecError> {
    if payload.len() < 8 {
        return Err(CodecError::MalformedPayload("CAL_READ header"));
    }
    let count = usize::from(payload[6]);
    if payload.len() != 8 + count * 4 {
        return Err(CodecError::MalformedPayload("CAL_READ values"));
    }
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        values.push(read_u32(payload, 8 + index * 4)?);
    }
    Ok((read_u32(payload, 0)?, read_u16(payload, 4)?, values))
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
        assert_eq!(count, 25);
    }

    #[test]
    fn decoder_handles_split_and_joined_frames() {
        let first = encode_frame(message::HELLO, 1, &[]);
        let second = encode_frame(message::STATUS, 2, &[]);
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
        garbage.extend_from_slice(&encode_frame(message::STATUS, 7, &[]));
        let frames = decoder.push(&garbage);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_ref().expect("resynchronized").sequence, 7);
    }

    #[test]
    fn block_batch_parses_capture_trigger_tick() {
        let mut payload = vec![1, ScopeMode::CaptureFrozen.wire_value(), 0, 0];
        payload.extend_from_slice(&0_u16.to_le_bytes());
        payload.extend_from_slice(&0_u16.to_le_bytes());
        payload.extend_from_slice(&1234_u32.to_le_bytes());
        payload.extend_from_slice(&1200_u32.to_le_bytes());
        payload.extend_from_slice(&9_u16.to_le_bytes());
        payload.extend_from_slice(&0_u16.to_le_bytes());
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&2_u16.to_le_bytes());
        payload.extend_from_slice(&4_u16.to_le_bytes());
        payload.extend_from_slice(&0.5_f32.to_le_bytes());

        let batch = parse_block_batch(&payload).expect("parse block batch");

        assert_eq!(batch.mode, ScopeMode::CaptureFrozen);
        assert_eq!(batch.trigger_tick, Some(1234));
        assert_eq!(batch.blocks.len(), 1);
        assert_eq!(batch.blocks[0].start_tick, 1200);
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
        });

        assert_eq!(payload.len(), 20);
        assert_eq!(
            read_u32(&payload, 8).expect("hysteresis bits"),
            0.05_f32.to_bits()
        );
        assert_eq!(read_u16(&payload, 18).expect("record points"), 1_000);
    }
}
