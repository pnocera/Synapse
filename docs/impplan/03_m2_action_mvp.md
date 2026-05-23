# 03 — M2: Action MVP (2 weeks)

PRD: `15_roadmap_and_milestones.md` §4. Subsystem detail: `03_action.md`. Schemas: `06_data_schemas.md` §4.

## Goal

`synapse-action` emits keyboard, mouse, gamepad through one serialization actor. Software backend + ViGEm. Aim curves + keystroke dynamics. `ReleaseAll` safety net live.

## Demo gate

Notepad open → agent: `act_click(element_id=<editor>)` → `act_type(text="Hello world.\nThis is Synapse.")` → `act_press(keys=["ctrl","s"])` → `observe()` returns the "Save As" dialog. ≤ 8 tool calls end-to-end.

---

## Inputs

- M1 demo gate passed
- ViGEmBus driver installed (`winget install Nefarius.ViGEmBus`) on dev box + CI runner
- `enigo = "0.6"` + `vigem-client = "0.1"` resolvable

---

## Deliverables

### Crates

| Crate | M2 contents |
|---|---|
| `synapse-action` | `ActionEmitter` mpsc actor; `SoftwareBackend` via `enigo` + direct `windows-rs` for batched `SendInput`; `VigemBackend` via `vigem-client`; held-key/button BitSet tracking; per-action timeout 30 s; aim curves `Instant`/`Linear`/`EaseInOut`/`Bezier`/`Natural`; keystroke dynamics `Burst`/`Linear`/`Natural`; UIA `InvokePattern` semantic click |
| `synapse-core` (extensions) | `Action` enum (all variants from `06 §4`); `AimCurve`, `AimNaturalParams`, `AimStyle`, `KeystrokeDynamics`, `MouseButton`, `ButtonAction`, `Key`, `KeyCode`, `PadId`, `PadButton`, `Stick`, `Trigger`, `GamepadReport`, `ComboStep`, `ComboInput`, `MouseTarget`, `AimTarget` |
| `synapse-mcp` (add tools) | `act_click`, `act_type`, `act_press`, `act_aim`, `act_drag`, `act_scroll`, `act_pad`, `act_clipboard`, `release_all` per `05 §3.11-3.19, §3.26` |

### Channel + lifetime invariants

- Action mpsc bounded cap 256; saturation ⇒ `ACTION_QUEUE_FULL`
- Per-backend rate cap (`03 §15`): software 5000 ev/s, ViGEm 1000 reports/s
- `held_key_max_duration_ms = 30000`; auto `KeyUp` + `STUCK_KEY_AUTO_RELEASED` event
- Panic hook fires `ReleaseAll` via static `OnceCell<ActionHandle>`; runs in ≤ 10 ms

### Error codes (must throw + test)

```
ACTION_QUEUE_FULL
ACTION_RATE_LIMITED
ACTION_BACKEND_UNAVAILABLE
ACTION_TARGET_INVALID
ACTION_HOLD_EXCEEDED_MAX
ACTION_VIGEM_NOT_INSTALLED
ACTION_VIGEM_PLUGIN_FAILED
ACTION_ELEMENT_NOT_RESOLVED
ACTION_FOREGROUND_LOST
ACTION_UNSUPPORTED_KEY
ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT
STUCK_KEY_AUTO_RELEASED
SAFETY_RELEASE_ALL_FIRED
```

---

## Work-items (PR-sized, ordered)

