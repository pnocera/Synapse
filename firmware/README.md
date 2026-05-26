# Synapse Pico HID Firmware

`pico-hid/` is a standalone Cargo workspace for the Raspberry Pi Pico
(RP2040). It is intentionally excluded from the root Synapse workspace because
it targets `thumbv6m-none-eabi` and uses embedded `no_std` dependencies.

## One-Time Toolchain Setup

```powershell
rustup target add thumbv6m-none-eabi
cargo install elf2uf2-rs
```

## Build

```powershell
cd firmware\pico-hid
cargo build --release
```

The project-local `.cargo/config.toml` sets the build target to
`thumbv6m-none-eabi`, so `--target` is not required.

## Create a UF2 Image

```powershell
elf2uf2-rs target\thumbv6m-none-eabi\release\pico-hid pico-hid.uf2
```

## Flash

1. Hold the Pico `BOOTSEL` button while plugging it into USB.
2. Wait for the `RPI-RP2` mass-storage volume to appear.
3. Copy `pico-hid.uf2` to that volume.
4. The Pico reboots automatically. The onboard GP25 LED should blink once per
   second.

Later M4 work adds the CDC ACM command channel, HID descriptors, protocol
parser, watchdog, telemetry, and release-bundled UF2 pipeline.
