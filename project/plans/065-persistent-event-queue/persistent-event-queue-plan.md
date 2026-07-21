# Plan: Persistent Event Queue — Disk-Backed Strand Event Persistence

## Related PRD

This plan contributes to [Persistent Events — Disk-Backed Event Queue](../prds/prd-persistent-events.md).

It implements the full PRD: file-based persistence in `rig/events/`, startup scan-and-restore, atomic writes on push, file removal on pop, and queue operations for listing and deleting pending events. The in-memory `InspectQueue` is replaced by a `StrandEventQueue` trait with `DiskBackedEventQueue` as the primary implementation.

## Problem

Knot queues strand events in-memory via `InspectQueue<Option<TimestampedStrandEvent>>`. When the process stops (Ctrl+C, crash, or restart), all pending events are lost.

Additionally, while events are queued, the user has no way to inspect, reorder, or cancel pending work. The queue is an opaque in-memory structure — the user cannot see "what's next" or decide "I don't want that strand processed right now."

The current event flow is:

```
NotifyEventSource → mpsc::channel → DebounceEngine → InspectQueue (in-mem) → ProcessStrand loop
```

The `InspectQueue` is a `Mutex<VecDeque<T>>` with `tokio::sync::Notify` for signaling. It supports `push()`, `pop()`, `push_or_replace()` (dedup), `snapshot()`, and `notified()` (async wait). The debounce engine pushes timestamped events into it; the process-strand loop pops them.

`InspectQueue` is a concrete type deeply embedded in the pipeline — the debounce engine creates it, `WriteState` snapshots it, and the process-strand loop reads from it directly. There is no abstraction layer that allows swapping in an alternative backing store.

## Target

A trait-based queue where the **primary implementation is disk-backed**:

- `StrandEventQueue` trait defines `push`, `push_or_replace`, `pop`, `snapshot`, `delete`, `pending_event`, and `notified`
- `DiskBackedEventQueue` implements the trait using `rig/events/{id}.json` files
- Every event pushed is written to disk (atomic: temp file → rename)
- On startup, `rig/events/` is scanned and loaded into the queue before processing begins
- When an event is popped, its file is removed from disk
- When an event is deleted (via API or manual), both the queue entry and its file are removed
- A user can modify a pending event's file on disk — the updated content is read on pop
- Malformed event files are skipped with a warning logged to stderr
- The debounce engine, process-strand loop, and `WriteState` are adapted to use the trait

```
┌─────────────────────────────────────────────────────────────────┐
│  NotifyEventSource                                              │
│       │                                                         │
│       ▼                                                         │
│  mpsc::channel → DebounceEngine                                 │
│                      │                                          │
│                      │ push_or_replace(event)                   │
│                      ▼                                          │
│           ┌────────────────────┐                                │
│           │ StrandEventQueue   │                                │
│           │ (trait)            │                                │
│           │                    │                                │
│           │  DiskBackedEventQueue (Arc<dyn StrandEventQueue>)   │
│           │    └── rig/events/*.json (the queue)                │
│           └────────────────────┘                                │
│                      │                                          │
│           pop() ◄────┼────────────────────► remove file         │
│                      │                                          │
│  ProcessStrand loop  │                                          │
│                      │                                          │
│  WriteState ──► snapshot()  (for state.json)                    │
│                      │                                          │
└─────────────────────────────────────────────────────────────────┘
```

### Why a Trait, Not a Wrapper

The previous plan proposed a `PersistentQueue` wrapper around the existing `InspectQueue`. This created a dual-source-of-truth problem (in-memory queue + disk files) and made listing/deletion awkward.

With a trait:

- The disk-backed implementation is the **source of truth** — the disk files ARE the queue
- `snapshot()` returns events with their disk `id`, enabling deletion
- `delete(id)` removes from memory and disk atomically
- `pop()` reads the file fresh (honouring any on-disk edits) then removes it
- No wrapper indirection — the queue IS the persistence layer
- Tests can use an in-memory mock implementation

