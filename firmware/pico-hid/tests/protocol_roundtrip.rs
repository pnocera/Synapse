use pico_hid::protocol::{
    DropReason, EncodeError, HOST_MAGIC, HostCommand, MAX_FRAME_LEN, MAX_PAYLOAD_LEN, NakReason,
    ParseResult, crc16_ccitt_false, encode_host_frame, next_sequence, parse_host_frame,
};

#[test]
fn crc16_ccitt_false_known_check_value() {
    assert_eq!(crc16_ccitt_false(b"123456789"), 0x29B1);
}

#[test]
fn malformed_frames_return_drop_or_nak_state() {
    let mut frame = [0u8; MAX_FRAME_LEN];
    let payload = [0xAA, 0x55, 0x10];
    let len = encode_host_frame(7, HostCommand::Ping, &payload, &mut frame).unwrap();

    let before_bad_magic = frame[..len].to_vec();
    frame[0] = 0x00;
    assert_eq!(
        parse_host_frame(&frame[..len]),
        ParseResult::Drop {
            reason: DropReason::BadMagic,
            consumed: 1
        }
    );
    frame[..len].copy_from_slice(&before_bad_magic);

    frame[len - 1] ^= 0x01;
    assert_eq!(
        parse_host_frame(&frame[..len]),
        ParseResult::Nak {
            nak: pico_hid::protocol::Nak {
                seq: 7,
                command: HostCommand::Ping as u8,
                reason: NakReason::CrcInvalid,
            },
            consumed: len,
        }
    );

    let overflow = [HOST_MAGIC, 0xFF, 0xFF];
    assert_eq!(
        parse_host_frame(&overflow),
        ParseResult::Drop {
            reason: DropReason::LenOverflow,
            consumed: 1
        }
    );
}

#[test]
fn sequence_wraparound_uses_wrapping_add() {
    assert_eq!(next_sequence(u32::MAX), 0);
    assert_eq!(next_sequence(0), 1);
}

#[test]
fn encode_rejects_payload_or_output_overflow() {
    let payload = [0u8; MAX_PAYLOAD_LEN + 1];
    let mut frame = [0u8; MAX_FRAME_LEN];
    assert_eq!(
        encode_host_frame(1, HostCommand::Ping, &payload, &mut frame),
        Err(EncodeError::PayloadTooLarge)
    );

    let payload = [0u8; 4];
    let mut too_small = [0u8; 4];
    assert_eq!(
        encode_host_frame(1, HostCommand::Ping, &payload, &mut too_small),
        Err(EncodeError::OutputTooSmall { needed: 14 })
    );
}

#[test]
fn protocol_roundtrips_1000_deterministic_frames() {
    let mut frame = [0u8; MAX_FRAME_LEN];
    let mut payload = [0u8; 128];
    let mut seed = 0x0C0D_E373_u32;

    for seq in 0..1000u32 {
        seed = lcg(seed);
        let payload_len = (seed as usize) % payload.len();
        for byte in &mut payload[..payload_len] {
            seed = lcg(seed);
            *byte = (seed >> 24) as u8;
        }

        let len = encode_host_frame(
            seq,
            HostCommand::PadReport,
            &payload[..payload_len],
            &mut frame,
        )
        .unwrap();

        match parse_host_frame(&frame[..len]) {
            ParseResult::Frame {
                frame: parsed,
                consumed,
            } => {
                assert_eq!(consumed, len);
                assert_eq!(parsed.seq, seq);
                assert_eq!(parsed.command, HostCommand::PadReport as u8);
                assert_eq!(parsed.payload, &payload[..payload_len]);
            }
            other => panic!("expected frame for seq {seq}, got {other:?}"),
        }
    }
}

fn lcg(seed: u32) -> u32 {
    seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}
