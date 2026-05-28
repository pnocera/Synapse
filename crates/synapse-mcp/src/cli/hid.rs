use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use anyhow::Context;
use clap::{Args, Subcommand};
use serde::Serialize;
use sha2::{Digest, Sha256};
use synapse_core::error_codes;
use synapse_hid_host::{
    FirmwareIdentity, HOST_COMMAND_RESET_TO_BOOTLOADER, HidError, HidGateway, connect_auto,
    expected_version_triplet,
};

const AUTO_PORT: &str = "auto";
const BUNDLED_UF2_NAME: &str = "pico-hid-0.1.0-m4.uf2";
const RPI_RP2_LABEL: &str = "RPI-RP2";
const BOOTLOADER_WAIT: Duration = Duration::from_secs(30);
const REENUMERATE_WAIT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const BUNDLED_UF2: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/release/firmware/pico-hid-0.1.0-m4.uf2"
));

#[derive(Debug, Args)]
pub struct HidCli {
    #[command(subcommand)]
    command: HidCommand,
}

#[derive(Debug, Subcommand)]
enum HidCommand {
    Identify(IdentifyArgs),
    Flash(FlashArgs),
}

#[derive(Debug, Args)]
struct IdentifyArgs {
    #[arg(long, default_value = AUTO_PORT, value_name = "COM7|auto")]
    port: String,
}

