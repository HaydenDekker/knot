# Design: Queue Identity Self-Heal and Persistent Wake

**Type:** Subsystem reference
**Subsystem:** disk-backed event queue
(`FileSystemEventStore` in `src/adapters/outbound/event_store.rs`,
`DiskBackedEventQueue` in `src/adapters/outbound/disk_event_queue.rs`,
the front-based consumer loops in `src/server.rs`)

## What It Is

Two invariants make the disk-backed queue ("the disk *is* the queue")
safe under external file operations and under its own concurrency:

1. **The filename stem is the queue entry's identity.** On every scan,
   a file whose JSON `id` differs from its filename stem is repaired in
   place — atomically rewritten with `id := stem` — and a warning is
   logged. The filename (and therefore the FIFO position) is
   authoritative.
2. **The wake is persistent by construction.** A queued event always
   wakes the processor; the only empty-queue state is a genuinely empty
   `events/` directory.

Together they close both known paths to the field symptom "rig alive,
queue non-empty, nothing happens, no log line" (the 2026-08-25
borrow-my-stuff incidents): the identity bug made `front()` return
`None` for a divergent head; the wait ordering could in principle drop
the wake for a push that landed in the check-then-wait gap.

## Identity Model (Plan 075)

### Why filename-wins

FIFO order has always come from **filename sort** (the
`{unix_ms}-{4-hex}` id prefix), while every file operation resolved
paths from the **JSON `id` field**. An operator who renames a queue
file to front the FIFO (a natural action — the queue is documented as
directly writable files) therefore creates `filename ≠ JSON id`, and
before 0.37.0:

- `front()` read by JSON id, hit a missing file, and **swallowed the
  failure with `.ok()`** → `None` → the consumer loop blocks on
  `notified()` forever (silent phantom-head wedge);
- `pop()` **panicked** on the same failure (`.expect`);
- `load_persisted` re-wrote the event under its original id (duplicate
  file, same dedup key) and late removal deleted the *restored* file,
  **orphaning** the renamed one.

The repair direction — rewrite the JSON id to the stem, not the file to
the id — was chosen deliberately:

- **Heal-by-renaming-the-file (rejected)** would *undo* the operator's
  backdate (the event jumps back to its original FIFO position — the
  exact frustration that produced the manual rename) and change FIFO
  order silently.
- **Thread paths through every operation (rejected)** is correct but
  ripples through `StrandQueueAccessor`, `ProcessStrand`, and step-mode
  resolution. Normalising at the single choke point (`scan_events`)
  restores the existing id-based invariants everywhere with **no
  signature changes**.

### Mechanics

`scan_events()` collects the directory entries first (so the repair
rewrites — which rename within the directory — cannot race the
directory iteration), then for each file:

- missing `id` → parse failure (existing malformed-skip path, warning
  extended with the name/id rule); empty `id` or a stemless `.json`
  filename → explicit malformed skip;
- `id == stem` → returned as-is;
- `id != stem` → `id := stem`, atomic temp→rename rewrite via the
  existing `write_event` path, one stderr warning:
  `[queue] repaired event file {name}: id {old} -> {stem} (filename is
  the queue identity)`.

The repair is **idempotent** (after one scan, name == id, no further
rewrites) and **convergent under races** (two scans racing on the same
divergent file both perform the identical atomic rewrite — last rename
wins, content identical). `queued_at` is preserved as the honest record
of when the event was queued; only the internal id changes.

Downstream, everything is unchanged: dedup removal, late removal,
`load_persisted`, `knot step --event` resolution, and state.json
snapshots all operate by id — which now always equals the on-disk
filename stem, so they target the real file.

### Vanished head (the legitimate `None`)

A head can still vanish between scan and read (concurrent late-removal
or `knot step`). That `None` is now **visible, not silent**:

- `front()` logs `[queue] head {name}.json vanished before read
  (concurrent removal?)` and returns `None`;
- `pop()` rescans once and retries with the new head; if the (re)scanned
  head is still unreadable or the queue drained concurrently, the same
  warning is logged and `None` returned — **no panic** (replacing the
  `.expect`).

### Operator contract

Renaming a queued event's file reorders the FIFO — it is the
**supported** way to front a queued event (e.g. a manual rectify).
Documented in the `knot-dispatch` skill (operator-facing detail) and
`docs/concepts.md` (invariant).

## Wake Persistence (Plan 076)

### The contract

`StrandEventQueue::notified()` (port) carries an **armed-at-call
contract**: the returned future registers its `Notify` permit at
creation; a signal sent after the call is guaranteed to wake an await
of the returned future, even if the await has not started. Both
implementations arm at call time:

