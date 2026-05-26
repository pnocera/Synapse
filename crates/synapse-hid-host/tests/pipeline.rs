use std::io::{self, ErrorKind, Read, Write};

use synapse_core::error_codes;
use synapse_hid_host::{
    ACK_RETRY_BACKOFF_MS, ACK_TIMEOUT_MS, DEVICE_COMMAND_ACK, DEVICE_COMMAND_NAK,
    HOST_COMMAND_MOUSE_MOVE_REL, HOST_MAGIC, HidError, HidPipeline, MAX_ACK_RETRIES, MAX_FRAME_LEN,
    MAX_OUTSTANDING_FRAMES, NAK_REASON_BUFFER_FULL, NAK_REASON_PAYLOAD_INVALID, PipelineConfig,
    encode_device_frame,
};

#[test]
fn pipeline_defaults_match_m4_contract() {
    let config = PipelineConfig::default();

    assert_eq!(config.max_outstanding, 16);
    assert_eq!(config.max_outstanding, MAX_OUTSTANDING_FRAMES);
    assert_eq!(config.ack_timeout_ms, 5);
    assert_eq!(config.ack_timeout_ms, ACK_TIMEOUT_MS);
    assert_eq!(config.max_retries, 3);
    assert_eq!(config.max_retries, MAX_ACK_RETRIES);
    assert_eq!(config.retry_backoff_ms, [5, 10, 20]);
    assert_eq!(config.retry_backoff_ms, ACK_RETRY_BACKOFF_MS);
}

#[test]
fn send_commands_writes_sixteen_frames_before_first_ack_read() {
    let mut responses = Vec::new();
    for seq in 1..=20 {
        responses.extend_from_slice(&ack(seq));
    }
    let mut transport = ScriptedTransport::new(responses);
    let commands = vec![move_rel_command(); 20];
    let mut pipeline = HidPipeline::new();

    let seqs = match pipeline.send_commands(&mut transport, &commands) {
        Ok(seqs) => seqs,
        Err(error) => panic!("twenty ACKed commands should pass: {error}"),
    };

    assert_eq!(seqs, (1..=20).collect::<Vec<u32>>());
    assert_eq!(transport.first_read_write_count, Some(16));
    assert_eq!(transport.written.len(), 20);
    assert_eq!(host_frame_seq(&transport.written[0]), 1);
    assert_eq!(host_frame_seq(&transport.written[15]), 16);
    assert_eq!(host_frame_seq(&transport.written[19]), 20);
}

#[test]
fn nak_retries_same_sequence_once_and_succeeds() {
    let mut responses = Vec::new();
    responses.extend_from_slice(&nak(1, NAK_REASON_BUFFER_FULL));
    responses.extend_from_slice(&ack(1));
    let mut transport = ScriptedTransport::new(responses);
    let mut pipeline = HidPipeline::new();

    let seq =
        match pipeline.send_command(&mut transport, HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0]) {
            Ok(seq) => seq,
            Err(error) => panic!("single NAK followed by ACK should pass: {error}"),
        };

    assert_eq!(seq, 1);
    assert_eq!(transport.written.len(), 2);
    assert_eq!(transport.written[0], transport.written[1]);
    assert_eq!(host_frame_seq(&transport.written[0]), 1);
}

#[test]
fn timeout_retries_three_times_then_returns_link_timeout() {
    let mut transport = ScriptedTransport::new(Vec::new());
    let mut pipeline = HidPipeline::with_config(PipelineConfig {
        max_outstanding: MAX_OUTSTANDING_FRAMES,
        ack_timeout_ms: 0,
        max_retries: MAX_ACK_RETRIES,
        retry_backoff_ms: [0, 0, 0],
    });

    let error =
        match pipeline.send_command(&mut transport, HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0]) {
            Ok(seq) => panic!("silent transport should time out, accepted seq {seq}"),
            Err(error) => error,
        };

    assert_eq!(
        error,
        HidError::LinkTimeout {
            operation: "waiting for ACK",
            timeout_ms: 0,
        }
    );
    assert_eq!(error.code(), error_codes::HID_LINK_TIMEOUT);
    assert_eq!(transport.written.len(), 4);
}

#[test]
fn malformed_ack_payload_seq_is_rejected() {
    let payload_seq = 2u32.to_le_bytes();
    let mut frame = [0u8; MAX_FRAME_LEN];
    let len = match encode_device_frame(1, DEVICE_COMMAND_ACK, &payload_seq, &mut frame) {
        Ok(len) => len,
        Err(error) => panic!("malformed ACK test frame should encode: {error:?}"),
    };
    let mut transport = ScriptedTransport::new(frame[..len].to_vec());
    let mut pipeline = HidPipeline::new();

    let error =
        match pipeline.send_command(&mut transport, HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0]) {
            Ok(seq) => panic!("mismatched ACK payload should fail, accepted seq {seq}"),
            Err(error) => error,
        };

    assert_eq!(
        error,
        HidError::CommandRejected {
            seq: 1,
            command: DEVICE_COMMAND_ACK,
            reason: NAK_REASON_PAYLOAD_INVALID,
        }
    );
    assert_eq!(error.code(), error_codes::HID_COMMAND_REJECTED);
}

const fn move_rel_command() -> synapse_hid_host::HostCommandRequest<'static> {
    synapse_hid_host::HostCommandRequest::new(HOST_COMMAND_MOUSE_MOVE_REL, &[1, 0, 2, 0])
}

fn ack(seq: u32) -> Vec<u8> {
    let payload = seq.to_le_bytes();
    let mut frame = [0u8; MAX_FRAME_LEN];
    let len = match encode_device_frame(seq, DEVICE_COMMAND_ACK, &payload, &mut frame) {
        Ok(len) => len,
        Err(error) => panic!("ACK frame should encode: {error:?}"),
    };
    frame[..len].to_vec()
}

fn nak(seq: u32, reason: u8) -> Vec<u8> {
    let mut payload = [0u8; 5];
    payload[..4].copy_from_slice(&seq.to_le_bytes());
    payload[4] = reason;
    let mut frame = [0u8; MAX_FRAME_LEN];
    let len = match encode_device_frame(seq, DEVICE_COMMAND_NAK, &payload, &mut frame) {
        Ok(len) => len,
        Err(error) => panic!("NAK frame should encode: {error:?}"),
    };
    frame[..len].to_vec()
}

fn host_frame_seq(frame: &[u8]) -> u32 {
    assert_eq!(frame[0], HOST_MAGIC);
    u32::from_le_bytes([frame[3], frame[4], frame[5], frame[6]])
}

struct ScriptedTransport {
    read_data: Vec<u8>,
    read_offset: usize,
    written: Vec<Vec<u8>>,
    first_read_write_count: Option<usize>,
}

impl ScriptedTransport {
    const fn new(read_data: Vec<u8>) -> Self {
        Self {
            read_data,
            read_offset: 0,
            written: Vec::new(),
            first_read_write_count: None,
        }
    }
}

impl Read for ScriptedTransport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.first_read_write_count.is_none() {
            self.first_read_write_count = Some(self.written.len());
        }

        if self.read_offset >= self.read_data.len() {
            return Err(io::Error::new(ErrorKind::TimedOut, "scripted timeout"));
        }

        let remaining = self.read_data.len() - self.read_offset;
        let count = remaining.min(buffer.len());
        buffer[..count]
            .copy_from_slice(&self.read_data[self.read_offset..self.read_offset + count]);
        self.read_offset += count;
        Ok(count)
    }
}

impl Write for ScriptedTransport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.written.push(buffer.to_vec());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
