use crc16::{CCITT_FALSE, State};

pub const HOST_MAGIC: u8 = 0x5A;
pub const DEVICE_MAGIC: u8 = 0xA5;
pub const MIN_LEN_FIELD: u16 = 7;
pub const LEN_FIELD_SIZE: usize = 2;
pub const SEQ_SIZE: usize = 4;
pub const CMD_SIZE: usize = 1;
pub const CRC_SIZE: usize = 2;
pub const FRAME_OVERHEAD: usize = 1 + LEN_FIELD_SIZE + MIN_LEN_FIELD as usize;
pub const MAX_PAYLOAD_LEN: usize = 1024;
pub const MAX_FRAME_LEN: usize = FRAME_OVERHEAD + MAX_PAYLOAD_LEN;

pub const HOST_COMMAND_PING: u8 = 0x01;
pub const HOST_COMMAND_IDENTIFY: u8 = 0x02;
pub const HOST_COMMAND_MOUSE_MOVE_REL: u8 = 0x10;
pub const HOST_COMMAND_MOUSE_BUTTON: u8 = 0x11;
pub const HOST_COMMAND_MOUSE_WHEEL: u8 = 0x12;
pub const HOST_COMMAND_KEY_DOWN: u8 = 0x20;
pub const HOST_COMMAND_KEY_UP: u8 = 0x21;
pub const HOST_COMMAND_KEY_MODS: u8 = 0x22;
pub const HOST_COMMAND_PAD_REPORT: u8 = 0x30;
pub const HOST_COMMAND_RELEASE_ALL: u8 = 0x40;
pub const HOST_COMMAND_WATCHDOG_KICK: u8 = 0x50;
pub const HOST_COMMAND_GET_TELEMETRY: u8 = 0x60;
pub const HOST_COMMAND_RESET_TO_BOOTLOADER: u8 = 0xF0;

pub const DEVICE_COMMAND_ACK: u8 = 0x80;
pub const DEVICE_COMMAND_NAK: u8 = 0x81;
pub const DEVICE_COMMAND_PONG: u8 = 0x82;
pub const DEVICE_COMMAND_IDENTIFY_RESP: u8 = 0x83;
pub const DEVICE_COMMAND_TELEMETRY_RESP: u8 = 0x84;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceFrame<'a> {
    pub seq: u32,
    pub command: u8,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    PayloadTooLarge,
    OutputTooSmall { needed: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    NeedMore { needed: usize },
    BadMagic { actual: u8 },
    LenTooShort { len: usize },
    LenOverflow { payload_len: usize },
    CrcInvalid { expected: u16, actual: u16 },
}

/// Encodes an empty host `IDENTIFY` frame.
///
/// # Errors
///
/// Returns [`EncodeError::OutputTooSmall`] when `out` cannot hold the frame.
pub fn encode_identify_frame(seq: u32, out: &mut [u8]) -> Result<usize, EncodeError> {
    encode_host_frame(seq, HOST_COMMAND_IDENTIFY, &[], out)
}

