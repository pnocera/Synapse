# 09 — Hardware HID Gateway

## 1. Why hardware

`SendInput`, `keybd_event`, `mouse_event`, and ViGEm virtual controllers are software-layer input. A determined detection system can know they're not real peripherals. We want a fallback that is genuinely a real peripheral for three reasons:

1. **Accessibility.** A user with motor impairments using eye-tracking or sip-and-puff input deserves a peripheral the OS treats the same as a real mouse. Synapse becoming that bridge is valuable.
2. **AI research and tournaments.** Sanctioned bot tournaments and university research often require the AI's output to flow through real hardware for fair comparison with human play.
3. **Demo recording / sim rigs.** People building dedicated rigs (sim cockpits, arcade cabinets, modded controllers) want a programmable HID device.

Optional component; Synapse runs without it. For the last 1% of authenticity or a Tier 2 use case (`08_anti_cheat_policy.md` §4.3), build and flash a board.

This doc specifies firmware design, host-side serial driver in `synapse-hid-host`, and wire protocol.

---

## 2. Hardware choices

Three reference platforms. All firmware is Rust, embedded async via `embassy`.

| Board | Cost | Why |
|---|---|---|
| **Raspberry Pi Pico (RP2040)** | ~$4 | Default. Cheap. Easy to source. Stable USB stack via `embassy-usb`. PIO blocks enable USB host later. |
| **Raspberry Pi Pico 2 (RP2350)** | ~$5 | Drop-in newer chip; same firmware with feature flag. |
| **Arduino Pro Micro / Leonardo (ATmega32u4)** | ~$10 | Legacy. Slower. Smaller flash. Stripped subset firmware. |

Default and primary: **Raspberry Pi Pico (RP2040)**. Rest of doc assumes RP2040.

### Bill of materials (minimum viable)

- 1× Raspberry Pi Pico (RP2040, with castellated pads)
- 1× USB-A cable (or USB-A → USB-C) to host PC
- Optional: small project box

No external components. Power and data over the same USB.

---

## 3. Device identity

Board enumerates as a **USB HID composite device** with three interfaces:

| Interface | Class | Subclass | Protocol | What it is |
|---|---|---|---|---|
| 0 | HID (3) | Boot (1) | Mouse (2) | Boot-protocol mouse — works in BIOS, Windows native |
| 1 | HID (3) | Boot (1) | Keyboard (1) | Boot-protocol keyboard |
| 2 | HID (3) | None (0) | None (0) | Vendor-defined gamepad (X-input-compatible report) |

Plus a fourth control interface:

| Interface | Class | Purpose |
|---|---|---|
| 3 | CDC ACM | Serial command channel from Synapse host driver to firmware |

VID/PID defaults:

```
VID: 0x1209  (pid.codes community VID)
PID: 0xC0C0  (Synapse-allocated within the pid.codes range)
```

pid.codes is the open community VID block for hobbyist projects; avoids spoofing any commercial VID/PID. Operators can rebuild firmware with their own — `VID`/`PID`/`MANUFACTURER_STR`/`PRODUCT_STR` are build-time constants.

We deliberately do **not** ship firmware that mimics specific commercial peripherals (Razer DeathAdder VID/PID, Xbox controller VID/PID). Operators wanting that must rebuild firmware with their own choice.

---

## 4. Firmware architecture (RP2040, Rust, embassy)

```
firmware/pico-hid/
├── Cargo.toml
├── memory.x                    # RP2040 linker
├── src/
│   ├── main.rs                 # entry point, embassy executor
│   ├── usb.rs                  # composite device descriptor builder
│   ├── hid_descriptors.rs      # report descriptors (mouse, kbd, pad)
│   ├── reports.rs              # report structs
│   ├── serial.rs               # CDC ACM serial channel
│   ├── protocol.rs             # parser for serial command frames
│   ├── pad_state.rs            # accumulates pad report
│   ├── safety.rs               # watchdog, release_all on link timeout
│   └── led.rs                  # status LED feedback
├── build.rs                    # builds .uf2 image
└── tests/
    └── protocol_roundtrip.rs   # off-board host-side parser tests
```

### 4.1 Embassy executor