## Architecture

### `PendingEvent` Domain Model

A serialisable form of a strand event with a unique ID, stored as JSON on disk.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEvent {
    pub id: PendingEventId,
    pub kind: String,          // "Created", "Modified", "Deleted"
    pub loom_id: String,
    pub knot_id: String,
    pub strand_path: String,
    pub queued_at: String,     // ISO 8601
}
```

`PendingEventId` is a newtype wrapping the unique file ID string (`{unix_timestamp_ms}-{4-hex-chars}`).

### `PendingEventOrShutdown` Enum

Replaces `Option<TimestampedStrandEvent>` as the queue value type. The shutdown sentinel is an explicit variant rather than `None`:

```rust
#[derive(Debug, Clone)]
pub enum PendingEventOrShutdown {
    Event(PendingEvent),
    Shutdown,
}
```

This avoids the `Option<T>` indirection and makes the shutdown signal self-documenting. The debounce engine pushes `PendingEventOrShutdown::Shutdown` on channel close (same as pushing `None` today).

### File Naming and Storage

`{unix_timestamp_ms}-{4-hex-chars}.json` in `rig/events/`.

- `unix_timestamp_ms` ensures FIFO ordering by filename sort
- `4-hex-chars` (16 bits of randomness) prevents collisions within the same millisecond
- The file ID matches the `PendingEvent.id` field
- No subdirectories — flat directory for simplicity

Persisted JSON schema:

```json
{
  "id": "1750000000000-a3f7",
  "kind": "Created",
  "loom_id": "review-loom",
  "knot_id": "reviewer",
  "strand_path": "/home/user/project/src/main.rs",
  "queued_at": "2026-07-21T10:30:00+01:00"
}
```

### Deduplication

Dedup key: `(strand_path, loom_id, knot_id, kind)`.

`push_or_replace()` scans `rig/events/*.json` sorted by filename. For each file it parses the event and checks the dedup key. If a match is found, the existing file is removed. A new file with a fresh ID (new timestamp) is always written. This means the deduped event moves to the back of the queue (newer timestamp), which is correct — the latest change should be processed last.

### `StrandEventQueue` Trait

```rust
pub trait StrandEventQueue: Send + Sync {
    /// Push an event. Returns the assigned ID.
    fn push(&self, event: PendingEvent) -> PendingEventId;

    /// Push an event, or replace an existing one with the same dedup key.
    /// Returns the ID of the (new or replaced) entry.
    fn push_or_replace(&self, event: PendingEvent) -> PendingEventId;

    /// Pop the next event from the queue (FIFO).
    ///
    /// Returns `Some(PendingEventOrShutdown::Event)` for real events,
    /// `Some(PendingEventOrShutdown::Shutdown)` for the shutdown sentinel,
    /// or `None` if the queue is empty (no sentinel, no events).
    ///
    /// On pop, the disk file is removed. If the file was modified on disk
    /// since it was pushed, the updated content is returned.
    fn pop(&self) -> Option<PendingEventOrShutdown>;

    /// Take a snapshot of all pending events (excludes shutdown sentinel).
    fn snapshot(&self) -> Vec<PendingEvent>;

    /// Delete a specific pending event by ID (removes the disk file).
    /// Returns `true` if the event was found and removed, `false` otherwise.
    fn delete(&self, id: &PendingEventId) -> bool;

    /// Get a single pending event by ID (reads from disk for latest content).
    fn pending_event(&self, id: &PendingEventId) -> Option<PendingEvent>;

    /// Return the number of pending events (excludes shutdown sentinel).
    fn len(&self) -> usize;

    /// Check if the queue is empty (no events, no sentinel).
    fn is_empty(&self) -> bool;

    /// Push a shutdown sentinel (called by debounce engine on channel close).
    fn push_shutdown(&self);

    /// Await a signal that an item was pushed.
    async fn notified(&self);
}
```

### `DiskBackedEventQueue` Implementation

The disk **is** the queue. No in-memory deque — every operation reads from `rig/events/` and orders by filename sort. This eliminates any dual-source-of-truth: there is only the disk.

Internal state: `tokio::sync::Notify` for signaling when files change, plus a flag for the shutdown sentinel.

Key behaviours:

- **`push`**: write `{id}.json` atomically (temp → rename), signal `Notify`
- **`push_or_replace`**: scan `*.json` files sorted by filename; check each event's dedup key; if match found, remove old file; always write new file. Signal `Notify`
- **`pop`**: scan `*.json` files sorted by filename; read the first file; remove it; return as `PendingEventOrShutdown::Event`. If shutdown flag is set and no files remain, return `Shutdown`
- **`snapshot`**: scan `*.json` files sorted by filename, parse each, return `Vec<PendingEvent>`
- **`delete`**: remove `{id}.json` from disk; return `true` if file existed
- **`pending_event`**: read `{id}.json` from disk, parse and return
- **`notified`**: delegate to inner `tokio::sync::Notify`

The shutdown sentinel is not persisted — it is a runtime-only flag set when the debounce engine closes its channel. On restart, only `Event` files are loaded.

### Debounce Engine Integration

The debounce engine is modified to accept a `StrandEventQueue` from the caller rather than creating its own `InspectQueue`:

- New method: `spawn_with_receiver_with_window_and_queue()` accepts the queue and timing params
- The debounce engine's expired-event handler calls `queue.push_or_replace(event)` and `queue.push_shutdown()` on channel close
- Existing `spawn_with_receiver_with_window()` is removed (no backward-compat needed — the queue is always caller-owned)

### Process-Strand Loop Adaptation

The process-strand loop in `server.rs` is adapted to use the trait:

- `pop()` now returns `Option<PendingEventOrShutdown>` instead of `Option<Option<TimestampedStrandEvent>>`
- Match on `PendingEventOrShutdown::Event` (process) or `PendingEventOrShutdown::Shutdown` (break)
- `notified()` is the same async wait
- `PendingEvent` is converted to `StrandEvent` before passing to `ProcessStrand::execute()`

### `WriteState` Adaptation

`WriteState` currently holds `Arc<Mutex<Option<Arc<InspectQueue<...>>>>>`. It is adapted to:

- Hold `Option<Arc<dyn StrandEventQueue>>` instead
- `build_queue_entries()` calls `queue.snapshot()` and maps `PendingEvent` to `RigStateStrandQueueEntry`
- The `StrandQueueAccessor` domain trait implementation is updated to work with the new queue

### `StrandQueueAccessor` Domain Trait

The domain-layer `StrandQueueAccessor` trait (`pending_strand_paths()`) is implemented by `DiskBackedEventQueue`. This keeps the domain's ability to query pending strand paths without depending on the concrete queue type.

### Startup Sequence

Current `run_startup()`:
1. Create rig directory
2. Create `.workspace-agent-config.yaml` if missing
3. Run `DiscoverLooms`
4. Register rig directory watch

New sequence (inserted after rig directory creation, before discovery):
1. Create `rig/events/` directory if missing
2. Scan `rig/events/` for `*.json` files
3. Parse each file into `PendingEvent`; on success, push into the queue via `push_or_replace()`
4. On parse failure, skip with warning to stderr
5. Continue with existing startup steps

This ensures persisted events are loaded **before** the file-watcher begins emitting new events, preserving FIFO ordering across restart boundaries.

### QueueIdle Detection

The existing `QueueIdle` detection (500ms poll after last event) works unchanged. When no files exist in `rig/events/` and no events arrive within 500ms, the queue is idle.

### Event File Modification While Running

The disk file is the only source of truth. When a user edits a file in `rig/events/` while Knot is running:
- On `pop()`, the file is read from disk — the updated content is used for processing
- On `snapshot()`, files are read from disk — the updated content is shown
- On `push_or_replace()`, the dedup key is computed from the file on disk (not cached in memory)

There is no in-memory index — the queue is just `rig/events/*.json` sorted by filename.

## Existing Tests

| Test Class | What it covers | Status |
|---|---|---|
| `debounce.rs` — `InspectQueue` tests | push/pop FIFO, push_or_replace dedup, snapshot, notified async | ✅ Green (12 tests) |
| `debounce.rs` — `DebounceEngine` tests | single event, rapid dedup, different files, shutdown sentinel | ✅ Green (8 tests) |
| `events.rs` — domain event tests | `StrandEvent` variants, serialization round-trip | ✅ Green |
| `server.rs` — composition tests | wiring, rig config loading, startup config creation | ✅ Green (5 tests) |
| `pipeline.rs` — integration tests | full event flow, shutdown, tie-off, rig-log | ✅ Green |
| `write_state.rs` — queue visibility tests | strand_queue in state.json snapshot | ✅ Green |
| `temp_file.rs` | temp file pattern detection | ✅ Green (9 tests) |

## Test Gaps

- No test for file-based event persistence (write to disk, read from disk)
- No test for startup event restoration from disk
- No test for atomic file write (temp → rename)
- No test for malformed event file handling during startup scan
- No integration test for restart survival (events persist across process stop/start)
- No test for event file removal after processing
- No test for event deletion (by ID)
- No test for event modification on disk (read fresh on pop)
- `InspectQueue` tests become obsolete — replaced by trait adapter tests

## Phases

### Phase 0: Pending Event Format and File Naming

Define the persisted event data model, file naming convention, and conversion to/from `StrandEvent`.

**New file:** `src/domain/pending_event.rs`

- `PendingEventId` — newtype wrapping the unique ID string
- `generate_event_id()` — produces `{unix_timestamp_ms}-{4-hex-chars}`
- `PendingEvent` — serialisable struct with `id`, `kind`, `loom_id`, `knot_id`, `strand_path`, `queued_at`
- `PendingEventOrShutdown` — enum with `Event(PendingEvent)` and `Shutdown` variants
- `dedup_key(event: &PendingEvent) -> DedupKey` — derives `(strand_path, loom_id, knot_id, kind)`
- `From<StrandEvent> for PendingEvent` — converts a `StrandEvent` to `PendingEvent` (generates new ID)
- `TryFrom<PendingEvent> for StrandEvent` — converts back, returns error for unknown `kind`

**Tests:**
- `generate_event_id` produces sortable unique IDs (10 rapid IDs are all unique and sorted by creation order)
- `PendingEvent::from(StrandEvent)` round-trips all three variants (Created, Modified, Deleted)
- `TryFrom<PendingEvent> for StrandEvent` succeeds for valid events
- `TryFrom` returns error for unknown `kind` string
- JSON serialization/deserialization round-trip for all variants
- `dedup_key` produces same key for same (path, loom, knot, kind)
- `dedup_key` produces different key for same path but different kind
- `PendingEventOrShutdown` variant matching

### Phase 1: StrandEventQueue Trait and Port Error

Define the application-layer port trait and error type.

**Changes to `src/application/ports.rs`:**
- Add `PortError::EventStoreFailed(String)` variant for file I/O errors
- Define `StrandEventQueue` trait with all methods (see Architecture section above)

**New file:** `src/application/event_queue.rs` — re-exports the trait and types

**Tests:**
- Trait is object-safe (`&dyn StrandEventQueue` compiles)
- `PortError::EventStoreFailed` displays correctly
- `PortError::EventStoreFailed` implements `std::error::Error`

### Phase 2: DiskBackedEventQueue Implementation

Implement the file-backed queue adapter.

**New file:** `src/adapters/outbound/event_store.rs` — `FileSystemEventStore` (low-level file CRUD):
- `new(events_dir: PathBuf)` — constructor, creates directory
- `write_event(event: &PendingEvent) -> Result<PathBuf, io::Error>` — atomic write (temp → rename)
- `remove_event(id: &PendingEventId) -> Result<(), io::Error>` — remove file (no-op if missing)
- `scan_events() -> Result<Vec<PendingEvent>, io::Error>` — scan `*.json`, parse each, skip malformed with warning, return sorted by filename
- `read_event(id: &PendingEventId) -> Result<PendingEvent, io::Error>` — read and parse single file
- `event_count() -> usize` — count `.json` files

**New file:** `src/adapters/outbound/disk_event_queue.rs` — `DiskBackedEventQueue`:
- Internal state: `tokio::sync::Notify` for signaling, plus a flag for the shutdown sentinel. No in-memory event storage.
- `new(events_dir: PathBuf)` — constructor
- `load_persisted() -> usize` — scan store, push events via `push_or_replace()`, return count
- Implements `StrandEventQueue` trait
- Implements `StrandQueueAccessor` (domain trait) — `pending_strand_paths()` from snapshot

**Tests for `FileSystemEventStore`:**
- `write_event` creates file with correct JSON content (atomic: temp then rename)
- `write_event` creates parent directory if missing
- `remove_event` deletes the file
- `remove_event` is no-op for missing file
- `scan_events` returns empty vec for empty directory
- `scan_events` returns events sorted by filename
- `scan_events` skips malformed JSON files with warning
- `scan_events` skips non-JSON files
- `read_event` reads and parses a single file
- `read_event` returns error for missing file
- `event_count` returns correct count
- Round-trip: write 3 events, scan, verify all 3 returned in order

**Tests for `DiskBackedEventQueue`:**
- `push` writes file to disk; `snapshot` returns it
- `pop` reads first file (by filename sort), removes it from disk, returns content
- `pop` returns `None` when no files exist and shutdown not signalled
- `push_or_replace` replaces existing event (same dedup key): old file removed, new file written
- `push_or_replace` pushes new event when no match (different dedup key)
- `delete` removes file from disk; returns `true` if file existed
- `delete` returns `false` for non-existent ID
- `pending_event` reads file from disk and returns event
- `pending_event` returns `None` for non-existent ID
- `snapshot` returns all events sorted by filename
- `len` returns correct count
- `is_empty` works correctly
- `push_shutdown` sets sentinel flag; after last event popped, `pop` returns `Shutdown`
- `notified` unblocks after `push`
- Round-trip: push 3 events, pop all 3, disk is empty
- Push events, don't pop (simulate crash), create new queue, verify events present on disk
- Modify event file on disk, `pop` returns updated content
- `snapshot` after modify returns updated content

### Phase 3: Debounce Engine Integration

Modify `DebounceEngine` to accept a `StrandEventQueue` instead of creating its own `InspectQueue`.

**Changes to `debounce.rs`:**
- Remove `InspectQueue` and `QueueReceiver` (replaced by trait)
- Add `spawn_with_receiver_with_window_and_queue(input_rx, join_set, window, check_interval, queue: Arc<dyn StrandEventQueue>)`
- Internal event loop calls `queue.push_or_replace(PendingEventOrShutdown::Event(event))` for expired events
- On channel close, flushes all pending via `push_or_replace`, then calls `queue.push_shutdown()`
- Remove `start()`, `start_with_window()`, `start_with_receiver()`, `start_with_receiver_with_window()`, `spawn_with_receiver()`, `spawn_with_receiver_with_window()` (all replaced by queue-accepting variants)
- `TimestampedStrandEvent` replaced by `PendingEvent` (which already has `queued_at`)
- `dedup_opt_key` replaced by `dedup_key` on `PendingEvent`

**Tests:**
- Debounce engine with disk queue: events are pushed through the queue
- Debounce engine with mock queue: existing dedup behaviour preserved
- Shutdown flush uses the queue
- Shutdown sentinel arrives as `PendingEventOrShutdown::Shutdown`
- Rapid events for same file produce exactly one queued event
- Different files emit independently
- Delete-after-modify emits delete

### Phase 4: Server Startup and Pipeline Integration

Wire `DiskBackedEventQueue` into `server.rs` startup and pipeline.

**Changes to `server.rs`:**
- `build_app_context()`: create `DiskBackedEventQueue`, store as `Arc<dyn StrandEventQueue>` in `AppContext`
- `AppContext`: replace `strand_queue: Arc<Mutex<Option<StrandQueue>>>` with `strand_queue: Option<Arc<dyn StrandEventQueue>>`
- `run_startup()`: after creating rig directory, create `rig/events/`, call `queue.load_persisted()` and log count
- `start_event_pipeline()`: pass `Arc<dyn StrandEventQueue>` to `DebounceEngine::spawn_with_receiver_with_window_and_queue()`
- Process-strand loop: match on `PendingEventOrShutdown::Event` (convert to `StrandEvent`, process) or `PendingEventOrShutdown::Shutdown` (break)
- `start_state_writer()`: pass queue to `WriteState`

**Changes to `write_state.rs`:**
- Replace `StrandQueueRef` type alias with `Option<Arc<dyn StrandEventQueue>>`
- `build_queue_entries()`: call `queue.snapshot()`, map `PendingEvent` to `RigStateStrandQueueEntry`

**Tests:**
- `run_startup` creates `rig/events/` directory
- `run_startup` loads persisted events (pre-written files in temp rig)
- `start_event_pipeline` wires queue to debounce engine
- `WriteState` builds queue entries from `DiskBackedEventQueue` snapshot

### Phase 5: Integration Test — Full Persistence Cycle

End-to-end test using the full pipeline with real filesystem.

**New file:** `tests/persistent_queue.rs`

**Tests:**
- Full cycle: start pipeline → create strand file → event debounced → processed → verify file exists in `rig/events/` during processing → verify file removed after processing
- Restart survival: push events, stop pipeline (simulate crash), restart pipeline → verify persisted events are loaded and processed
- Malformed file handling: write invalid JSON to `rig/events/`, restart → verify warning logged, processing continues with valid events
- Empty events directory: clean restart → no events loaded, clean startup
- Multiple events: push 5 events, verify 5 files exist, process all, verify 0 files remain
- Delete from queue: push event, call `queue.delete(id)`, verify event removed from queue and disk, not processed
- Modify on disk: push event, edit file on disk, process event → verify event processed with updated content

### Phase 6: Cleanup

Remove obsolete types and wire final state.

**Changes:**
- Remove `InspectQueue`, `QueueReceiver`, `TimestampedStrandEvent` from `debounce.rs`
- Remove `StrandQueue` type alias from `server.rs`
- Update domain glossary: add `Events Directory` term documenting `rig/events/` layout and purpose

**Tests:**
- All existing tests still pass (adapted for new types)
- `cargo test` passes

## Notes

- **No new dependencies** — all functionality uses standard library + existing dependencies (chrono, tokio, serde).
- **`InspectQueue` is removed** — not kept alongside the trait. The trait is the abstraction; `DiskBackedEventQueue` is the implementation. Tests use a mock implementation.
- **Dedup is preserved** — `push_or_replace()` uses the same `(strand_path, loom_id, knot_id, kind)` key. When replacing, the old disk file is removed and a new one written.
- **Atomic writes** prevent partial files from being read on startup. `FileSystemEventStore` writes to `{id}.json.tmp` then renames to `{id}.json`.
- **Shutdown sentinel** — `PendingEventOrShutdown::Shutdown` replaces `Option<T>`. It is NOT persisted to disk (it's a runtime signal only). On startup, only `Event` files are loaded.
- **File modification while running** — `pop()` reads the file fresh from disk, so any on-disk edits are honoured when the event is processed. `snapshot()` also reads fresh for accurate listing.
- **Performance** — file I/O on every push/pop is acceptable. Knot is not a high-throughput queue; it processes events at human timescales (agent invocations take seconds to minutes). The debounce window (100ms) already serialises rapid bursts.
