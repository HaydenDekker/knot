# Plan: Consumer Persistent Wake — No Lost Queue Notifications

## Related PRD

This plan contributes to the persistent-event-queue work defined in
[065 Persistent Event Queue](../065-persistent-event-queue/persistent-event-queue-plan.md)
and the front-based late-removal loop introduced by
[73 Knot Step](../73-knot-step/knot-step-plan.md). It pairs with
[075 Queue Entry Identity Self-Heal](../075-queue-identity-self-heal/queue-identity-self-heal-plan.md)
(075 fixes the observed incident; this plan closes the *remaining*
independent path to the same symptom: **queue idle while non-empty,
with no log line**).

## Problem

The consumer loop that drains the event queue waits for work with a
check-then-wait sequence (`next_event` inside
`spawn_process_strand_loop`, `src/server.rs`):

```rust
loop {
    if let Some(item) = queue.front() {      // 1. check (fresh disk scan)
        return Some(item);
    }
    if queue.shutdown_signaled() {           // 2.
        return None;
    }
    queue.notified().await;                  // 3. arm + wait
}
```

`DiskBackedEventQueue::notified()` wraps `tokio::sync::Notify`, which
is **non-persistent**: a `notify_one()` with no waiter registered is
dropped. The permit is registered only when the `notified()` future is
*polled* (step 3), i.e. **after** the `front()` check (step 1).
Consequences:

1. **Lost wake in blocking mode.** Any push (debounce engine flushing
   a watcher event, `push_shutdown`) that lands in the window between
   step 1 and the permit registration is lost. The loop then parks in
   step 3 until the *next* push or shutdown. If the lost push was the
   only pending event, the queue sits **idle with a non-empty queue** —
   the same symptom as the borrow-my-stuff incidents — with no error
   and no log. The 500 ms burst-drain timeout only covers the window
   after a processed event; the blocking (idle) wait has no timeout,
   so a lost wake there is unrecoverable without external intervention
   (a restart, or another event happening to be pushed).
2. **The same shape exists in step mode** — `step_head_event`
   (`src/server.rs`) does `front()` check, then
   `timeout(remaining, queue.notified()).await`; the same lost-wake
   window makes `knot step` report `queue empty` while a file is
   sitting in `events/` (bounded by the 500 ms deadline in the
   single-shot case, but the check-then-arm ordering is the same
   anti-pattern and the step's own settle window can feed a push into
   it).
3. **Test helpers share the shape** — `recv_with_timeout` in
   `src/application/debounce.rs` tests pops, then selects on
   `sleep` vs `queue.notified()`; it only stays correct because the
   sleep branch re-polls within 10 ms. The primitive's laziness is a
   footgun for every future caller.

The window is small (microseconds to milliseconds — one mutex check
and one future allocation between steps 1 and 3), which is why it has
not fired in the field yet; but it is the *same failure mode* as the
075 incidents (silent idle, non-empty queue, fresh state.json) and it
cannot be ruled out by code inspection. Making the wait persistent
removes the entire class rather than shrinking the window.

## Target

When this plan is complete:

1. **`StrandEventQueue::notified()` arms the permit at call time.**
   The returned future already holds a registered `Notify` permit when
   it is created (create + `enable()` before boxing). A
   `notify_one()` after the call is guaranteed to be observed by an
   await of the returned future, even if the await has not started yet.
   Signature unchanged; semantics documented on the port.

2. **The consumer loops arm before they check.** In `next_event`
   (service loop) and `step_head_event` (step mode), the notified
   future is created *first*, then `front()` is checked; if `front()`
   returns an event the armed future is dropped (harmless — the permit
   is simply not consumed). The ordering makes the lost-wake window
   structurally impossible: a push before the arm is visible to the
   fresh `front()` scan; a push after the arm is captured by the
   permit.

3. **A push can never be missed while the loop is idle** — in service
   mode (blocking wait) or step mode (deadline wait). The existing
   burst-drain behaviour (500 ms timeout → `QueueIdle`) is unchanged.

4. **The armed-wake semantics are pinned by a unit test that fails
   under the old lazy semantics** — an armed-but-not-yet-awaited
   future must be woken by a push that occurs before the await.

## Non-Goals

- Changing `tokio::sync::Notify` to a custom persistent counter/broad-
  cast — arming at call time gives persistence for the single-consumer
   pattern with no new synchronisation.
