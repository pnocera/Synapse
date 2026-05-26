use std::fmt;
use std::time::Duration;

use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};
use tracing::debug;

use crate::error::{HidError, HidResult};
use crate::handshake::{FirmwareIdentity, IDENTIFY_TIMEOUT_MS, perform_identify_handshake};
use crate::pipeline::{HidPipeline, HostCommandRequest};

pub const DEFAULT_BAUD_RATE: u32 = 1_000_000;
pub const DEFAULT_READ_TIMEOUT_MS: u64 = 5;

pub struct HidGateway {
    port_name: String,
    baud_rate: u32,
    read_timeout: Duration,
    identity: FirmwareIdentity,
    pipeline: HidPipeline,
    port: Box<dyn SerialPort>,
}

impl HidGateway {
    /// Opens the configured CDC ACM serial port for the Pico HID gateway.
    ///
    /// # Errors
    ///
    /// Returns [`HidError::PortNotFound`] when the requested port is not
    /// present in serial-port enumeration, or [`HidError::PortOpenFailed`]
    /// when the OS reports an error while opening an enumerated port.
    pub fn connect(port_name: impl Into<String>) -> HidResult<Self> {
        let port_name = port_name.into();
        let read_timeout = Duration::from_millis(DEFAULT_READ_TIMEOUT_MS);

        if !port_is_present(&port_name) {
            return Err(HidError::PortNotFound { port_name });
        }

        let mut port = serialport::new(&port_name, DEFAULT_BAUD_RATE)
            .timeout(read_timeout)
            .data_bits(DataBits::Eight)
            .flow_control(FlowControl::None)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .open()
            .map_err(|error| port_open_failed(port_name.clone(), error))?;
        let identity =
            perform_identify_handshake(port.as_mut(), Duration::from_millis(IDENTIFY_TIMEOUT_MS))?;

        debug!(
            port_name = %port_name,
            baud_rate = DEFAULT_BAUD_RATE,
            read_timeout_ms = DEFAULT_READ_TIMEOUT_MS,
            "opened HID CDC serial port"
        );

        Ok(Self {
            port_name,
            baud_rate: DEFAULT_BAUD_RATE,
            read_timeout,
            identity,
            pipeline: HidPipeline::default(),
            port,
        })
    }

    #[must_use]
    pub fn port_name(&self) -> &str {
        &self.port_name
    }

    #[must_use]
    pub const fn baud_rate(&self) -> u32 {
        self.baud_rate
    }

    #[must_use]
    pub const fn read_timeout(&self) -> Duration {
        self.read_timeout
    }

    #[must_use]
    pub const fn identity(&self) -> &FirmwareIdentity {
        &self.identity
    }

    pub fn serial_port_mut(&mut self) -> &mut dyn SerialPort {
        self.port.as_mut()
    }

    /// Sends one ACK/NAK command through the HID serial pipeline.
    ///
    /// # Errors
    ///
    /// Returns [`HidError::LinkTimeout`] when the command exhausts the ACK
    /// retry budget, or [`HidError::CommandRejected`] when the firmware returns
    /// a malformed/rejected response.
    pub fn send_command(&mut self, command: u8, payload: &[u8]) -> HidResult<u32> {
        self.pipeline
            .send_command(self.port.as_mut(), command, payload)
    }

    /// Sends a batch of ACK/NAK commands with the M4 sliding window.
    ///
    /// # Errors
    ///
    /// Returns [`HidError::LinkTimeout`] when any command exhausts the ACK
    /// retry budget, or [`HidError::CommandRejected`] when the firmware returns
    /// a malformed/rejected response.
    pub fn send_commands(&mut self, commands: &[HostCommandRequest<'_>]) -> HidResult<Vec<u32>> {
        self.pipeline.send_commands(self.port.as_mut(), commands)
    }
}

impl fmt::Debug for HidGateway {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HidGateway")
            .field("port_name", &self.port_name)
            .field("baud_rate", &self.baud_rate)
            .field("read_timeout", &self.read_timeout)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

fn port_is_present(port_name: &str) -> bool {
    match serialport::available_ports() {
        Ok(ports) => ports
            .iter()
            .any(|candidate| port_names_match(&candidate.port_name, port_name)),
        Err(error) => {
            debug!(?error, "serial-port enumeration failed before open");
            true
        }
    }
}

fn port_names_match(left: &str, right: &str) -> bool {
    normalize_port_name(left).eq_ignore_ascii_case(normalize_port_name(right))
}

fn normalize_port_name(port_name: &str) -> &str {
    port_name.strip_prefix("\\\\.\\").unwrap_or(port_name)
}

fn port_open_failed(port_name: String, error: serialport::Error) -> HidError {
    let os_error_code = parse_os_error_code(&error.description);
    HidError::PortOpenFailed {
        port_name,
        kind: error.kind(),
        detail: error.description,
        os_error_code,
    }
}

fn parse_os_error_code(description: &str) -> Option<i32> {
    let rest = description.strip_prefix("OS Error ")?;
    let number = rest.split_whitespace().next()?;
    number.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{normalize_port_name, parse_os_error_code, port_names_match};

    #[test]
    fn port_name_matching_accepts_windows_device_prefix_and_case() {
        assert!(port_names_match(r"\\.\COM7", "com7"));
        assert!(port_names_match("COM12", r"\\.\com12"));
        assert!(!port_names_match("COM12", "COM13"));
    }

    #[test]
    fn os_error_code_parser_reads_serialport_fallback_text() {
        assert_eq!(
            parse_os_error_code("OS Error 5 (FormatMessageW() returned error 9)"),
            Some(5)
        );
        assert_eq!(parse_os_error_code("Access is denied."), None);
    }

    #[test]
    fn normalize_port_name_keeps_plain_port_names() {
        assert_eq!(normalize_port_name("COM7"), "COM7");
        assert_eq!(normalize_port_name(r"\\.\COM7"), "COM7");
    }
}