| # | Title | Acceptance |
|---|---|---|
| 1 | `feat(core): Action enum + all sub-types from 06 §4` | round-trip serde JSON + bincode; insta snapshot of every variant |
| 2 | `feat(action): ActionEmitter mpsc actor + held-state tracking` | proptest: random Action stream + final ReleaseAll ⇒ empty `held_keys`/`held_buttons` (13 §5) |
| 3 | `feat(action): SoftwareBackend via enigo + windows-rs SendInput batches` | bench `action_software_press` ≤ 1 ms p99 (13 §7 / 10 §2) |
| 4 | `feat(action): aim curves Instant / Linear / EaseInOut / Bezier / Natural` | proptest: curve step sequence start[0]=start, end[N-1]=end, total_ms within tolerance; bench `aim_curve_step_calc_natural` ≤ 1 µs/step |
| 5 | `feat(action): keystroke dynamics Burst / Linear / Natural` | proptest: chars round-trip via RecordingBackend; modifier-state consistent |
| 6 | `feat(action): UIA InvokePattern path when element_id+Invoke supported` | semantic click on Notepad menu item ≤ 25 ms p99 (10 §2) |
| 7 | `feat(action): VigemBackend (X360 + DS4) via vigem-client` | pad plug-in lazy on first call; `wait_for_ready`; gamepad report applied; bench send 1000 reports/s without drop |
| 8 | `feat(action): rate limiter per backend` | overshooting cap surfaces `ACTION_RATE_LIMITED` + re-queue with backoff |
| 9 | `feat(action): held-key auto-release timeout + STUCK_KEY_AUTO_RELEASED event` | KeyDown without paired KeyUp within 30s emits auto-release + event |
| 10 | `feat(action): ReleaseAll on shutdown + SIGINT + panic hook` | integration: hold keys, kill daemon, assert all keys released via RecordingBackend or external HID monitor |
| 11 | `feat(action): MouseDrag, MouseScroll, double/triple-click via GetDoubleClickTime` | drag = down + curve + up; scroll uses `MOUSEEVENTF_WHEEL`/`HWHEEL`; double/triple uses OS double-click time |
| 12 | `feat(mcp): act_click, act_type, act_press, act_aim, act_drag, act_scroll, act_pad, act_clipboard, release_all` | schemas match `05 §3.11-3.19, §3.26`; `additionalProperties: false` |
| 13 | `test(e2e): notepad_type_save` | `13 §8` scenario: open Notepad, type, save, verify file content |
| 14 | `bench: action latencies per backend per kind` | `criterion` set committed; weekly regression CI active |

---

## Acceptance gates (block M3)

```
✓ Notepad type-save demo passes via stdio MCP
✓ act_press p99 ≤ 3 ms software (10 §12)
✓ act_click(element_id) semantic invoke p99 ≤ 25 ms (10 §12)
✓ act_click(x,y) EaseInOut 80ms p99 ≤ 60 ms (10 §12)
✓ ReleaseAll fires within 10 ms on Ctrl+C / SIGINT / panic
✓ proptest no_stuck_keys passes 1000+ cases
✓ No mocks gate completion — real SendInput on real Notepad in E2E
✓ ViGEm pad updates round-trip via vigem-client to a controller-aware test program
✓ All 9 M2 tools schema-snapshotted via insta
```

---

## Risks (`15 §9` + extras)

| Risk | Mitigation |
|---|---|
| ViGEm install friction | Setup wizard offers `winget install Nefarius.ViGEmBus`; runtime detects, surfaces `ACTION_VIGEM_NOT_INSTALLED`; ViGEm-backed features skip silently w/ warn log if absent |
| `Natural` curve feel iteration | Default `EaseInOut` for productivity profiles; `Natural` parameters tuned at M5 with real gameplay data |
| `enigo` limitations on raw scan codes | Profile flag `keyboard.use_scancodes` for games that read raw input; direct `windows-rs` `SendInput` w/ `KEYEVENTF_SCANCODE` |
| Unicode typing into games that ignore `KEYEVENTF_UNICODE` | per-game profile flag falls back to per-char scancode |
| `held_key_max_duration_ms` collisions with reflex `hold_move` (M3) | M3 raises cap or registers reflex-owned holds; M2 alone enforces 30 s |
| `Stick` analog smoothing for sim profiles | optional `AnalogCurve::Smooth { tau_ms }` added in M3/M4 if profile demands it |

---

## Out of scope at M2 (deferred ≥ M3)

- Reflexes / event-driven actions
- `act_combo` (frame-accurate sequencer; needs reflex runtime — M3)
- `act_run_shell`, `act_launch` (gated via permission model — M3/M4)
- Hardware HID backend (M4)
- RocksDB action log persistence (M3)

---

## Definition of Done

M2 closed when demo passes + acceptance gates green + `git tag v0.1.0-m2`. Open next: `04_m3_reflex_mcp_surface.md`.