- The name/id identity bug (plan 075) — independent root cause.
- Watcher delivery latency (debounce window, 50 ms poll) — unchanged.

## Existing Tests

| Test | What it covers | Status |
|------|----------------|--------|
| `disk_event_queue.rs` `notified_unblocks_after_push` | Spawn a task that awaits `notified()`, push after 50 ms, task unblocks | ✅ Green — but the task is already *polling* when the push lands; does not distinguish lazy vs armed semantics |
| `in_memory_event_queue.rs` `notified_waits_for_push` / `notified_waits_for_shutdown` | Same shape for the test-only queue | ✅ Green — same blind spot |
| `tests/late_removal.rs` Part B (`front_loop_drains_queue_then_idles`, crash-window, poison-pill) | Service loop drains a pre-seeded queue then idles cleanly; restart re-queues in-flight events | ✅ Green — pre-seeded queues, no push-while-idle timing |
| `tests/pipeline.rs` | End-to-end watcher → debounce → queue → process with real timing | ✅ Green — exercises the wake path under load |
| `tests/step.rs` | Step-mode head resolution incl. empty-queue settle | ✅ Green |

## Test Gaps

- No test creates the `notified()` future *without polling it*, pushes,
  then awaits — the exact lost-wake ordering. Under the current lazy
  semantics the permit is unregistered at push time, the push is lost,
  and the await hangs (test times out); under armed semantics it
  wakes.
- No test pushes while the service loop is in the **blocking** (non-
  burst) wait and asserts the event is processed without any further
  push or restart.
- No documentation of the arm-before-check contract on the port, so
  future callers (and the test helpers) can reintroduce the race.

## Phases

### Phase 1: Armed-at-call `notified()` port semantics (TDD)

1. `src/application/ports.rs` — `StrandEventQueue::notified()`: extend
   the doc comment with the contract: *the returned future registers
   its `Notify` permit at creation; a signal sent after the call is
   guaranteed to wake an await of the returned future, even if the
   await has not started. Callers should create the future **before**
   re-checking `front()` and may drop it if the check already
   returned an event.*
2. `src/adapters/outbound/disk_event_queue.rs` — implement:

   ```rust
   fn notified(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
       let mut n = self.notify.notified();
       n.enable();           // permit registered now, not at first poll
       Box::pin(n)
   }
   ```

3. `src/application/in_memory_event_queue.rs` — same change (test-only
   queue, same port).
4. `push` / `push_or_replace` / `push_shutdown` keep `notify_one()` —
   with arm-before-check ordering there is always exactly one consumer
   and the armed permit makes the signal persistent; no
   `notify_waiters` needed.

Unit tests (failing first — they hang/timeout under lazy semantics —
then green), in both `disk_event_queue.rs` and
`in_memory_event_queue.rs`:

- `armed_notified_wakes_when_push_precedes_await` — call
  `queue.notified()` (future created, **not polled/awaited**);
  `tokio::time::sleep(50 ms)`; `queue.push(event)`; then
  `timeout(200 ms, fut).await` → must complete. (Old semantics: the
  push lands with no registered waiter and is dropped — the timeout
  fires, test fails.)
- `armed_notified_dropped_on_front_hit_is_harmless` — arm the future,
  `front()` returns an event (queue pre-seeded), drop the future, push
  again, arm again → the second wait wakes. Pins that dropping an
  unconsumed armed permit does not desynchronise the queue.
- `push_before_arm_is_still_visible_to_front` — push an event, *then*
  create the armed future, then `front()` → returns the event (the
  fresh-scan half of the guarantee).

### Phase 2: Arm-before-check in the consumer loops

`src/server.rs`:

1. `next_event` (inside `spawn_process_strand_loop`):

   ```rust
   async fn next_event(queue: &Arc<DiskBackedEventQueue>)
       -> Option<domain::pending_event::PendingEvent>
   {
       loop {
           let wait = queue.notified();   // permit armed before the check
           if let Some(item) = queue.front() {
               return Some(item);         // armed future dropped — harmless
           }
           if queue.shutdown_signaled() {
               return None;
           }
           wait.await;
       }
   }
   ```

   Update the surrounding comment block (which currently explains the
   burst/idle flat loop) to state the persistence guarantee.
