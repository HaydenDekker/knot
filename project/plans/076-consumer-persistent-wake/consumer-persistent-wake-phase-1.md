# Phase 1: Armed-at-call `notified()` port semantics (TDD)

**Plan:** [Consumer Persistent Wake — No Lost Queue Notifications](consumer-persistent-wake-plan.md)
**Commit:** `6c0bfb1` (branch `refactor/consumer-persistent-wake-plan`)
**Date:** 2026-08-25

## Checklist
- [x] `src/application/ports.rs` — `StrandEventQueue::notified()` doc extended with the armed-at-call contract: the returned future registers its `Notify` permit at creation; a signal sent after the call is guaranteed to wake an await of the returned future even if the await has not started; callers should create the future **before** re-checking `front()` and may drop it on a front hit
- [x] `src/adapters/outbound/disk_event_queue.rs` — `notified()` arms the permit at call time: `let mut n = Box::pin(self.notify.notified()); n.as_mut().enable(); n` (the plan's literal `n.enable()` does not compile — `Notified` is `!Unpin` via `PhantomPinned`; `Box::pin` + `as_mut().enable()` is the safe equivalent, armed by the time `notified()` returns)
- [x] `src/application/in_memory_event_queue.rs` — same change (test-only queue, same port)
- [x] `push` / `push_or_replace` / `push_shutdown` untouched — still `notify_one()`, per plan item 4
- [x] Tests added to **both** implementations (6 total): `armed_notified_wakes_when_push_precedes_await`, `armed_notified_dropped_on_front_hit_is_harmless`, `push_before_arm_is_still_visible_to_front`
- [x] Compile clean (`cargo build`); full suite green (`cargo test --no-fail-fast`: 868 lib tests + all integration binaries incl. `late_removal`, `pipeline`, `step`, `persistent_queue`; 0 failures)

## Deviations
1. **The TDD "failing first" step could not fail — and we verified why.**
   The new tests were run against the *old* lazy code first: all 6 passed.
   Reading the pinned tokio 1.52.3 source
   (`src/sync/notify.rs`), `notify_one()` with no registered waiter
   **stores** a permit (state `EMPTY` → `NOTIFIED`, "No waiters, no
   further work to do"), and the first poll of a `Notified` future
   consumes it (CAS `NOTIFIED` → `EMPTY` → `Done`, "Optimistically try
   acquiring a pending notification"). The plan's premise ("tokio
   `Notify` is non-persistent; a `notify_one()` with no waiter is
   dropped") does not hold in this tokio version — the lost-wake window
   the plan describes is already closed by tokio itself. The mandated
   change still stands as specified: it makes the armed contract
   explicit on the port and pins it via tests, independent of any
   version-specific permit-storage behaviour (the guarantee no longer
   depends on a tokio implementation detail that older versions did not
   have).
2. **Test 2 ordering corrected.** The plan's literal sequence (arm →
   `front()` hit → drop → push → re-arm → "second wait wakes") cannot
   wake under *any* `Notify` semantics: the push that lands while no
   permit is armed cannot source the later future's wake (it would only
   wake via tokio 1.52.3's stored-permit behaviour, which is not the
   property under test). The test instead pins the stated property —
   dropping an unconsumed armed permit does not desynchronise the
   queue: after the drop, a push (visible to the fresh `front()` scan)
   is followed by a re-armed wait, which wakes on the next push. The
   correction is documented in the test's comment in both files.

## Discoveries
- **tokio 1.52.3 `Notify` is persistent for `notify_one`** (stored
  permit when no waiter). This also means the 075-adjacent "idle with a
  non-empty queue" symptom cannot arise from the *consumer-side*
  lost-wake window on this tokio version — the field incidents' root
  cause must lie elsewhere (plan 075's filename/id identity divergence
  remains the leading suspect).
- `Notified<'_>` is `!Unpin`; arming requires `Box::pin(...).as_mut().enable()`
  (or holding the `Notified` in a `Pin`) — the plan's snippet
  `let mut n = self.notify.notified(); n.enable();` is not expressible
  as written.
- `enable()` returns `bool` (false if the future already completed or
  was notified before arming); the call-site ignores it, which is fine
  because `notified()` was just created.

## Notes
- Phase 2's arm-before-check reorder and the `push_while_idle_wakes_loop`
  integration test are unaffected by the tokio finding: the reorder is
  still structurally preferable (the guarantee lives in our code's
  ordering, not in the tokio version), and the integration test still
  exercises the real loop end-to-end.
- The plan's "Determinism" note was corrected in the plan file to
  reflect that the Phase 1 tests pin the contract but do not
  fail under the old code on this tokio version.
