# Plan: File Watcher with Debounce

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan adds file system watching via the `notify` crate with per-file event debouncing. It detects strand events (create/modify/delete) and emits structured events for downstream processing.

## Problem

Knot can discover looms and maintain state files, but it cannot yet react to file system events. When a strand is created, modified, or deleted in a watched source directory, there is no mechanism to detect and emit these events. Without file watching, the entire reactive pipeline is inert.

## Target

- The `notify` crate is added as a dependency.
- Each loom's source directory is watched for file system events.
- Events are debounced per-file using a 100ms timer — only the last event within the debounce window is emitted.
- Strand events (`Created`, `Modified`, `Deleted`) are emitted as structured events containing the loom ID, knot IDs, and file path.
- The watcher runs as a background task, emitting events through a channel for consumption by the processing pipeline.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |

## Test Gaps

- No tests for file watcher setup and teardown.
- No tests for event emission on create/modify/delete.
- No tests for per-file debouncing (rapid events coalesce to one).
- No tests for watcher restart on error.
- No integration test: create a file → event emitted through channel.

## Phases

### Phase 0: Notify Integration
- [ ] Add `notify` crate as a dependency (`notify = "7"`)
- [ ] Add `tokio::sync::mpsc` channel for event distribution
- [ ] Implement `FileWatcher` struct that wraps `notify::Watcher`
- [ ] Wire watcher to loom source directories
- [ ] Unit tests: watcher starts, watcher receives raw events from notify

### Phase 1: Strand Event Mapping
- [ ] Map `notify::Event` types to strand events: `Create` → `StrandEvent::Created`, `Modify` → `StrandEvent::Modified`, `Remove` → `StrandEvent::Deleted`
- [ ] Filter events to only include files (not directories) within source directories
- [ ] Attach loom context (loom ID, associated knot IDs) to each event
- [ ] Unit tests: create event maps to Created, modify maps to Modified, remove maps to Deleted, directory events are ignored, files outside source dir are ignored

### Phase 2: Per-File Debounce
- [ ] Implement debounce logic: per-file `tokio::time::Instant` tracker
- [ ] When an event arrives for a file, reset a 100ms timer
- [ ] After 100ms of no events for that file, emit the last event
- [ ] If a new event arrives within the window, update the pending event and reset the timer
- [ ] Unit tests: single event emits after 100ms, rapid events on same file emit only the last one, events on different files emit independently, delete event after modify emits delete

### Phase 3: Background Task and Channel
- [ ] Run the watcher as a `tokio::task::spawn` background task
- [ ] Emit debounced strand events through an `mpsc::Sender<StrandEvent>`
- [ ] Provide `FileWatcher::start()` returning `(mpsc::Receiver<StrandEvent>, JoinHandle)`
- [ ] Provide graceful shutdown via `JoinHandle` abort
- [ ] Integration test: create file in watched dir → event received on channel after debounce, modify file → modify event received, delete file → delete event received

## Notes
