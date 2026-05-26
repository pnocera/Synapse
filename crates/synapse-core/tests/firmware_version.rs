use synapse_core::{
    EXPECTED_FW_MAJOR, SYNAPSE_PICO_HID_BUILD_HASH_LEN, SYNAPSE_PICO_HID_FW_MAJOR,
    SYNAPSE_PICO_HID_FW_MINOR, SYNAPSE_PICO_HID_FW_PATCH,
};

#[test]
fn expected_firmware_major_matches_current_pico_major() {
    assert_eq!(EXPECTED_FW_MAJOR, SYNAPSE_PICO_HID_FW_MAJOR);
    assert_eq!(SYNAPSE_PICO_HID_FW_MAJOR, 0);
    assert_eq!(SYNAPSE_PICO_HID_FW_MINOR, 1);
    assert_eq!(SYNAPSE_PICO_HID_FW_PATCH, 0);
    assert_eq!(SYNAPSE_PICO_HID_BUILD_HASH_LEN, 8);
}
