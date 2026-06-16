# Plan: Queue Event Dedup — Prevent Duplicate Strand Processing

## Problem

The debounce engine coalesces rapid events within a 100ms window, but once an event leaves that window and enters the output channel, there's no further deduplication. If the agent writes to a watched strand directory during processing, subsequent `Modified` events for the same file get queued behind the in-flight event. After processing completes, ProcessStrand picks them up and re-processes the same strand multiple times — wasting agent calls and polluting loom-log with duplicate lifecycle entries.

The current pipeline:

```
notify → mpsc(100) → DebounceEngine → mpsc(100) → ProcessStrand (sync)
```

The output `mpsc` channel is opaque — the debounce engine can't inspect what's already queued, so it blindly emits every expired entry.

## Target

At most one pending event per `(strand_path, loom_id, knot_id, event_type)` in the entire pipeline. The queue becomes inspectable so the debounce engine can check before emitting. Different event types (Created/Modified/Deleted) always pass through — only repeated events of the same type are deduped.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test | What it covers | Status |
|------|---------------|--------|
| `debounce.rs` — 5 unit tests | Debounce window, different files, delete-after-modify, same-file-different-knots | ✅ Green — defines current (no-queue-dedup) behaviour |
| `pipeline.rs` — integration tests | Full notify→debounce→process flow | ✅ Green |
| `agent_integration.rs` — integration | Agent execution end-to-end | ✅ Green |
| `task_management.rs` — integration | Channel-cascade shutdown | ✅ Green |

## Test Gaps

- No test for "multiple queued events for same strand collapse to one"
- No test for "different event types both pass through"
- No test for "queue inspect-and-replace before emit"

## Phases

### Phase 0: Queue data structure — `InspectQueue` type

**File:** `src/application/debounce.rs`

Create `InspectQueue<T>` — a `VecDeque<T>` behind `Mutex` with a `Notify` for signaling. Supports:

- `push(event)` — adds to back, signals one waiter
- `push_or_replace(event, key_fn)` — scans queue for existing item with same key; replaces in-place if found, otherwise pushes
- `pop()` — removes from front, returns `Option<T>`
- `notified()` — awaits signal (for the consumer loop)

Key type is `(StrandPath, LoomId, KnotId, StrandEventKind)` where `StrandEventKind` is a new small enum: `Created`, `Modified`, `Deleted`.

- [x] Define `StrandEventKind` enum in debounce module
- [x] Implement `InspectQueue<StrandEvent>` with `push_or_replace`
- [x] Unit tests: push/pop, replace existing, no-replace different key

### Phase 1: DebounceEngine emits into `InspectQueue`

**File:** `src/application/debounce.rs`

Replace the debounce engine's output `mpsc::Sender<StrandEvent>` with `Arc<InspectQueue<StrandEvent>>`.

When a debounced event fires:
1. Compute the dedup key from the event
2. Call `queue.push_or_replace(event, key)` — replaces if same key already queued, otherwise pushes
3. Signal via `queue.notifier.notify_one()`

The `spawn_with_receiver` API changes to return `(Arc<InspectQueue<StrandEvent>>, JoinHandle<()>)` instead of `(mpsc::Receiver<StrandEvent>, JoinHandle<()>)`.

- [ ] Replace output mpsc with InspectQueue
- [ ] Wire `push_or_replace` in the expiry handler
- [ ] Wire `push_or_replace` in `flush_all` (shutdown drain)
- [ ] Update `spawn_with_receiver` signature
- [ ] Update existing debounce unit tests to compile against new API
- [ ] New test: rapid events for same key produce exactly one queued event
- [ ] New test: different event types both appear in queue

### Phase 2: ProcessStrand reads from `InspectQueue`

**File:** `src/server.rs`

ProcessStrand's event loop reads from `InspectQueue` instead of `debounce_rx.recv()`.

```rust
loop {
    let event = {
        loop {
            if let Some(e) = queue.pop() {
                break e;
            }
            queue.notifier.notified().await;
        }
    };
    use_case.execute(event)?;
}
```

Shutdown: the debounce engine pushes a sentinel (use `Option<StrandEvent>`) — `None` means "drain complete, exit". ProcessStrand breaks on `None`.

- [ ] Replace `debounce_rx.recv()` with InspectQueue pop loop
- [ ] Handle `Option<StrandEvent>` sentinel for shutdown
- [ ] Preserve burst-active / QueueIdle logic (500ms timeout after each event)
- [ ] Compile check: `cargo build` passes
- [ ] Full test suite: `cargo test` passes

### Phase 3: Integration test — duplicate events collapse

**File:** `tests/pipeline.rs`

Integration test that verifies the end-to-end dedup behaviour:

1. Start server with a knot watching a strand directory
2. Write to the strand file → triggers Created event
3. While processing, rapidly write to the file multiple times (>5 writes, spaced >100ms apart to bypass debounce window)
4. Verify only one `StrandProcessed` appears in loom-log per event type
5. Verify only one agent execution occurs

- [ ] Write integration test in `tests/pipeline.rs`
- [ ] Test passes
- [ ] Full test suite passes, clippy clean

## Notes

- The queue is small (capacity 100 in practice), so linear scan for `push_or_replace` is fine — no need for a HashMap index.
- The dedup key includes event type so Created/Modified/Deleted are never collapsed into each other — the agent always sees at least one of each type that occurs.
- This is purely an application-layer change — no new ports, no domain changes, no adapter changes.
