# Plan: Strand Queue Visibility in State

## Problem

`rig/state.json` shows looms, knots, and their processing status — but not the **pending strand event queue**. When strands are waiting in the debounce/processing pipeline (e.g. during bursts or while a knot is busy), there's no way to see what's queued and waiting to be processed from `state.json`.

The `InspectQueue<Option<StrandEvent>>` exists in the event pipeline and holds debounced events ready for `ProcessStrand`. But:

1. `StrandEvent` carries `loom_id`, `knot_id`, `strand_path` — but **no timestamp**
2. The queue is created inside `start_event_pipeline()` and not shared with `WriteState`
3. `RigState` has no field for the strand queue

## Target

`rig/state.json` includes a `strand_queue` array showing all pending strand events, each with:

```json
{
  "strand_queue": [
    {
      "strand_path": "/home/user/project/src/main.rs",
      "loom_id": "review-loom",
      "knot_id": "review",
      "event_type": "created",
      "queued_at": "2026-06-30T12:00:00Z"
    }
  ]
}
```

Each entry shows: the file path, the loom and knot it targets, the event type (created/modified/deleted), and the timestamp when the event entered the pipeline.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test | What it covers | Status |
|------|---------------|--------|
| `debounce.rs` — InspectQueue tests | push/pop, push_or_replace dedup, notified signalling, shutdown sentinel | ✅ Green — 12 tests |
| `debounce.rs` — DebounceEngine tests | Single event, rapid events, different files, delete-after-modify, same-file-different-knots | ✅ Green — 7 tests |
| `write_state.rs` — WriteState tests | Empty rig, with looms, derive_knot_state from log, execute builds+writes | ✅ Green — 9 tests |
| `entities.rs` — RigState tests | Construction, serialization roundtrip, omits null fields, multiple looms | ✅ Green — 6 tests |
| `server.rs` — composition tests | AppContext wiring, startup config creation | ✅ Green — 4 tests |
| `tests/pipeline.rs` — integration | Full notify→debounce→process flow, queue idle detection | ✅ Green |

## Test Gaps