```rust
#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = embassy_rp::usb::Driver::new(p.USB, Irqs);

    let mut builder = embassy_usb::Builder::new(driver, /* descriptors */);
    let mouse_handle  = mouse::register(&mut builder);
    let kbd_handle    = keyboard::register(&mut builder);
    let pad_handle    = pad::register(&mut builder);
    let serial_handle = serial::register(&mut builder);

    let mut device = builder.build();
    let (cmd_tx, cmd_rx) = embassy_sync::channel::Channel::new();

    spawner.spawn(device_task(device)).unwrap();
    spawner.spawn(serial_task(serial_handle, cmd_tx)).unwrap();
    spawner.spawn(command_dispatcher(cmd_rx, mouse_handle, kbd_handle, pad_handle)).unwrap();
    spawner.spawn(safety_watchdog()).unwrap();
    spawner.spawn(led_indicator()).unwrap();
}
```

### 4.2 Cooperative loops

| Task | Purpose | Latency target |
|---|---|---|
| `device_task` | USB stack background pump | n/a; embassy-driven |
| `serial_task` | Reads framed bytes from CDC, parses commands, dispatches | ≤ 0.5 ms per command |
| `command_dispatcher` | Applies command to relevant HID interface | ≤ 1 ms host→USB-on-wire |
| `safety_watchdog` | Releases all inputs if no host command in N ms | resolution 50 ms |
| `led_indicator` | Blinks status (idle / active / error) | n/a |

### 4.3 HID descriptors

**Mouse (boot-protocol superset).** Standard boot mouse (3 buttons + X/Y 8-bit deltas) extended with 5 buttons (forward/back) and 16-bit X/Y for higher resolution. Boot-protocol-compatible structure works at BIOS.

**Keyboard (boot-protocol superset).** 8-byte boot keyboard report: modifiers byte + reserved + 6 keycodes. Reports HID Usage IDs directly.

**Gamepad.** XInput-compatible custom HID report:

```
buttons: u16,        // bitfield: A,B,X,Y, LB,RB, Back,Start, LS,RS, DUp,DDown,DLeft,DRight, Guide, Reserved
left_trigger: u8,
right_trigger: u8,
thumb_lx: i16,
thumb_ly: i16,
thumb_rx: i16,
thumb_ry: i16,
```

Total 14 bytes. Sent at up to 1000 Hz (matches XInput poll rate).

---

## 5. Wire protocol (host ↔ firmware)

