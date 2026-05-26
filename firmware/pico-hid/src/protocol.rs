pub const HOST_MAGIC: u8 = 0x5A;
pub const DEVICE_MAGIC: u8 = 0xA5;
pub const CRC16_CCITT_FALSE_INIT: u16 = 0xFFFF;
pub const CRC16_CCITT_FALSE_POLY: u16 = 0x1021;
pub const MIN_LEN_FIELD: u16 = 7;
pub const LEN_FIELD_SIZE: usize = 2;
pub const SEQ_SIZE: usize = 4;
pub const CMD_SIZE: usize = 1;
pub const CRC_SIZE: usize = 2;
pub const FRAME_OVERHEAD: usize = 1 + LEN_FIELD_SIZE + MIN_LEN_FIELD as usize;
pub const MAX_PAYLOAD_LEN: usize = 1024;
pub const MAX_FRAME_LEN: usize = FRAME_OVERHEAD + MAX_PAYLOAD_LEN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HostCommand {
    Ping = 0x01,
    Identify = 0x02,
    MouseMoveRel = 0x10,
    MouseButton = 0x11,
    MouseWheel = 0x12,
    KeyDown = 0x20,
    KeyUp = 0x21,
    KeyMods = 0x22,
    PadReport = 0x30,
    ReleaseAll = 0x40,
    WatchdogKick = 0x50,
    GetTelemetry = 0x60,
    ResetToBootloader = 0xF0,
}

