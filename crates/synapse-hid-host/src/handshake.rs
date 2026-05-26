use synapse_core::error_codes;
use synapse_core::{
    EXPECTED_FW_MAJOR, SYNAPSE_PICO_HID_BUILD_HASH_LEN, SYNAPSE_PICO_HID_FW_MINOR,
    SYNAPSE_PICO_HID_FW_PATCH,
};

pub const IDENTIFY_RESP_LEN: usize = 20;

/// Parsed firmware identity from the Pico `IDENTIFY_RESP` payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FirmwareIdentity {
    pub fw_major: u8,
    pub fw_minor: u8,
    pub fw_patch: u8,
    pub build_hash: [u8; SYNAPSE_PICO_HID_BUILD_HASH_LEN],
    pub vid: u16,
    pub pid: u16,
    pub capabilities: u32,
}

impl FirmwareIdentity {
    /// Returns true when the firmware major version matches the host contract.
    #[must_use]
    pub const fn matches_expected_version(&self) -> bool {
        self.fw_major == EXPECTED_FW_MAJOR
    }
}

/// Host-side failures while parsing or validating the firmware handshake.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum HandshakeError {
    #[error("identify payload length {actual} did not match expected {expected}")]
    InvalidIdentifyPayloadLength { actual: usize, expected: usize },
    #[error("firmware major version {actual} did not match expected {expected}")]
    FirmwareVersionMismatch { expected: u8, actual: u8 },
}

impl HandshakeError {
    /// Returns the structured Synapse error code for this handshake failure.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidIdentifyPayloadLength { .. } => error_codes::HID_PROTOCOL_HANDSHAKE_FAILED,
            Self::FirmwareVersionMismatch { .. } => error_codes::HID_FIRMWARE_VERSION_MISMATCH,
        }
    }
}

/// Parses the fixed 20-byte Pico `IDENTIFY_RESP` payload.
///
/// # Errors
///
/// Returns [`HandshakeError::InvalidIdentifyPayloadLength`] when the payload
/// length does not exactly match [`IDENTIFY_RESP_LEN`].
pub fn parse_identify_response(payload: &[u8]) -> Result<FirmwareIdentity, HandshakeError> {
    if payload.len() != IDENTIFY_RESP_LEN {
        return Err(HandshakeError::InvalidIdentifyPayloadLength {
            actual: payload.len(),
            expected: IDENTIFY_RESP_LEN,
        });
    }

    let mut build_hash = [0u8; SYNAPSE_PICO_HID_BUILD_HASH_LEN];
    build_hash.copy_from_slice(&payload[4..12]);

    Ok(FirmwareIdentity {
        fw_major: payload[0],
        fw_minor: payload[1],
        fw_patch: payload[2],
        build_hash,
        vid: u16::from_le_bytes([payload[12], payload[13]]),
        pid: u16::from_le_bytes([payload[14], payload[15]]),
        capabilities: u32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]),
    })
}

/// Parses and validates the Pico `IDENTIFY_RESP` payload against host constants.
///
/// # Errors
///
/// Returns [`HandshakeError::InvalidIdentifyPayloadLength`] for malformed
/// payload sizes or [`HandshakeError::FirmwareVersionMismatch`] when the
/// firmware major version differs from [`EXPECTED_FW_MAJOR`].
pub fn parse_and_validate_identify_response(
    payload: &[u8],
) -> Result<FirmwareIdentity, HandshakeError> {
    let identity = parse_identify_response(payload)?;
    validate_expected_major(&identity)?;
    Ok(identity)
}

/// Validates the parsed firmware major version against [`EXPECTED_FW_MAJOR`].
///
/// # Errors
///
/// Returns [`HandshakeError::FirmwareVersionMismatch`] when the parsed major
/// version does not match the host contract.
pub const fn validate_expected_major(identity: &FirmwareIdentity) -> Result<(), HandshakeError> {
    if identity.matches_expected_version() {
        Ok(())
    } else {
        Err(HandshakeError::FirmwareVersionMismatch {
            expected: EXPECTED_FW_MAJOR,
            actual: identity.fw_major,
        })
    }
}

/// Returns the host-expected firmware version triplet.
#[must_use]
pub const fn expected_version_triplet() -> [u8; 3] {
    [
        EXPECTED_FW_MAJOR,
        SYNAPSE_PICO_HID_FW_MINOR,
        SYNAPSE_PICO_HID_FW_PATCH,
    ]
}
