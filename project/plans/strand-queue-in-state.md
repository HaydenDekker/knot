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

## Implementation Status: ✅ Complete (2026-07-01)

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

- [x] Add `RigStateStrandQueueEntry` struct to `entities.rs`:
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
- [x] Add `strand_queue: Vec<RigStateStrandQueueEntry>` to `RigState` (serialises as empty array when empty — no `skip_serializing_if` needed since `Vec` serialises to `[]`)
- [x] Create `TimestampedStrandEvent` wrapper in `debounce.rs`:
  ```rust
  #[derive(Debug, Clone)]
  struct TimestampedStrandEvent {
      event: StrandEvent,
      queued_at: String,
  }
  ```
- [x] Change `InspectQueue` in the debounce pipeline from `InspectQueue<Option<StrandEvent>>` to `InspectQueue<Option<TimestampedStrandEvent>>`
- [x] Update debounce engine's `push_or_replace` / `push` calls to wrap events in `TimestampedStrandEvent` with `format_timestamp()`
- [x] Update `dedup_opt_key` to work with `Option<TimestampedStrandEvent>` (key extracted from inner `.event`)
- [x] Update `QueueReceiver::recv()` return type to `Option<TimestampedStrandEvent>`
- [x] Update ProcessStrand consumer in `server.rs` to unwrap `.event` from `TimestampedStrandEvent`
- [x] Unit tests: `RigStateStrandQueueEntry` construction + serialization roundtrip
- [x] Unit tests: `RigState` with `strand_queue` field serialises correctly (including empty queue)
- [x] Update existing debounce tests to compile against `TimestampedStrandEvent`

### Phase 1: Expose queue snapshot through `InspectQueue`

**File:** `src/application/debounce.rs`

Add a method to snapshot the current queue contents for the state writer.

- [x] Add `snapshot(&self) -> Vec<T>` method to `InspectQueue` — locks the mutex and clones the `VecDeque` into a `Vec`. Generic over `T: Clone`; callers working with `Option<T>` filter `None` sentinels themselves
- [x] Unit test: snapshot returns current items in FIFO order
- [x] Unit test: snapshot excludes `None` sentinel
- [x] Unit test: snapshot is empty when queue is empty

### Phase 2: Wire queue `Arc` into `AppContext` and `WriteState`

**Files:** `src/server.rs`, `src/application/usecases/write_state.rs`

Share the queue reference with the state writer.

- [x] Add `strand_queue: Arc<Mutex<Option<StrandQueue>>>` field to `AppContext` in `server.rs` (interior mutability via `Mutex` avoids borrow conflicts with `Clone`)
- [x] `start_event_pipeline()` stores the `Arc<InspectQueue>` into `AppContext.strand_queue` via the Mutex, and returns the Arc
- [x] Add `strand_queue: StrandQueueRef` parameter to `WriteState::new()` and `WriteState` struct
- [x] In `WriteState::build_state()`, call `queue.snapshot()` if queue is present, map each `TimestampedStrandEvent` to `RigStateStrandQueueEntry`, and attach to `RigState`
- [x] In `start_state_writer()` in `server.rs`, clone the queue `Arc<Mutex<...>>` and pass to `WriteState::new()`
- [x] In `build_app_context()`, initialise `strand_queue` as `Arc::new(Mutex::new(None))`
- [x] Unit tests in `write_state.rs`:
  - `build_state_empty_queue` — queue present but empty, `strand_queue` is `[]`
  - `build_state_with_queued_events` — queue has events, `strand_queue` populated correctly (Created/Modified/Deleted all mapped, None sentinel filtered)
  - `build_state_no_queue` — queue is `None` inside Mutex (backward compat), `strand_queue` is `[]`
- [x] Update existing `WriteState` tests to pass the new parameter

### Phase 3: Integration verification

**File:** `tests/pipeline.rs`

End-to-end verification that the strand queue appears in `state.json`.

- [x] Integration test: `strand_queue_visible_in_state_during_processing` — creates multiple strands with a slow mock agent (8s), verifies at least one entry appears in `state.json.strand_queue` with correct fields (path, loom_id, knot_id, event_type, queued_at)
- [x] Integration test: `strand_queue_empty_after_processing` — creates a strand with a fast agent, waits for processing to complete, then verifies `strand_queue` is empty in `state.json`
- [x] Integration test: `strand_queue_multiple_events_visible` — creates 3 strands with a slow mock agent, verifies at least 2 entries appear in `strand_queue` with correct structure and paths
- [x] Run full test suite — 472 unit tests + 30 integration tests pass (parallel test interference from PATH manipulation is pre-existing)

## Notes

The `InspectQueue` is already thread-safe (`Mutex<VecDeque<T>>`), so sharing via `Arc` is straightforward. The `snapshot()` method holds the lock briefly (just cloning the vec), which is acceptable given the 5-second state write interval — no contention with the 5ms debounce check cycle.

`TimestampedStrandEvent` wraps the event inside the queue only — the debounce engine's internal `pending` HashMap still uses raw `StrandEvent` (the timestamp is captured when the debounced event is emitted, i.e. when it enters the output queue). This is the right semantics: `queued_at` = "ready for processing", not "raw event received".

The dedup key for `InspectQueue` must work on `Option<TimestampedStrandEvent>`. The existing `dedup_opt_key` pattern already handles this — we just change it to extract the key from the inner `.event` field.

## Completion

- Phase 0–3 all complete, tests passing
- Version bumped to `0.21.0` (MINOR — new backwards-compatible field in `state.json`)
- Full test suite passes (472 unit tests + 30 integration tests)
