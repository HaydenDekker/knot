# Phase 3: Tests

**Plan:** [Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget](087-event-source-continuation-and-queue-budget-plan.md)

## Checklist

Mirror the existing 086 test seams in `process_strand_helpers.rs`, `process_strand.rs`, `disk_event_queue.rs`, and `mod_watchers.rs`. Several are the red-first tests introduced in Phases 0–2; this phase confirms the full table is present and green.

- [ ] **Filesystem continuation still lands in the strand dir** — D1 preservation
- [ ] **Event-source continuation lands in `tie-offs/<rig>/<loom>/<event-id>/`** and re-triggers the same knot — D1 (the v1 gap)
- [ ] **Queue-wait exemption**: a continuation dequeued after a long gap keeps its full `budget-secs` (not `budget - gap`) — D2 (the core fix)
- [ ] **Budget decays by `execution_secs` across hops; `Σ exec ≤ profile_timeout`** — D2 invariant
- [ ] **Iteration cap**: a hop that would exceed `MAX_CONTINUATIONS` → `BatchIncomplete (reason: caps)`, no continuation file — Bound 1
- [ ] **Budget exhaustion**: `budget-secs < MIN_REMAINING_SECS` at dequeue → degenerate `Failed` tie-off + `BatchIncomplete (reason: deadline)` — Bound 2
- [ ] **Continuation file captured in the project's per-turn commit** (replay-by-commit source; rig repo stays source-only) — D3
- [ ] **Suppression ordering**: on cap/budget-exhaustion, no spurious `TasksIncomplete`/handoff log — observability fix
- [ ] **Compatibility shim**: a pre-087 file with only `batch-deadline-epoch` still resolves a budget — migration
- [ ] **FIFO**: a continuation written to the inbox is queued **after** any already-queued events (append, not front) — D3 ordering
- [ ] `cargo test --no-fail-fast` fully green (record the pass/fail counts); confirm no new failures beyond any pre-existing baseline

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- `execution_secs` is wall-clock (dequeue → handoff) — total processing time including Knot overhead; the test should fix a known dequeue→handoff span and assert the stamped `budget-secs` decays by exactly that span.
- The queue-wait-exemption test is the discriminating one: it must show the continuation's runner timeout equals the stored `budget-secs` even when dequeue time is far past the handoff time (queue wait is never folded in).