#[derive(Debug, Args)]
struct FlashArgs {
    #[arg(long, default_value = AUTO_PORT, value_name = "COM7|auto")]
    port: String,
    #[arg(long, value_name = "PATH")]
    uf2: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
struct Uf2Info {
    kind: &'static str,
    name: String,
    path: Option<String>,
    bytes: usize,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct IdentifyOutput {
    status: &'static str,
    port: String,
    fw_ver: String,
    fw_major: u8,
    fw_minor: u8,
    fw_patch: u8,
    build_hash: String,
    vid: String,
    pid: String,
    capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
struct FlashOutput {
    status: &'static str,
    port_arg: String,
    initial_port: Option<String>,
    reset_to_bootloader_sent: bool,
    bootloader_volume: String,
    copied_to: String,
    uf2: Uf2Info,
    final_identity: IdentifyOutput,
}

#[derive(Debug, Serialize)]
struct ErrorOutput<'a> {
    status: &'static str,
    tool: &'static str,
    code: &'static str,
    detail: String,
    port: Option<String>,
    uf2: Option<&'a Uf2Info>,
    bootloader_volumes: Vec<String>,
}

struct BootloaderStart {
    initial_port: Option<String>,
    reset_to_bootloader_sent: bool,
    bootloader_volume: PathBuf,
}

impl HidCli {
    pub fn run(self) -> anyhow::Result<ExitCode> {
        match self.command {
            HidCommand::Identify(args) => run_identify(&args),
            HidCommand::Flash(args) => run_flash(&args),
        }
    }
}

fn run_identify(args: &IdentifyArgs) -> anyhow::Result<ExitCode> {
    match connect_gateway(&args.port) {
        Ok(gateway) => {
            print_json_stdout(&identity_output(gateway.port_name(), gateway.identity()))?;
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            print_hid_error("hid identify", Some(&args.port), &error, None)?;
            Ok(exit_for_hid_error(&error))
        }
    }
}

fn run_flash(args: &FlashArgs) -> anyhow::Result<ExitCode> {
    let uf2 = match load_uf2(args.uf2.as_deref()) {
        Ok(uf2) => uf2,
        Err(error) => {
            print_error(&ErrorOutput {
                status: "error",
                tool: "hid flash",
                code: error_codes::TOOL_PARAMS_INVALID,
                detail: format!("{error:#}"),
                port: Some(args.port.clone()),
                uf2: None,
                bootloader_volumes: bootloader_volume_strings(),
            })?;
            return Ok(ExitCode::from(2));
        }
    };

    let bootloader = match prepare_bootloader(&args.port, &uf2.info)? {
        Ok(bootloader) => bootloader,
        Err(exit_code) => return Ok(exit_code),
    };

    let copied_to = bootloader.bootloader_volume.join(&uf2.info.name);
    fs::write(&copied_to, &uf2.bytes)
        .with_context(|| format!("copy UF2 to {}", copied_to.display()))?;
    thread::sleep(Duration::from_secs(1));

    let gateway = match wait_for_identity(&args.port, REENUMERATE_WAIT) {
        Ok(gateway) => gateway,
        Err(error) => {
            print_hid_error("hid flash", Some(&args.port), &error, Some(&uf2.info))?;
            return Ok(exit_for_hid_error(&error));
        }
    };
    if !matches_expected_triplet(gateway.identity()) {
        let expected = expected_version_triplet();
        print_error(&ErrorOutput {
            status: "error",
            tool: "hid flash",
            code: error_codes::HID_FIRMWARE_VERSION_MISMATCH,
            detail: format!(
                "firmware version {}.{}.{} did not match bundled {}.{}.{}",
                gateway.identity().fw_major,
                gateway.identity().fw_minor,
                gateway.identity().fw_patch,
                expected[0],
                expected[1],
                expected[2]
            ),
            port: Some(gateway.port_name().to_owned()),
            uf2: Some(&uf2.info),
            bootloader_volumes: bootloader_volume_strings(),
        })?;
        return Ok(ExitCode::from(2));
    }

    let output = FlashOutput {
        status: "ok",
        port_arg: args.port.clone(),
        initial_port: bootloader.initial_port,
        reset_to_bootloader_sent: bootloader.reset_to_bootloader_sent,
        bootloader_volume: bootloader.bootloader_volume.display().to_string(),
        copied_to: copied_to.display().to_string(),
        uf2: uf2.info,
        final_identity: identity_output(gateway.port_name(), gateway.identity()),
    };
    print_json_stdout(&output)?;
    Ok(ExitCode::SUCCESS)
}

fn prepare_bootloader(
    port_arg: &str,
    uf2: &Uf2Info,
) -> anyhow::Result<Result<BootloaderStart, ExitCode>> {
    if let Some(volume) = bootloader_volumes().into_iter().next() {
        return Ok(Ok(BootloaderStart {
            initial_port: None,
            reset_to_bootloader_sent: false,
            bootloader_volume: volume,
        }));
    }

    let mut gateway = match connect_gateway(port_arg) {
        Ok(gateway) => gateway,
        Err(error) => {
            print_hid_error("hid flash", Some(port_arg), &error, Some(uf2))?;
            return Ok(Err(exit_for_hid_error(&error)));
        }
    };
    let port = gateway.port_name().to_owned();
    if let Err(error) = gateway.try_send_command(HOST_COMMAND_RESET_TO_BOOTLOADER, &[]) {
        print_hid_error("hid flash", Some(&port), &error, Some(uf2))?;
        return Ok(Err(exit_for_hid_error(&error)));
    }
    drop(gateway);

    let Some(volume) = wait_for_bootloader_volume(BOOTLOADER_WAIT) else {
        print_error(&ErrorOutput {
            status: "error",
            tool: "hid flash",
            code: error_codes::HID_PORT_NOT_FOUND,
            detail: format!("{RPI_RP2_LABEL} bootloader volume was not observed"),
            port: Some(port_arg.to_owned()),
            uf2: Some(uf2),
            bootloader_volumes: bootloader_volume_strings(),
        })?;
        return Ok(Err(ExitCode::from(1)));
    };

    Ok(Ok(BootloaderStart {
        initial_port: Some(port),
        reset_to_bootloader_sent: true,
        bootloader_volume: volume,
    }))
}

fn connect_gateway(port: &str) -> synapse_hid_host::HidResult<HidGateway> {
    if is_auto_port(port) {
        connect_auto()
    } else {
        HidGateway::connect(port.to_owned())
    }
}

fn wait_for_identity(port: &str, timeout: Duration) -> synapse_hid_host::HidResult<HidGateway> {
    let started = Instant::now();
    let mut last_error = None;
    while started.elapsed() < timeout {
        match connect_gateway(port) {
            Ok(gateway) => return Ok(gateway),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(POLL_INTERVAL);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| HidError::PortNotFound {
        port_name: port.to_owned(),
    }))
}

fn is_auto_port(port: &str) -> bool {
    port.trim().eq_ignore_ascii_case(AUTO_PORT)
}

struct LoadedUf2 {
    info: Uf2Info,
    bytes: Vec<u8>,
}

fn load_uf2(path: Option<&Path>) -> anyhow::Result<LoadedUf2> {
    if let Some(path) = path {
        let bytes =
            fs::read(path).with_context(|| format!("read UF2 override {}", path.display()))?;
        let name = path.file_name().map_or_else(
            || BUNDLED_UF2_NAME.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let info = Uf2Info {
            kind: "override",
            name,
            path: Some(path.display().to_string()),
            bytes: bytes.len(),
            sha256: sha256_hex(&bytes),
        };
        Ok(LoadedUf2 { info, bytes })
    } else {
        let bytes = BUNDLED_UF2.to_vec();
        let info = Uf2Info {
            kind: "bundled",
            name: BUNDLED_UF2_NAME.to_owned(),
            path: Some(format!("include_bytes:{BUNDLED_UF2_NAME}")),
            bytes: bytes.len(),
            sha256: sha256_hex(&bytes),
        };
        Ok(LoadedUf2 { info, bytes })
    }
}

fn wait_for_bootloader_volume(timeout: Duration) -> Option<PathBuf> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if let Some(volume) = bootloader_volumes().into_iter().next() {
            return Some(volume);
        }
        thread::sleep(POLL_INTERVAL);
    }
    None
}

fn bootloader_volume_strings() -> Vec<String> {
    bootloader_volumes()
        .iter()
        .map(|path| path.display().to_string())
        .collect()
}

#[cfg(windows)]
fn bootloader_volumes() -> Vec<PathBuf> {
    ('A'..='Z')
        .filter_map(|letter| {
            let root = format!("{letter}:\\");
            volume_label(&root)
                .filter(|label| label.eq_ignore_ascii_case(RPI_RP2_LABEL))
                .map(|_| PathBuf::from(root))
        })
        .collect()
}

#[cfg(not(windows))]
fn bootloader_volumes() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn volume_label(root: &str) -> Option<String> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows::{
        Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW},
        core::PCWSTR,
    };

    const DRIVE_REMOVABLE: u32 = 2;

    let wide_root = OsStr::new(root)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: `wide_root` is NUL-terminated and lives for the duration of the call.
    if unsafe { GetDriveTypeW(PCWSTR(wide_root.as_ptr())) } != DRIVE_REMOVABLE {
        return None;
    }

    let mut label = [0u16; 64];
    // SAFETY: the input pointer is valid and the output slice is writable.
    unsafe {
        GetVolumeInformationW(
            PCWSTR(wide_root.as_ptr()),
            Some(&mut label),
            None,
            None,
            None,
            None,
        )
    }
    .ok()?;
    let end = label
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(label.len());
    Some(String::from_utf16_lossy(&label[..end]))
}

