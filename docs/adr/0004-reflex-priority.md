# ADR-0004: Reflex Priority And Starvation Semantics

## Context

OQ-005 asked whether contending reflexes should use strict priority or a
probabilistic mix. M3 needs deterministic behavior for reflexes that target the
same cursor, key, mouse button, or gamepad resource.

## Decision

Reflex priority is a `u32`; lower numbers have higher priority. The default
priority is `100`. When two active reflexes with the same priority contend for
the same resource in the same scheduler tick, the newer registration wins.
Losing reflexes accumulate contended tick time and become `Starved` after 2
seconds. Each starvation interval emits one `reflex_starved` event and writes
one `REFLEX_STARVED` audit row.

## Rationale

Strict priority is predictable and easy to debug from status and audit rows.
Lower-number-wins leaves room for urgent operator or safety reflexes at `0..99`
while ordinary reflex registrations can use the default `100`. Newer-wins tie
breaks make replacement registrations take effect without relying on random
iteration order.

## Alternatives Considered

- Higher-number-wins priority - rejected because lower numbers map more
  naturally to urgent queues and leave `0` as the strongest override.
- Probabilistic arbitration - rejected because it makes cursor and input
  behavior harder to explain during manual FSV and gameplay debugging.
- Older-wins tie breaks - rejected because replacing a reflex should not leave
  the older registration silently owning the device resource.

## Consequences

- Positive: contended input dispatch is deterministic and explainable from
  status, events, and audit rows.
- Negative: lower-priority reflexes can starve under constant contention.
- Trade-off accepted: starvation is observable after 2 seconds via
  `REFLEX_STARVED`, giving agents a concrete signal to cancel or reprioritize
  reflexes.

## Supersedes

- OQ-005 in `docs/computergames/16_open_questions.md`

## References

- Issue: #292
- Decision issue: #336
- Spec: `docs/computergames/04_reflex_runtime.md` §6
