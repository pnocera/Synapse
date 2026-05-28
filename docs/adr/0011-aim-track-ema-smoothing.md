# ADR-0011: Aim Track EMA Smoothing Default

## Status

Accepted 2026-05-28.

## Context

The `aim_track` reflex follows a moving target at the reflex scheduler rate.
Detection boxes can jitter frame to frame even when the target is visually
stable. A controller that follows every raw delta hunts around the crosshair;
a controller that smooths too heavily lags behind moving entities.

Synapse already exposes `ema_alpha` as an explicit `reflex_register` override.
The unresolved decision was the default for callers that omit the field.

NIST describes single exponential smoothing as weighting the current
observation with a smoothing constant and the prior smoothed value with the
remaining weight. For Synapse target deltas, the update is:

```text
smoothed_delta_t = alpha * capped_delta_t + (1 - alpha) * smoothed_delta_t-1
```

## Decision

Use `alpha = 0.7` as the default `aim_track` EMA smoothing constant.

The canonical code constant is
`synapse_core::DEFAULT_AIM_TRACK_EMA_ALPHA`. `synapse-reflex` re-exports this
as `DEFAULT_EMA_ALPHA` for local callers, but the source of authority is
`synapse-core`.

`aim_track` applies gain and max-speed clamping before the EMA. The first
tick uses the capped delta directly because no previous smoothed delta exists.
Callers can still set `ema_alpha` explicitly:

- `ema_alpha = 1.0`: no smoothing after clamping.
- `ema_alpha = 0.0`: hold the previous smoothed delta after the first tick.
- `0.0 < ema_alpha < 1.0`: blend current capped delta with the prior smoothed
  delta.

## Rationale

`0.7` gives most weight to the current detection while retaining enough prior
state to damp one-frame noise. With the OQ-013 jitter example
`820 -> 824 -> 818 -> 822`, the smoothed positions become
`820 -> 822.8 -> 819.44 -> 821.232`. That preserves direction changes without
fully mirroring each frame of detector jitter.

Lower values such as `0.3` reduce jitter more aggressively but lag too much
when a target moves across the screen. Higher values such as `0.9` are more
reactive but are closer to the raw detector output and therefore hunt more
under noisy boxes.

## Alternatives Considered

- **`alpha = 1.0`** - rejected as the default because it disables smoothing
  and follows detector jitter directly.
- **`alpha = 0.5`** - rejected as the default because it adds more lag than
  needed for the first game-profile pass.
- **Profile-specific default only** - rejected because the MCP tool needs a
  documented default even when a caller registers `aim_track` outside a
  bundled game profile.

## Consequences

- Positive: `aim_track` has one documented default shared by code, docs, and
  MCP runtime behavior.
- Positive: profile authors can reason about omitted `ema_alpha` fields by
  reading a core constant instead of a reflex-local magic number.
- Positive: registration with explicit `ema_alpha` remains available for
  profile/game-specific tuning.
- Negative: the value is a first M4 default, not a proven optimum for every
  detector, framerate, or game.
- Negative: real Minecraft tuning still depends on the operator-gated
  Minecraft runtime/demo work tracked outside this ADR.

## Verification Plan

Manual FSV reads the source files and docs before and after the edit, then
triggers `reflex_register` through the real MCP daemon with omitted and
boundary `ema_alpha` values. The physical source of truth is the on-disk source
file bytes plus the local RocksDB `CF_REFLEX_AUDIT` rows read back through MCP
storage/reflex history tools after registration.

Supporting Rust checks can compile and exercise the controller math, but those
checks are regression evidence only and are not FSV.

## Supersedes

- OQ-013 in `docs/computergames/16_open_questions.md`.
- Any M3/M4 impplan text that treated the `0.7` value as an unowned
  reflex-local magic number.

## References

- Issue: #420
- OQ-013 in `docs/computergames/16_open_questions.md`
- NIST/SEMATECH e-Handbook of Statistical Methods, single exponential
  smoothing: https://www.itl.nist.gov/div898/handbook/pmc/section4/pmc431.htm
