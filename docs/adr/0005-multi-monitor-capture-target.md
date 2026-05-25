# ADR-0005: Multi-Monitor Capture Target

## Context

OQ-012 asked whether Synapse should capture separate monitor targets or stitch
the virtual desktop when a host has multiple monitors. M3 needs deterministic
capture, profile matching, and manual FSV readback without hiding behavior
behind implicit multi-monitor fan-out.

## Decision

Synapse has one active capture target per session. The default target is the
primary monitor. Agents switch targets explicitly with
`set_capture_target({target:{kind:"monitor", monitor_index:N}})` or a window
target. Synapse does not stitch the virtual desktop for perception, and it does
not run separate concurrent monitor captures in M3.

When monitors are added, removed, reordered, or DPI settings change mid-session,
the existing active target remains in effect while it is still resolvable. If it
is no longer resolvable, capture degrades closed and the agent must call
`set_capture_target` again. Profile resolution uses the foreground app/window
state and the current active target; monitor changes do not implicitly change
the active profile.

Audio loopback is independent of the visual capture target. It follows the
configured/default render endpoint rather than a monitor index.

## Rationale

One explicit active target keeps capture resource use bounded and makes source
of truth verification straightforward: the active target is the daemon's current
capture state and `set_capture_target` response. Stitched virtual-desktop frames
make windows spanning monitors appear coherent, but they also introduce large
surfaces, DPI seams, and ambiguous coordinate origins. Concurrent per-monitor
captures would increase GPU/CPU cost before M3 has evidence that agents need
that capability.

## Alternatives Considered

- Stitched virtual desktop - rejected because it increases frame size and mixes
  per-monitor DPI/coordinate behavior into one perception frame.
- Concurrent capture per monitor - rejected for M3 because it raises resource
  cost and makes manual readback less direct.
- Profile-driven automatic monitor switching - rejected because it can change
  the agent's visual context without an explicit tool call.

## Consequences

- Positive: capture state has a single source of truth per session.
- Positive: manual FSV can verify target switches by reading the
  `set_capture_target` response and subsequent observation dimensions/target
  metadata.
- Negative: an agent that needs another monitor must switch targets explicitly.
- Trade-off accepted: windows spanning monitors are handled by selecting a
  window target when needed instead of stitching the whole desktop.

## Supersedes

- OQ-012 in `docs/computergames/16_open_questions.md`

## References

- Decision issue: #337
- Tool surface: `docs/computergames/05_mcp_tool_surface.md` §3.9
- Perception subsystem: `docs/computergames/02_perception.md` §2