- No test for `RigState.strand_queue` field (doesn't exist yet)
- No test for timestamp capture on strand events
- No test verifying state.json reflects queue contents during processing
- No integration test for "queue visible in state while events are pending"

## Phases

### Phase 0: `StrandQueueEntry` domain type + timestamp on queue events

**Files:** `src/domain/entities.rs`, `src/application/debounce.rs`

Create a serialisable snapshot type for the state file and wrap queued events with timestamps.

- [ ] Add `RigStateStrandQueueEntry` struct to `entities.rs`:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub struct RigStateStrandQueueEntry {
      pub strand_path: String,
      pub loom_id: String,
      pub knot_id: String,
      #[serde(rename = "event_type")]
      pub event_kind: String,      // "created", "modified", "deleted"
      pub queued_at: String,        // ISO 8601 UTC
  }
  ```
- [ ] Add `strand_queue: Vec<RigStateStrandQueueEntry>` to `RigState` (serialises as empty array when empty — no `skip_serializing_if` needed since `Vec` serialises to `[]`)
- [ ] Create `TimestampedStrandEvent` wrapper in `debounce.rs`:
  ```rust
  #[derive(Debug, Clone)]
  struct TimestampedStrandEvent {
      event: StrandEvent,
      queued_at: String,
  }
  ```
- [ ] Change `InspectQueue` in the debounce pipeline from `InspectQueue<Option<StrandEvent>>` to `InspectQueue<Option<TimestampedStrandEvent>>`
- [ ] Update debounce engine's `push_or_replace` / `push` calls to wrap events in `TimestampedStrandEvent` with `format_timestamp()`
- [ ] Update `dedup_opt_key` to work with `Option<TimestampedStrandEvent>` (key extracted from inner `.event`)
- [ ] Update `QueueReceiver::recv()` return type to `Option<TimestampedStrandEvent>`
- [ ] Update ProcessStrand consumer in `server.rs` to unwrap `.event` from `TimestampedStrandEvent`
- [ ] Unit tests: `RigStateStrandQueueEntry` construction + serialization roundtrip
- [ ] Unit tests: `RigState` with `strand_queue` field serialises correctly (including empty queue)
- [ ] Update existing debounce tests to compile against `TimestampedStrandEvent`

### Phase 1: Expose queue snapshot through `InspectQueue`

**File:** `src/application/debounce.rs`

Add a method to snapshot the current queue contents for the state writer.

- [ ] Add `snapshot(&self) -> Vec<TimestampedStrandEvent>` method to `InspectQueue` — locks the mutex and clones the `VecDeque` into a `Vec` (only the `Some(event)` entries, excluding the `None` shutdown sentinel)
- [ ] Unit test: snapshot returns current items in FIFO order
- [ ] Unit test: snapshot excludes `None` sentinel
- [ ] Unit test: snapshot is empty when queue is empty

### Phase 2: Wire queue `Arc` into `AppContext` and `WriteState`

**Files:** `src/server.rs`, `src/application/usecases/write_state.rs`

Share the queue reference with the state writer.

- [ ] Add `strand_queue: Option<Arc<InspectQueue<Option<TimestampedStrandEvent>>>>` field to `AppContext` in `server.rs`
- [ ] In `start_event_pipeline()`, store the `Arc<InspectQueue>` into `AppContext.strand_queue` after creating it (via `ctx.strand_queue = Some(Arc::clone(&debounce_rx))` — but `debounce_rx` is local, so instead return it from the function or share via a temporary)
- [ ] Actually: `spawn_with_receiver` returns the `Arc<InspectQueue>`. Change `start_event_pipeline` to accept `ctx` by mutable reference and set `ctx.strand_queue`, or return the Arc alongside. Simpler approach: add the `Arc` to `AppContext` before spawning, then clone it into the pipeline.
- [ ] Add `strand_queue: Option<Arc<InspectQueue<Option<TimestampedStrandEvent>>>>` parameter to `WriteState::new()` and `WriteState` struct
- [ ] In `WriteState::build_state()`, call `queue.snapshot()` if queue is present, map each `TimestampedStrandEvent` to `RigStateStrandQueueEntry`, and attach to `RigState`
- [ ] In `start_state_writer()` in `server.rs`, clone the queue `Arc` and pass to `WriteState::new()`
- [ ] In `build_app_context()`, initialise `strand_queue` as `None`
- [ ] Unit tests in `write_state.rs`:
  - `build_state_empty_queue` — queue present but empty, `strand_queue` is `[]`
  - `build_state_with_queued_events` — queue has events, `strand_queue` populated correctly
  - `build_state_no_queue` — queue is `None` (backward compat), `strand_queue` is `[]`
- [ ] Update existing `WriteState` tests to pass the new parameter

### Phase 3: Integration verification

**File:** `tests/pipeline.rs` (or new integration test file)

End-to-end verification that the strand queue appears in `state.json`.

- [ ] Integration test: create a strand file, verify it appears in `state.json.strand_queue` before processing completes (use tight debounce timing via env vars)
- [ ] Integration test: after processing completes, verify `strand_queue` is empty in `state.json`
- [ ] Integration test: multiple strands queued — all appear in `strand_queue` with correct paths, loom/knot IDs, and timestamps
- [ ] Run full test suite, verify no regressions

## Notes

The `InspectQueue` is already thread-safe (`Mutex<VecDeque<T>>`), so sharing via `Arc` is straightforward. The `snapshot()` method holds the lock briefly (just cloning the vec), which is acceptable given the 5-second state write interval — no contention with the 5ms debounce check cycle.

`TimestampedStrandEvent` wraps the event inside the queue only — the debounce engine's internal `pending` HashMap still uses raw `StrandEvent` (the timestamp is captured when the debounced event is emitted, i.e. when it enters the output queue). This is the right semantics: `queued_at` = "ready for processing", not "raw event received".

The dedup key for `InspectQueue` must work on `Option<TimestampedStrandEvent>`. The existing `dedup_opt_key` pattern already handles this — we just change it to extract the key from the inner `.event` field.
