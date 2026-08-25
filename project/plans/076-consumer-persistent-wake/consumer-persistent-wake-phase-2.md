# Phase 2: Arm-before-check in the consumer loops

**Plan:** [Consumer Persistent Wake — No Lost Queue Notifications](consumer-persistent-wake-plan.md)
**Commit:** `84f4f58` (branch `refactor/consumer-persistent-wake-plan`)
**Date:** 2026-08-25

## Checklist
- [x] `src/server.rs` `next_event` (service loop) — `let wait = queue.notified();` created at the top of the loop body, **before** the `front()` check; a front hit returns with the armed future dropped (harmless); the tail is now `wait.await`
- [x] `src/server.rs` `next_event` comment block — "Wake persistence" paragraph added to the burst/idle flat-loop comment stating the guarantee: a push before the arm is visible to the fresh scan, a push after the arm is held by the permit; the only idle state is a genuinely empty queue. Doc comment on `next_event` itself added
- [x] `src/server.rs` `step_head_event` (step mode) — same reorder: `let wait = queue.notified();` at the top of the loop body before `front()`; deadline logic (`timeout(remaining, wait)`) kept intact; a front hit or a timeout drops the armed future and the next iteration re-arms before its check
- [x] `src/application/debounce.rs` test helper `recv_with_timeout` — reordered to arm before the `pop()` check; the `select!` branch now awaits the pre-armed `wait`; the 10 ms sleep branch remains as a bound on residual cases; doc comment updated
- [x] `tests/late_removal.rs` (Part B, real loop + mock `pi`) — new `push_while_idle_wakes_loop`: empty queue at startup, wait past one burst window (1 s) with the loop pinned in the blocking wait (0 event files, 0 `KnotCompleted`, no `QueueIdle`), one push via the real pipeline (strand file → watcher → debounce → queue), assert processed exactly once (`KnotCompleted` ×1) and event file removed by late removal — no second push, no restart
- [x] `cargo build` clean (only pre-existing warnings); `cargo test --no-fail-fast` fully green — 868 lib tests, all integration binaries; `late_removal` 21 passed (20 → 21, new test included), `pipeline` 21, `step` 19, `persistent_queue` 8 — expectations unchanged

## Deviations
1. **Integration test push path:** the plan allowed "direct `push` through the queue Arc, or by writing the event file + a short debounce settle — use whichever the fixture exposes." The fixture exposes the file-watcher path, so the test writes the strand file and lets the real watcher → debounce → queue pipeline deliver the push. This is the stronger of the two sanctioned options: it exercises the blocking-wake end-to-end including the debounce engine's flush, not just a queue-level push.

## Discoveries
- The deterministic pin of the **blocking** (non-burst) wait in the new test works via the *absence* of `QueueIdle` before any processed event: a burst window — and the `QueueIdle` that ends it — requires a processed event, so an empty-queue loop that has written no `QueueIdle` after 1 s can only be in the blocking wait. No queue-internal state inspection needed.
- No further tokio surprises: the reorder is a structural property of our code's ordering and is correct regardless of the pinned tokio's stored-permit behaviour (see the Phase 1 record).

## Notes
- All three call sites now share one shape — arm, check, await the armed future — so a future caller copying any of them inherits the correct ordering.
- Phase 3 (documentation, changelog, version) is the remaining phase.