impl HostCommand {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::Ping),
            0x02 => Some(Self::Identify),
            0x10 => Some(Self::MouseMoveRel),
            0x11 => Some(Self::MouseButton),
            0x12 => Some(Self::MouseWheel),
            0x20 => Some(Self::KeyDown),
            0x21 => Some(Self::KeyUp),
            0x22 => Some(Self::KeyMods),
            0x30 => Some(Self::PadReport),
            0x40 => Some(Self::ReleaseAll),
            0x50 => Some(Self::WatchdogKick),
            0x60 => Some(Self::GetTelemetry),
            0xF0 => Some(Self::ResetToBootloader),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeviceCommand {
    Ack = 0x80,
    Nak = 0x81,
    Pong = 0x82,
    IdentifyResp = 0x83,
    TelemetryResp = 0x84,
    EventButtonPressLocal = 0x90,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NakReason {
    CrcInvalid = 0x01,
    LenInvalid = 0x02,
    UnknownCommand = 0x03,
    PayloadInvalid = 0x04,
    BufferFull = 0x05,
    WatchdogExpired = 0x06,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Frame<'a> {
    pub seq: u32,
    pub command: u8,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseResult<'a> {
    Frame { frame: Frame<'a>, consumed: usize },
    NeedMore { needed: usize },
    Drop { reason: DropReason, consumed: usize },
    Nak { nak: Nak, consumed: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DropReason {
    BadMagic,
    LenTooShort,
    LenOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Nak {
    pub seq: u32,
    pub command: u8,
    pub reason: NakReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    PayloadTooLarge,
    OutputTooSmall { needed: usize },
}

pub fn parse_host_frame(input: &[u8]) -> ParseResult<'_> {
    parse_frame(input, HOST_MAGIC, true)
}

pub fn parse_host_frame_any_command(input: &[u8]) -> ParseResult<'_> {
    parse_frame(input, HOST_MAGIC, false)
}

pub fn parse_device_frame(input: &[u8]) -> ParseResult<'_> {
    parse_frame(input, DEVICE_MAGIC, false)
}

pub fn parse_frame(
    input: &[u8],
    expected_magic: u8,
    reject_unknown_host_command: bool,
) -> ParseResult<'_> {
    if input.is_empty() {
        return ParseResult::NeedMore { needed: 1 };
    }

    if input[0] != expected_magic {
        return ParseResult::Drop {
            reason: DropReason::BadMagic,
            consumed: 1,
        };
    }

    if input.len() < 3 {
        return ParseResult::NeedMore { needed: 3 };
    }

    let len = u16::from_le_bytes([input[1], input[2]]) as usize;
    if len < MIN_LEN_FIELD as usize {
        return ParseResult::Drop {
            reason: DropReason::LenTooShort,
            consumed: 1,
        };
    }

    let payload_len = len - MIN_LEN_FIELD as usize;
    if payload_len > MAX_PAYLOAD_LEN {
        return ParseResult::Drop {
            reason: DropReason::LenOverflow,
            consumed: 1,
        };
    }

    let frame_len = 1 + LEN_FIELD_SIZE + len;
    if input.len() < frame_len {
        return ParseResult::NeedMore { needed: frame_len };
    }

    let seq_start = 1 + LEN_FIELD_SIZE;
    let command_index = seq_start + SEQ_SIZE;
    let payload_start = command_index + CMD_SIZE;
    let crc_start = payload_start + payload_len;
    let crc_expected = u16::from_le_bytes([input[crc_start], input[crc_start + 1]]);
    let crc_actual = crc16_ccitt_false(&input[1..crc_start]);
    let seq = u32::from_le_bytes([
        input[seq_start],
        input[seq_start + 1],
        input[seq_start + 2],
        input[seq_start + 3],
    ]);
    let command = input[command_index];

    if crc_actual != crc_expected {
        return ParseResult::Nak {
            nak: Nak {
                seq,
                command,
                reason: NakReason::CrcInvalid,
            },
            consumed: frame_len,
        };
    }

    if reject_unknown_host_command
        && expected_magic == HOST_MAGIC
        && HostCommand::from_u8(command).is_none()
    {
        return ParseResult::Nak {
            nak: Nak {
                seq,
                command,
                reason: NakReason::UnknownCommand,
            },
            consumed: frame_len,
        };
    }

    ParseResult::Frame {
        frame: Frame {
            seq,
            command,
            payload: &input[payload_start..crc_start],
        },
        consumed: frame_len,
    }
}

pub fn encode_host_frame(
    seq: u32,
    command: HostCommand,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, EncodeError> {
    encode_frame(HOST_MAGIC, seq, command as u8, payload, out)
}

pub fn encode_device_frame(
    seq: u32,
    command: DeviceCommand,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, EncodeError> {
    encode_frame(DEVICE_MAGIC, seq, command as u8, payload, out)
}

pub fn encode_frame(
    magic: u8,
    seq: u32,
    command: u8,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, EncodeError> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(EncodeError::PayloadTooLarge);
    }

    let len = payload.len() + MIN_LEN_FIELD as usize;
    let frame_len = 1 + LEN_FIELD_SIZE + len;
    if out.len() < frame_len {
        return Err(EncodeError::OutputTooSmall { needed: frame_len });
    }

    out[0] = magic;
    out[1..3].copy_from_slice(&(len as u16).to_le_bytes());
    out[3..7].copy_from_slice(&seq.to_le_bytes());
    out[7] = command;
    out[8..8 + payload.len()].copy_from_slice(payload);

    let crc_start = 8 + payload.len();
    let crc = crc16_ccitt_false(&out[1..crc_start]);
    out[crc_start..crc_start + CRC_SIZE].copy_from_slice(&crc.to_le_bytes());

    Ok(frame_len)
}

pub fn encode_ack(seq: u32, out: &mut [u8]) -> Result<usize, EncodeError> {
    encode_device_frame(seq, DeviceCommand::Ack, &seq.to_le_bytes(), out)
}

pub fn encode_nak(seq: u32, reason: NakReason, out: &mut [u8]) -> Result<usize, EncodeError> {
    let mut payload = [0u8; 5];
    payload[..4].copy_from_slice(&seq.to_le_bytes());
    payload[4] = reason as u8;
    encode_device_frame(seq, DeviceCommand::Nak, &payload, out)
}

pub const fn next_sequence(seq: u32) -> u32 {
    seq.wrapping_add(1)
}

pub fn crc16_ccitt_false(bytes: &[u8]) -> u16 {
    let mut crc = CRC16_CCITT_FALSE_INIT;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ CRC16_CCITT_FALSE_POLY;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}