```rust
let mut n = Box::pin(self.notify.notified());
n.as_mut().enable();   // permit registered now, not at first poll
n
```

(`Notified` is `!Unpin`, so `Box::pin` + `as_mut().enable()` — the plan
snippet `n.enable()` is not expressible as written.)

The consumer loops use **arm-before-check** ordering:

```rust
loop {
    let wait = queue.notified();   // permit armed before the check
    if let Some(item) = queue.front() {
        return Some(item);         // armed future dropped — harmless
    }
    // shutdown / deadline checks…
    wait.await;
}
```

A push before the arm is visible to the fresh `front()` scan; a push
after the arm is captured by the permit. A front hit or a timeout drops
the armed future (harmless — the event is in hand or the next iteration
re-arms before its check). The three call sites (`next_event`,
`step_head_event`, the `recv_with_timeout` test helper) share this one
shape, so a future caller copying any of them inherits the correct
ordering.

### Design decisions

- **No watchdog timeout on the blocking wait (rejected)** — a periodic
  re-scan would mask the race but change steady-state idle behaviour
  (extra scans, `QueueIdle`-adjacent churn) to fix a microsecond
  window. Arming the permit makes the wait *correct*, not merely
  retrying.
- **`notify_waiters()` (rejected)** — wakes every registered waiter;
  the wrong tool for a single-consumer queue, and it would not fix the
  *unregistered* window on its own. The arming order is the fix.
- **`notify_one()` kept** — with one consumer and an armed permit, the
  signal is persistent; no broadcast needed.

### The tokio finding

The plan assumed `tokio::sync::Notify` is non-persistent (a
`notify_one()` with no registered waiter is dropped) and that the new
unit tests would fail under the old lazy code. **On the pinned tokio
1.52.3 that premise does not hold**: `notify_one()` with no waiter
stores a permit (state `EMPTY` → `NOTIFIED`), and a `Notified`
future's first poll consumes it (verified in the tokio source and by
running the new tests against the pre-fix code — all pass). Two
consequences:

1. The lost-wake window is already closed *by tokio* on this version —
   the field incidents' root cause is the identity divergence (075),
   not the wait (076).
2. The 076 change is still correct and worthwhile: the guarantee lives
   in Knot's own code (port contract + loop ordering), pinned by tests,
   independent of a tokio implementation detail older versions did not
   have.

## Test Inventory

| Level | Tests | Pins |
|---|---|---|
| Unit (event store) | `scan_repairs_renamed_file_and_normalises_id` | repair + idempotency + field preservation |
| Unit (disk queue ×7) | `front_returns_renamed_head_without_wedging`, `pop_reads_renamed_head_without_panicking`, `front_vanished_head_is_graceful_none`, `pop_vanished_head_rescans_and_returns_tail`, `dedup_removes_renamed_file_by_healed_id`, `late_removal_deletes_the_peeked_file` | incident wedge/panic/orphan paths |
| Unit (both queues ×6) | `armed_notified_wakes_when_push_precedes_await`, `armed_notified_dropped_on_front_hit_is_harmless`, `push_before_arm_is_still_visible_to_front` | armed-at-call contract |
| Integration (`tests/queue_identity.rs`) | `backdated_head_processes_first_and_drains` (the incident repro), `restart_over_duplicate_key_files_collapses_to_one`, `step_mode_renamed_head_does_not_panic` | full composition: real queue + real loop + mock `pi` |
| Integration (`tests/late_removal.rs`) | `push_while_idle_wakes_loop` | single push through the real pipeline wakes the blocking (non-burst) wait |

The incident repro is a genuine regression proof: disabling the
`scan_events` heal block makes all three `queue_identity` tests fail
exactly as the incident mechanism predicts (two phantom-head wedge
timeouts, one step-mode "queue empty" skip).

## Known Environmental Note

`tests/thinking_level` (spawns a mock CLI inside a 10 s runner timeout)
has flaked under heavy machine load (the binary exits with zero output)
— twice across this work's full-suite runs, never in isolation, never
correlated with either plan's changes. If a verification gate fails on
that single unrelated binary, re-run the suite before concluding.

## Incident Forensics

The 2026-08-25 borrow-my-stuff timeline (14:52 backdate rename →
15:05:29 idle #1 → 15:22 restart duplicates → 15:30:42 orphan →
15:30:43 idle #2 → 15:47 manual deletion) is recorded in the
[075 plan](../plans/075-queue-identity-self-heal/queue-identity-self-heal-plan.md)
and its phase records; full monitor notes live in that project's
`_monitor_notes.md`.
