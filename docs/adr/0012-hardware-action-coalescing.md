# ADR-0012: Hardware Action Coalescing

## Status

Accepted 2026-05-28.

## Context

Synapse hardware mouse output reaches the RP2040 firmware as
`MOUSE_MOVE_REL` commands over the CDC command channel. The firmware then
updates a boot-mouse HID report, whose X/Y fields are bounded to `-127..=127`
per axis. The hardware path is intentionally different from software
`SendInput`: the OS receives a physical USB HID device.

USB HID mouse reports are ultimately constrained by endpoint polling cadence.
The HID class descriptor examples define `bInterval` as the endpoint polling
interval for transfers, and Windows documents full-speed interrupt endpoint
periods in 1 ms frames. Synapse's Pico docs and firmware target a 1 ms HID poll
floor. Sending multiple tiny relative moves for the same curve segment faster
than that poll cadence increases CDC/HID traffic without producing separately
observable host HID frames.

OQ-016 asked whether rapid small hardware moves should be coalesced. The
current action architecture executes one `Action` synchronously. Buffering
standalone `MouseMoveRelative` calls across `ActionBackend::execute` returns
would either delay the action after a successful return or require a background
flush path whose errors could no longer be returned to the caller. That is not
acceptable for M4.

## Decision

Hardware backend coalescing applies to internally generated curve batches for
absolute `MouseMove`, one-shot `AimAt`, and drag move segments.

After curve sampling and firmware-range chunking, the hardware backend merges
adjacent `MOUSE_MOVE_REL` deltas when all conditions are true:

1. The implied span from the first pending delta to the candidate delta is
   `<= 2 ms`.
2. The merged `dx` and `dy` each remain within the firmware boot-mouse range
   `-127..=127`.
3. Each axis keeps the same sign or includes zero, so sign reversals are not
   collapsed away.

Software backend output is unchanged. `sample_curve` is unchanged.
Standalone direct `MouseMoveRelative` actions still send one command per
action in M4. Cross-call buffering is deferred until Synapse has an action
scheduler/flush surface that can return or audit delayed hardware-link errors.

## Rationale

Curve-generated hardware batches are the place where Synapse knows the full
synthetic movement path, target duration, and total displacement before
emitting commands. Coalescing there reduces wire traffic while preserving final
cursor displacement and firmware payload invariants.

The `<= 2 ms` window matches OQ-016's proposed deferred window and gives the
firmware/USB path no more than two nominal 1 ms HID polls of smoothing. Keeping
the firmware range check after every merge means coalescing cannot create a
payload the firmware would reject. Preserving sign reversals prevents a small
overshoot/correction pair from disappearing into a zero net delta.

## Alternatives Considered

- **No coalescing anywhere** - rejected because very short hardware curves can
  generate more CDC frames than the HID poll cadence can expose.
- **Coalesce standalone `MouseMoveRelative` calls across backend executions** -
  rejected for M4 because it needs a delayed flush/error-reporting surface that
  the current synchronous `ActionBackend` trait does not provide.
- **Merge all adjacent deltas regardless of direction** - rejected because it
  can erase deliberate overshoot/correction samples.
- **Use a larger window** - rejected for M4 because it would trade away more
  curve shape before real hardware telemetry proves it is needed.

## Consequences

- Positive: hardware curve batches emit fewer `MOUSE_MOVE_REL` frames when
  samples are faster than the HID poll floor.
- Positive: final displacement and firmware `-127..=127` bounds stay
  deterministic.
- Positive: software and curve-sampling math remain unchanged.
- Negative: sub-2 ms curve detail can be reduced on the hardware path.
- Negative: rapid standalone relative actions are not coalesced until a later
  action scheduler can own flush/audit semantics.

## Verification Plan

Manual FSV reads the source files and docs before and after the edit. Runtime
FSV must trigger the real Synapse action surface where hardware HID is
configured, then separately inspect the physical source of truth: host HID
telemetry/firmware command counters where available, Synapse action audit rows,
and cursor/game state readback for the final displacement.

When hardware is not configured, the configured-host readback must still inspect
the actual USB/COM/PnP state and record what physical hardware surface is
available before claiming hardware-in-loop evidence. Supporting Rust checks can
exercise emitted command vectors, but those checks are regression evidence only
and are not FSV.

## Supersedes

- OQ-016 in `docs/computergames/16_open_questions.md`.

## References

- Issue: #421
- OQ-016 in `docs/computergames/16_open_questions.md`
- Microsoft `_USB_ENDPOINT_DESCRIPTOR` reference for interrupt polling periods:
  https://learn.microsoft.com/windows-hardware/drivers/ddi/usbspec/ns-usbspec-_usb_endpoint_descriptor
- USB HID Device Class Definition, Appendix E endpoint descriptors:
  https://www.usb.org/sites/default/files/hid1_12.pdf
