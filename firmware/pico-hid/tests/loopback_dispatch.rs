#![cfg(feature = "loopback")]

use pico_hid::dispatch::{DispatchState, IdentifyInfo, MAX_RESPONSE_PAYLOAD_LEN, dispatch_frame};
use pico_hid::protocol::{
    DeviceCommand, HOST_MAGIC, HostCommand, MAX_FRAME_LEN, MAX_PAYLOAD_LEN, NakReason, ParseResult,
    encode_device_frame, encode_frame, parse_device_frame, parse_host_frame,
    parse_host_frame_any_command,
};
use pico_hid::reports::{GAMEPAD_REPORT_LEN, GamepadReport};

#[test]
fn loopback_dispatch_pongs_without_mutating_hid_state() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();
    state.mouse.buttons = 1;
    state.keyboard.modifiers = 0x02;
    state.gamepad = GamepadReport {
        buttons: 0x0001,
        left_trigger: 2,
        right_trigger: 3,
        thumb_lx: -4,
        thumb_ly: 5,
        thumb_rx: -6,
        thumb_ry: 7,
        reserved: 0,
    };

    let mouse_before = state.mouse;
    let keyboard_before = state.keyboard;
    let gamepad_before = state.gamepad;
    let payload = [0x10, 0x20, 0x30, 0x40, 0x50];
    let outcome = dispatch_frame(
        &mut state,
        frame(1, HostCommand::MouseMoveRel as u8, &payload),
        identify,
    );

    assert_eq!(outcome.command, DeviceCommand::Pong);
    assert_eq!(outcome.payload_len, payload.len());
    assert_eq!(&outcome.payload[..outcome.payload_len], payload);
    assert_eq!(state.mouse, mouse_before);
    assert_eq!(state.keyboard, keyboard_before);
    assert_eq!(state.gamepad, gamepad_before);
    assert_eq!(state.telemetry.frames_received, 1);
    assert_eq!(state.telemetry.commands_executed, 1);
    assert_eq!(state.telemetry.link_errors, 0);
}

#[test]
fn loopback_dispatch_accepts_empty_and_max_payloads() {
    assert_eq!(MAX_RESPONSE_PAYLOAD_LEN, MAX_PAYLOAD_LEN);

    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();
    let empty = dispatch_frame(
        &mut state,
        frame(2, HostCommand::ReleaseAll as u8, &[]),
        identify,
    );
    assert_eq!(empty.command, DeviceCommand::Pong);
    assert_eq!(empty.payload_len, 0);

    let payload = [0xA5u8; MAX_PAYLOAD_LEN];
    let full = dispatch_frame(
        &mut state,
        frame(3, HostCommand::Ping as u8, &payload),
        identify,
    );
    assert_eq!(full.command, DeviceCommand::Pong);
    assert_eq!(full.payload_len, MAX_PAYLOAD_LEN);
    assert_eq!(&full.payload[..full.payload_len], payload);
}

#[test]
fn loopback_parser_keeps_normal_parser_strict() {
    let mut host_frame = [0u8; MAX_FRAME_LEN];
    let len = encode_frame(HOST_MAGIC, 77, 0xFE, &[1, 2, 3], &mut host_frame).unwrap();

    match parse_host_frame(&host_frame[..len]) {
        ParseResult::Nak { nak, consumed } => {
            assert_eq!(consumed, len);
            assert_eq!(nak.seq, 77);
            assert_eq!(nak.command, 0xFE);
            assert_eq!(nak.reason, NakReason::UnknownCommand);
        }
        other => panic!("strict parser should reject unknown command, got {other:?}"),
    }

    match parse_host_frame_any_command(&host_frame[..len]) {
        ParseResult::Frame {
            frame: parsed,
            consumed,
        } => {
            assert_eq!(consumed, len);
            assert_eq!(parsed.seq, 77);
            assert_eq!(parsed.command, 0xFE);
            assert_eq!(parsed.payload, &[1, 2, 3]);
        }
        other => panic!("loopback parser should accept unknown command, got {other:?}"),
    }
}

#[test]
fn loopback_protocol_roundtrips_1000_pongs_in_order() {
    let mut host_frame = [0u8; MAX_FRAME_LEN];
    let mut device_frame = [0u8; MAX_FRAME_LEN];
    let mut payload = [0u8; GAMEPAD_REPORT_LEN];

    for seq in 0..1000u32 {
        let payload_len = ((seq as usize) * 7) % (payload.len() + 1);
        for (index, byte) in payload[..payload_len].iter_mut().enumerate() {
            *byte = seq.wrapping_add(index as u32).to_le_bytes()[0];
        }

        let command = match seq % 4 {
            0 => HostCommand::Ping as u8,
            1 => HostCommand::PadReport as u8,
            2 => HostCommand::ReleaseAll as u8,
            _ => 0xFE,
        };
        let host_len = encode_frame(
            HOST_MAGIC,
            seq,
            command,
            &payload[..payload_len],
            &mut host_frame,
        )
        .unwrap();

        let parsed = match parse_host_frame_any_command(&host_frame[..host_len]) {
            ParseResult::Frame {
                frame: parsed,
                consumed,
            } => {
                assert_eq!(consumed, host_len);
                parsed
            }
            other => panic!("expected host frame for seq {seq}, got {other:?}"),
        };

        let device_len = encode_device_frame(
            parsed.seq,
            DeviceCommand::Pong,
            parsed.payload,
            &mut device_frame,
        )
        .unwrap();

        match parse_device_frame(&device_frame[..device_len]) {
            ParseResult::Frame {
                frame: returned,
                consumed,
            } => {
                assert_eq!(consumed, device_len);
                assert_eq!(returned.seq, seq);
                assert_eq!(returned.command, DeviceCommand::Pong as u8);
                assert_eq!(returned.payload, &payload[..payload_len]);
            }
            other => panic!("expected device PONG for seq {seq}, got {other:?}"),
        }
    }
}

fn frame<'a>(seq: u32, command: u8, payload: &'a [u8]) -> pico_hid::protocol::Frame<'a> {
    pico_hid::protocol::Frame {
        seq,
        command,
        payload,
    }
}
