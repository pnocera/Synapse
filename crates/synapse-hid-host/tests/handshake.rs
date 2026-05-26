use synapse_core::error_codes;
use synapse_core::{
    EXPECTED_FW_MAJOR, SYNAPSE_PICO_HID_FW_MINOR, SYNAPSE_PICO_HID_FW_PATCH,
    SYNAPSE_PICO_HID_USB_PID, SYNAPSE_PICO_HID_USB_VID,
};
use synapse_hid_host::handshake::{
    HandshakeError, IDENTIFY_RESP_LEN, expected_version_triplet,
    parse_and_validate_identify_response, parse_identify_response,
};

#[test]
fn parse_identify_response_reads_current_wire_layout() {
    let payload = identify_payload(EXPECTED_FW_MAJOR);
    let identity = match parse_and_validate_identify_response(&payload) {
        Ok(identity) => identity,
        Err(error) => panic!("matching identify payload should parse: {error}"),
    };

    assert_eq!(identity.fw_major, EXPECTED_FW_MAJOR);
    assert_eq!(identity.fw_minor, SYNAPSE_PICO_HID_FW_MINOR);
    assert_eq!(identity.fw_patch, SYNAPSE_PICO_HID_FW_PATCH);
    assert_eq!(&identity.build_hash, b"TESTHASH");
    assert_eq!(identity.vid, SYNAPSE_PICO_HID_USB_VID);
    assert_eq!(identity.pid, SYNAPSE_PICO_HID_USB_PID);
    assert_eq!(identity.capabilities, 0x1F);
    assert_eq!(
        expected_version_triplet(),
        [
            EXPECTED_FW_MAJOR,
            SYNAPSE_PICO_HID_FW_MINOR,
            SYNAPSE_PICO_HID_FW_PATCH
        ]
    );
}

#[test]
fn parse_identify_response_rejects_major_version_mismatch() {
    let mismatched_major = EXPECTED_FW_MAJOR.wrapping_add(1);
    let payload = identify_payload(mismatched_major);
    let error = match parse_and_validate_identify_response(&payload) {
        Ok(identity) => panic!("mismatched identify payload should fail: {identity:?}"),
        Err(error) => error,
    };

    assert_eq!(
        error,
        HandshakeError::FirmwareVersionMismatch {
            expected: EXPECTED_FW_MAJOR,
            actual: mismatched_major,
        }
    );
    assert_eq!(error.code(), error_codes::HID_FIRMWARE_VERSION_MISMATCH);
}

#[test]
fn parse_identify_response_rejects_malformed_lengths() {
    let short = [0u8; IDENTIFY_RESP_LEN - 1];
    let short_error = match parse_identify_response(&short) {
        Ok(identity) => panic!("short identify payload should fail: {identity:?}"),
        Err(error) => error,
    };
    assert_eq!(
        short_error,
        HandshakeError::InvalidIdentifyPayloadLength {
            actual: IDENTIFY_RESP_LEN - 1,
            expected: IDENTIFY_RESP_LEN,
        }
    );
    assert_eq!(
        short_error.code(),
        error_codes::HID_PROTOCOL_HANDSHAKE_FAILED
    );

    let long = [0u8; IDENTIFY_RESP_LEN + 1];
    let long_error = match parse_identify_response(&long) {
        Ok(identity) => panic!("long identify payload should fail: {identity:?}"),
        Err(error) => error,
    };
    assert_eq!(
        long_error,
        HandshakeError::InvalidIdentifyPayloadLength {
            actual: IDENTIFY_RESP_LEN + 1,
            expected: IDENTIFY_RESP_LEN,
        }
    );
    assert_eq!(
        long_error.code(),
        error_codes::HID_PROTOCOL_HANDSHAKE_FAILED
    );
}

fn identify_payload(major: u8) -> [u8; IDENTIFY_RESP_LEN] {
    let mut payload = [0u8; IDENTIFY_RESP_LEN];
    payload[0] = major;
    payload[1] = SYNAPSE_PICO_HID_FW_MINOR;
    payload[2] = SYNAPSE_PICO_HID_FW_PATCH;
    payload[4..12].copy_from_slice(b"TESTHASH");
    payload[12..14].copy_from_slice(&SYNAPSE_PICO_HID_USB_VID.to_le_bytes());
    payload[14..16].copy_from_slice(&SYNAPSE_PICO_HID_USB_PID.to_le_bytes());
    payload[16..20].copy_from_slice(&0x1Fu32.to_le_bytes());
    payload
}