2. `step_head_event`: same reorder — create `let wait = queue.notified();`
   at the top of the loop body, before the `front()` check; keep the
   deadline logic (`timeout(remaining, wait)`); a timed-out armed
   future is dropped and the next iteration re-arms (a push in between
   is caught by the next `front()` scan — no gap).
3. `src/application/debounce.rs` test helper `recv_with_timeout`:
   reorder to arm before the `pop()` check (the helper's 10 ms sleep
   branch already bounds any residual case; the reorder removes the
   reliance on that).

Integration tests:

- `tests/late_removal.rs` (Part B, real loop + mock `pi`):
  `push_while_idle_wakes_loop` — start the pipeline with an **empty**
  queue and let it reach the blocking idle (assert via the first
  `QueueIdle` rig-log entry *or* by waiting past one burst window);
  push one event to the queue (direct `push` through the queue Arc, or
  by writing the event file + a short debounce settle — use whichever
  the fixture exposes); assert the event is processed (loom-log
  `KnotCompleted` + queue file removed) with no restart and no second
  push. (Under the old semantics this test can flake exactly inside
  the lost-wake window; the point of the test is to exercise the
  blocking-wake path end-to-end, with the Phase 1 unit test carrying
  the deterministic proof.)
- Full `cargo test --no-fail-fast` green; `tests/pipeline.rs`,
  `tests/step.rs`, `tests/late_removal.rs` unchanged in expectation.

### Phase 3: Documentation, changelog, version

- `docs/concepts.md` — queue section: one sentence on the wake
  guarantee (a queued event always wakes the processor; the only
  empty-queue state is a genuinely empty `events/` directory).
- `.agents/skills/knot-update/SKILL.md` — changelog entry for the
  bumped version: no document migration; behaviour change is
  internal (queue wake reliability).
- Version bump (MINOR) and `docs/release-notes.md` entry via the
  project-plan-completion skill. **Coordination with plan 075:** if
  both plans complete in the same release window, apply a single bump
  (0.36.0 → 0.37.0) and record both entries in the release notes and
  the knot-update changelog; if 075 lands first, 076 bumps 0.37.0 →
  0.38.0.
- Publish updated skills globally per AGENTS.md (verify with diff).

## Notes

- **Why not a timeout on the blocking wait?** A watchdog timeout
  (e.g. re-scan every N seconds in idle mode) would also mask this
  race, but it changes the steady-state behaviour of an idle rig
  (periodic scans, extra `QueueIdle`-adjacent churn) to fix a
  microsecond window. Arming the permit makes the wait correct
  rather than merely retrying.
- **Why `enable()` at call time is safe with `notify_one`:** the loop
  has exactly one consumer; the armed permit is either (a) consumed by
  the await, (b) dropped after a `front()` hit (no signal was owed —
  the event is already in hand), or (c) timed out in step mode (next
  iteration re-arms before the next check). In no case does an armed
  permit swallow a *later* push: `Notify` permits are one-shot and the
  next wait re-arms.
- **`notify_waiters()` considered and rejected:** it would wake every
  registered waiter and is the wrong tool for a single-consumer
  queue; it also would not fix the *unregistered* window on its own —
  the arming order is the fix.
- **Relation to plan 075:** 075 makes `front()` correct for divergent
  files; this plan makes the *wait* correct for the gap between checks.
  Together they eliminate both known paths to "idle with a non-empty
  queue". They are independent and can be implemented in either
  order, though 075 addresses the field incident.
- **Determinism (corrected after Phase 1):** the plan assumed the
  Phase 1 unit test would fail under the old lazy semantics. It does
  not, on the pinned tokio 1.52.3: its `Notify::notify_one()` *stores*
  a permit when no waiter is registered (verified in the tokio source
  and by running the new tests against the old code — all pass). The
  tests therefore pin the armed-at-call contract on our port (keeping
  the guarantee independent of tokio version details) rather than
  proving the old code racy; the Phase 2 integration test still
  exercises the real loop end-to-end. See the
  [Phase 1 record](consumer-persistent-wake-phase-1.md) for the full
  verification.

## Implementation Status: 🔄 In Progress — Phases 1–2/3 complete (2026-08-25, `6c0bfb1` + `84f4f58`, see [Phase 1 record](consumer-persistent-wake-phase-1.md) and [Phase 2 record](consumer-persistent-wake-phase-2.md))