fn identity_output(port: &str, identity: &FirmwareIdentity) -> IdentifyOutput {
    IdentifyOutput {
        status: "ok",
        port: port.to_owned(),
        fw_ver: format!(
            "{}.{}.{}",
            identity.fw_major, identity.fw_minor, identity.fw_patch
        ),
        fw_major: identity.fw_major,
        fw_minor: identity.fw_minor,
        fw_patch: identity.fw_patch,
        build_hash: hex_bytes(&identity.build_hash),
        vid: format!("0x{:04X}", identity.vid),
        pid: format!("0x{:04X}", identity.pid),
        capabilities: capability_names(identity.capabilities),
    }
}

fn capability_names(bits: u32) -> Vec<String> {
    let mut names = Vec::new();
    for (mask, name) in [
        (1 << 0, "mouse"),
        (1 << 1, "keyboard"),
        (1 << 2, "gamepad"),
        (1 << 3, "telemetry"),
        (1 << 4, "watchdog"),
        (1 << 5, "bootloader"),
    ] {
        if bits & mask != 0 {
            names.push(name.to_owned());
        }
    }
    let known = (1 << 6) - 1;
    let unknown = bits & !known;
    if unknown != 0 {
        names.push(format!("unknown:0x{unknown:08X}"));
    }
    names
}

fn matches_expected_triplet(identity: &FirmwareIdentity) -> bool {
    let expected = expected_version_triplet();
    [identity.fw_major, identity.fw_minor, identity.fw_patch] == expected
}

fn print_hid_error(
    tool: &'static str,
    port: Option<&str>,
    error: &HidError,
    uf2: Option<&Uf2Info>,
) -> anyhow::Result<()> {
    print_error(&ErrorOutput {
        status: "error",
        tool,
        code: error.code(),
        detail: error.to_string(),
        port: port.map(ToOwned::to_owned),
        uf2,
        bootloader_volumes: bootloader_volume_strings(),
    })
}

fn exit_for_hid_error(error: &HidError) -> ExitCode {
    match error {
        HidError::FirmwareVersionMismatch { .. } => ExitCode::from(2),
        HidError::PortNotFound { .. }
        | HidError::PortOpenFailed { .. }
        | HidError::ProtocolHandshakeFailed { .. }
        | HidError::CommandRejected { .. }
        | HidError::QueueFull { .. }
        | HidError::LinkTimeout { .. }
        | HidError::PortDisconnected { .. } => ExitCode::from(1),
    }
}

fn print_json_stdout(value: &impl Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_error(value: &ErrorOutput<'_>) -> anyhow::Result<()> {
    eprintln!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_bytes(&digest)
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02X}");
    }
    output
}
