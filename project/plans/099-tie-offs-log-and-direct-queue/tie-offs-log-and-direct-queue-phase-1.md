# Phase 1: Payload-embedded queue entries

**Plan:** [Unified Tie-Off Log and Direct Event Queueing](tie-offs-log-and-direct-queue-plan.md)

**Runs only after Phase 0 is green and committed.** (Phase 0 made no
behaviour change — its pane is the recorded baseline; this phase starts
from that baseline.)

## Checklist
- [x] `PendingEvent` gains `delivery: Option<EventDelivery>` — serde
      default `None`, skipped on serialisation when absent (pre-099
      entries on disk load unchanged) — and a new `kind: "Event"`
      (`EVENT_ENTRY_KIND`); `EventDelivery` carries `event_id`,
      `producer_knot`, `producer_loom`, `original_strand`, `body`, and
      the `BTreeMap` payload (deterministic JSON)
- [x] The event entry's `strand_path` is the synthetic descriptor
      `event:<producer-knot>/<event-id>` — greppable, never stat-ed
      (file validation skipped for the `Event` kind)
- [x] `StrandEvent` gains `EventDelivered`; the consumer pipeline
      branches on it: no `@file` ref to the adapter, the rendered
      delivery prepended to the prompt, batch stamps a fresh hop
- [x] `write_tie_off` takes the trace fields from the delivery, not a
      dispatched file
- [x] `tests/payload_queue.rs` (21 tests) — per-subscriber fan-out with
      the payload in the entry body; no dispatched `.md` written;
      removal at completion and at failure; `load_persisted` redelivery
      after a simulated crash; the consumer prompt carries the payload
      with no `@file`; legacy JSON loads; payload-less `Event` kind
      rejected; render + determinism
- [x] Verification — clippy clean for the new code; `tests/payload_queue`
      green (21 passed) and the full suite green; commit sha `0e0c501`
      recorded as this phase's external-verdict request. The ON-COMMIT
      suite is not required by this item.

## Deviations
- None — the phase executed the Phase 0 decisions D1–D3 as written.

## Discoveries
- The direct-queue-write path for manual dispatch of the `Event` kind
  is not exercised by a test yet — it lands with Phase 5 (manual
  dispatch retires onto it, D8).

## Notes
- Committed on `main` (the project's main-only pattern — the plan 097
  precedent; no feature branch for this plan).
- The dispatcher's view of the producer (token only, loom unknown)
  surfaced while wiring Phase 2 — recorded there.