/// Encodes a host command frame.
///
/// # Errors
///
/// Returns [`EncodeError::PayloadTooLarge`] when `payload` exceeds
/// [`MAX_PAYLOAD_LEN`] or [`EncodeError::OutputTooSmall`] when `out` cannot
/// hold the encoded frame.
pub fn encode_host_frame(
    seq: u32,
    command: u8,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, EncodeError> {
    encode_frame(HOST_MAGIC, seq, command, payload, out)
}

/// Encodes a device response frame.
///
/// # Errors
///
/// Returns [`EncodeError::PayloadTooLarge`] when `payload` exceeds
/// [`MAX_PAYLOAD_LEN`] or [`EncodeError::OutputTooSmall`] when `out` cannot
/// hold the encoded frame.
pub fn encode_device_frame(
    seq: u32,
    command: u8,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, EncodeError> {
    encode_frame(DEVICE_MAGIC, seq, command, payload, out)
}

/// Parses one complete device frame.
///
/// # Errors
///
/// Returns [`ParseError::NeedMore`] when the input prefix is incomplete, or a
/// concrete parse error for invalid magic, length, or CRC.
pub fn parse_device_frame(input: &[u8]) -> Result<DeviceFrame<'_>, ParseError> {
    parse_device_frame_prefix(input).map(|(frame, _consumed)| frame)
}

/// Parses one complete device frame prefix and returns the consumed byte count.
///
/// # Errors
///
/// Returns [`ParseError::NeedMore`] when the input prefix is incomplete, or a
/// concrete parse error for invalid magic, length, or CRC.
pub fn parse_device_frame_prefix(input: &[u8]) -> Result<(DeviceFrame<'_>, usize), ParseError> {
    parse_frame(input, DEVICE_MAGIC)
}

fn encode_frame(
    magic: u8,
    seq: u32,
    command: u8,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, EncodeError> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(EncodeError::PayloadTooLarge);
    }

    let payload_len = u16::try_from(payload.len()).map_err(|_| EncodeError::PayloadTooLarge)?;
    let len = payload_len + MIN_LEN_FIELD;
    let frame_len = 1 + LEN_FIELD_SIZE + usize::from(len);
    if out.len() < frame_len {
        return Err(EncodeError::OutputTooSmall { needed: frame_len });
    }

    out[0] = magic;
    out[1..3].copy_from_slice(&len.to_le_bytes());
    out[3..7].copy_from_slice(&seq.to_le_bytes());
    out[7] = command;
    out[8..8 + payload.len()].copy_from_slice(payload);

    let crc_start = 8 + payload.len();
    let crc = crc16_ccitt_false(&out[1..crc_start]);
    out[crc_start..crc_start + CRC_SIZE].copy_from_slice(&crc.to_le_bytes());

    Ok(frame_len)
}

fn parse_frame(input: &[u8], expected_magic: u8) -> Result<(DeviceFrame<'_>, usize), ParseError> {
    if input.is_empty() {
        return Err(ParseError::NeedMore { needed: 1 });
    }

    if input[0] != expected_magic {
        return Err(ParseError::BadMagic { actual: input[0] });
    }

    if input.len() < 3 {
        return Err(ParseError::NeedMore { needed: 3 });
    }

    let len = u16::from_le_bytes([input[1], input[2]]) as usize;
    if len < MIN_LEN_FIELD as usize {
        return Err(ParseError::LenTooShort { len });
    }

    let payload_len = len - MIN_LEN_FIELD as usize;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(ParseError::LenOverflow { payload_len });
    }

    let frame_len = 1 + LEN_FIELD_SIZE + len;
    if input.len() < frame_len {
        return Err(ParseError::NeedMore { needed: frame_len });
    }

    let seq_start = 1 + LEN_FIELD_SIZE;
    let command_index = seq_start + SEQ_SIZE;
    let payload_start = command_index + CMD_SIZE;
    let crc_start = payload_start + payload_len;
    let crc_expected = u16::from_le_bytes([input[crc_start], input[crc_start + 1]]);
    let crc_actual = crc16_ccitt_false(&input[1..crc_start]);
    if crc_actual != crc_expected {
        return Err(ParseError::CrcInvalid {
            expected: crc_expected,
            actual: crc_actual,
        });
    }

    Ok((
        DeviceFrame {
            seq: u32::from_le_bytes([
                input[seq_start],
                input[seq_start + 1],
                input[seq_start + 2],
                input[seq_start + 3],
            ]),
            command: input[command_index],
            payload: &input[payload_start..crc_start],
        },
        frame_len,
    ))
}

#[must_use]
pub fn crc16_ccitt_false(bytes: &[u8]) -> u16 {
    State::<CCITT_FALSE>::calculate(bytes)
}