Host (`synapse-hid-host` crate) talks to firmware over USB CDC ACM at **1 Mbaud** (informational; CDC ACM isn't baud-rate-limited, but most host drivers respect the setting for buffering).

Binary, framed, with explicit acks. Parseable with no allocations on firmware side.

### 5.1 Frame layout

```
+--------+-------+--------+----------+-------+-----+
| MAGIC  | LEN   | SEQ    | CMD      | PAYLOAD| CRC|
| 0x5A   | u16le | u32le  | u8       | bytes  | u16le|
+--------+-------+--------+----------+-------+-----+
```

- `MAGIC`: 0x5A (sync; firmware resyncs by skipping bytes until it sees this)
- `LEN`: total frame length excluding `MAGIC`, including `CRC`
- `SEQ`: monotonic sequence number assigned by host
- `CMD`: command identifier
- `PAYLOAD`: command-specific
- `CRC`: CRC16/CCITT-FALSE over `LEN..CRC`

### 5.2 Commands (host → firmware)

| `CMD` | Name | Payload | Effect |
|---|---|---|---|
| 0x01 | `PING` | `[u32 nonce]` | firmware echoes `PONG` with same nonce |
| 0x02 | `IDENTIFY` | empty | firmware replies with `IDENTIFY_RESP { fw_ver, build_hash, vid, pid, capabilities_mask }` |
| 0x10 | `MOUSE_MOVE_REL` | `[i16 dx][i16 dy]` | mouse delta |
| 0x11 | `MOUSE_BUTTON` | `[u8 button][u8 down_flag]` | button state |
| 0x12 | `MOUSE_WHEEL` | `[i8 dy][i8 dx]` | wheel ticks |
| 0x20 | `KEY_DOWN` | `[u8 hid_code]` | keyboard key down |
| 0x21 | `KEY_UP` | `[u8 hid_code]` | keyboard key up |
| 0x22 | `KEY_MODS` | `[u8 mods_bitfield]` | set modifier state directly |
| 0x30 | `PAD_REPORT` | `[14 bytes raw report]` | apply pad report |
| 0x40 | `RELEASE_ALL` | empty | all mouse buttons up, all keys up, pad neutral |
| 0x50 | `WATCHDOG_KICK` | `[u32 timeout_ms]` | reset watchdog with new timeout |
| 0x60 | `GET_TELEMETRY` | empty | replies with `TELEMETRY_RESP { uptime_ms, frames_received, frames_dropped, link_errors }` |
| 0xF0 | `RESET_TO_BOOTLOADER` | empty | enters UF2 bootloader (for re-flashing) |

### 5.3 Responses (firmware → host)

Same frame layout, `MAGIC = 0xA5` (mirror byte) to distinguish direction.

| `CMD` | Name | Payload |
|---|---|---|
| 0x80 | `ACK` | `[u32 seq_acked]` |
| 0x81 | `NAK` | `[u32 seq_acked][u8 reason_code]` |
| 0x82 | `PONG` | `[u32 nonce]` |
| 0x83 | `IDENTIFY_RESP` | (see above) |
| 0x84 | `TELEMETRY_RESP` | (see above) |
| 0x90 | `EVENT_BUTTON_PRESS_LOCAL` | (reserved; future: physical buttons on the board) |

### 5.4 Sequence numbers and ack semantics

Host assigns monotonic `SEQ`. Firmware acks every accepted frame within ≤ 200 µs. Host considers a frame failed if no ACK within 5 ms; resends with same `SEQ`. After 3 retries, host raises `HID_LINK_TIMEOUT`; surfaces `ACTION_HID_PORT_DISCONNECTED` to caller.

For volume input (e.g., a curve emitting 50 small mouse moves), host can pipeline up to 16 outstanding unacked frames. Firmware buffers up to 64 frames; overflow returns `NAK { reason: BUFFER_FULL }`.

### 5.5 NAK reason codes

```
0x01 NAK_CRC_INVALID
0x02 NAK_LEN_INVALID
0x03 NAK_UNKNOWN_CMD
0x04 NAK_PAYLOAD_INVALID
0x05 NAK_BUFFER_FULL
0x06 NAK_WATCHDOG_EXPIRED       // firmware refused; watchdog had already released all
```

### 5.6 Frame loss handling

USB CDC ACM is reliable in practice. CRC + ack detects protocol bugs and link-level glitches (cable disconnect, unplug-replug). Frame loss is not expected during normal operation.

---

## 6. Safety: the watchdog

Firmware enforces a watchdog. If no command received within `WATCHDOG_TIMEOUT_MS` (default 1000 ms):

1. Log event internally (telemetry counter increments)
2. Issue internal `RELEASE_ALL` — all mouse buttons up, all keys up, gamepad neutral
3. Continue running, ready for new commands

Prevents stuck inputs if the host process crashes or USB link freezes mid-action.

Host can: tune timeout via `WATCHDOG_KICK`; disable by setting timeout to 0 (not recommended; safety machinery); receive a `link_state_changed` event from `synapse-hid-host` on watchdog fire.

---

## 7. Host-side driver (`synapse-hid-host`)

```rust
pub struct HidGateway {
    port: SerialPort,         // serialport crate handle
    seq: AtomicU32,
    inflight: Mutex<HashMap<u32, oneshot::Sender<Result<Ack>>>>,
    rx_task: JoinHandle<()>,
}

impl HidGateway {
    pub fn connect(port_name: &str) -> Result<Self> {
        let port = serialport::new(port_name, 1_000_000)
            .timeout(Duration::from_millis(5))
            .data_bits(serialport::DataBits::Eight)
            .stop_bits(serialport::StopBits::One)
            .parity(serialport::Parity::None)
            .open()?;
        // Identity handshake
        let identity = handshake(&mut port)?;
        validate_fw_version(&identity)?;
        // Spawn rx task
        let rx_task = tokio::task::spawn_blocking(move || rx_loop(/* ... */));
        Ok(HidGateway { port, /* ... */ })
    }

    pub async fn mouse_move(&self, dx: i16, dy: i16) -> Result<()> {
        self.send_command(Cmd::MouseMoveRel { dx, dy }).await
    }

    pub async fn key_press(&self, hid_code: u8, hold: Duration) -> Result<()> {
        self.send_command(Cmd::KeyDown { hid_code }).await?;
        tokio::time::sleep(hold).await;
        self.send_command(Cmd::KeyUp { hid_code }).await
    }

    // ...
}
```

Threading: one blocking I/O thread for serial reads (`serialport` is sync); pushes parsed responses through a channel into tokio. Writes are async-from-tokio with a small `Mutex<SerialPort>` to serialize.

### 7.1 Auto-detect

`synapse-mcp` at startup with `--hardware-hid auto` enumerates COM ports, sends `IDENTIFY` to each, finds Synapse firmware by `IDENTIFY_RESP` payload. First match wins; error if none.

### 7.2 Reconnection

On serial error (port closed, USB unplugged), driver retries every 500 ms. While disconnected, all action calls using `Backend::Hardware` return `ACTION_HID_PORT_DISCONNECTED` immediately (no queueing).

### 7.3 Firmware version handshake

`IDENTIFY_RESP` includes `fw_ver` (semver) and `build_hash` (8 bytes). Host compares `fw_ver.major` against compiled-in `EXPECTED_FW_MAJOR`. Mismatch returns `HID_FIRMWARE_VERSION_MISMATCH` and aborts. Operator runs `synapse-mcp hid flash` to update.

---

## 8. Building and flashing the firmware

```powershell
# One-time
rustup target add thumbv6m-none-eabi
cargo install elf2uf2-rs

# Build
cd firmware/pico-hid
cargo build --release --target thumbv6m-none-eabi
elf2uf2-rs target/thumbv6m-none-eabi/release/pico-hid pico-hid.uf2

# Flash
# 1. Hold BOOTSEL on the Pico while plugging USB
# 2. Pico appears as a USB mass storage device "RPI-RP2"
# 3. Copy pico-hid.uf2 to it; Pico reboots into Synapse firmware
```

Helper: `synapse-mcp hid flash --port COM7`:

1. Detect if device is in Synapse firmware mode (sends `IDENTIFY`).
2. If yes, send `RESET_TO_BOOTLOADER` to reboot into UF2.
3. Wait for mass storage to appear.
4. Copy bundled `pico-hid.uf2`.
5. Wait for re-enumeration as Synapse firmware.
6. Verify with `IDENTIFY`.

Bundled `.uf2` files are released as GitHub release assets per Synapse version, signed by the project key.

---

## 9. Power and electrical

- USB bus-powered. ~50 mA under load. Pico's regulator handles 5 V input fine.
- No external components for reference design.
- Optional: tactile button on GP0 as emergency unplug (firmware reads; on press, sends `RELEASE_ALL` and clears state).

Status LED:

| LED state | Meaning |
|---|---|
| Off | Idle, watchdog not running |
| Slow heartbeat (1 Hz) | Connected, no recent commands |
| Steady on | Receiving commands actively |
| Fast blink (5 Hz) | Watchdog fired (released all) |
| SOS pattern | Firmware error; reflash needed |

---

## 10. Performance budget

| Stage | Target p99 |
|---|---|
| Host: action → serial bytes on wire | ≤ 200 µs |
| USB CDC bus latency (full-speed USB) | ~1 ms (USB poll interval) |
| Firmware: parse frame → HID report ready | ≤ 100 µs |
| Firmware: HID report → on the USB IN endpoint | next 1 ms poll |
| End-to-end: host call → physical USB IN packet | ≤ 4 ms p99 |

1 ms USB poll is the hard floor. Hardware HID will always be ~3 ms slower than software `SendInput` (which doesn't go over USB). The cost of authenticity.

---

## 11. Testing the firmware

| Test | How |
|---|---|
| Protocol roundtrip | `cargo test -p pico-hid --tests` (host-side parser tests with hand-crafted frames) |
| Firmware loopback | Build with `--features loopback`; firmware echoes every command back as `PONG`. Host driver sends 1000 commands, asserts all return. |
| Watchdog | Connect, send commands, stop >1s, observe `RELEASE_ALL` via internal telemetry. |
| Stress | Send 10,000 mouse-move-rel commands at full rate; assert no drops, all acked. |
| Re-enumeration | Trigger `RESET_TO_BOOTLOADER`, observe device drops, mass storage appears, reflash, reconnect. |

CI runs protocol roundtrip tests on Linux (no hardware required). Firmware-loopback runs weekly on a self-hosted CI machine with a Pico attached.

---

## 12. Limitations and notes

- **Full-speed USB only.** No high-speed USB 2.0 on the Pico. Fine for HID; insufficient for video streaming, but we don't.
- **Single boot-mouse / boot-keyboard** at a time. Windows accepts one of each composite device. Don't plug in multiple Synapse boards.
- **Mouse resolution.** 16-bit signed delta per axis. Large moves split into many small reports anyway.
- **Latency stable on Win11 22H2+.** Older Windows builds may have USB poll jitter (1 ms poll effectively 1-3 ms). Not Synapse's problem to fix.
- **PIO USB host (advanced).** The Pico's PIO blocks can run a second USB host port (see `vynxc/VBox`). Synapse v1 doesn't ship this; v2 option for "pass through a real mouse and inject corrections."

---

## 13. What this doc does NOT cover

- Anti-cheat policy gating around hardware backend → `08_anti_cheat_policy.md`
- High-level action API routing to hardware → `03_action.md`
- Action serialization invariants → `03_action.md` §4
- Build pipeline / installer integration → `14_build_and_packaging.md`
