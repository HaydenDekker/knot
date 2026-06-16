# Plan: Strand Directory Auto-Creation

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

It addresses the goal: *"When a knot's `strand_dir` does not exist at registration time, Knot creates the directory automatically so the knot can begin watching immediately. The creation is recorded in the loom-log."*

## Problem

When a knot's `strand_dir` does not exist on disk at registration time, `event_source.watch()` fails with `EventWatchFailed` because `notify::watch()` requires the target directory to exist. This prevents users from defining knots that point to directories they intend to create later.

## Target

At knot registration (initial loom registration and dynamic knot addition), if `strand_dir` does not exist, Knot creates it automatically and logs the creation in the loom-log. The knot is then registered normally with its watcher active.

## Implementation Status: ⬜ Draft

## Existing Tests
| Test Class | What it covers | Status |
|------------|---------------|--------|
| `loom_log.rs` (unit) | LoomLogPort append/read of all LoomEvent variants | ✅ Green |
| `usecases.rs` (unit) | `register_loom` with mock ports | ✅ Green |
| `auto_discovery_and_knot_crud.rs` (integration) | KnotAdded, KnotModified, KnotDeleted events | ✅ Green |
| `rig_lifecycle.rs` (integration) | Full rig lifecycle including strand processing | ✅ Green |
| `loom_repository.rs` (unit) | `resolve_path` for nonexistent paths (graceful fallback) | ✅ Green |

## Test Gaps

- No test for `register_loom` when `strand_dir` does not exist (currently fails)
- No test for `ConfigEventHandler::handle_knot_added` when `strand_dir` does not exist
- No `LoomEvent::DirectoryCreated` variant or its serialization

## Phases

### Phase 0: Domain — Add `DirectoryCreated` LoomEvent Variant
- [ ] Add `DirectoryCreated` variant to `LoomEvent` enum in `src/domain/events.rs`
  - Fields: `loom_id`, `knot_id`, `directory` (the strand_dir path created), `timestamp`
- [ ] Add `DirectoryCreated` arm to `LoomEvent` match in `FileSystemLoomLog::append()` in `src/adapters/outbound/loom_log.rs` (extracts `loom_id` for file routing)
- [ ] Add unit test in `loom_log.rs` verifying `DirectoryCreated` serialises and round-trips correctly

### Phase 1: Application — Ensure Strand Directory Exists Before Watch
- [ ] Add private helper method `ensure_strand_dir_and_watch(&self, loom_id, knot_id, strand_dir)` on `ConfigEventHandler` in `src/application/usecases.rs`:
  - Check `strand_dir.exists()`
  - If missing: `fs::create_dir_all(&strand_dir)`, log `LoomEvent::DirectoryCreated` via `self.log_port`, log via `logging::log_knot_event()`
  - Call `self.event_source.set_loom_ids()` and `self.event_source.watch()`
- [ ] Replace direct `event_source.watch()` calls in `register_loom()` with the helper
- [ ] Replace direct `event_source.watch()` calls in `handle_knot_added()` with the helper
- [ ] Add unit test in `usecases.rs` using `MockEventSource` + `MockLoomLogPort` + `tempfile` — verifies `DirectoryCreated` event is logged when strand_dir is missing, and watcher is started regardless
- [ ] Add integration test in `tests/auto_discovery_and_knot_crud.rs` — create a knot with nonexistent strand_dir, verify loom-log contains `DirectoryCreated`, verify knot is active

### Phase 2: Knot Modification — Auto-Create on Strand Dir Change
- [ ] In `handle_knot_modified()`, when `strand_dir` changes, also call the helper for the new directory (covers the case where the user updates a knot to point to a new directory)
- [ ] Add unit test verifying `DirectoryCreated` is logged when `handle_knot_modified` changes to a nonexistent dir
- [ ] Verify all existing tests still pass

## Notes

- The helper is application-layer (not domain) because it performs filesystem I/O (`fs::create_dir_all`). The domain only defines the event shape.
- `handle_knot_modified` when strand_dir changes: if the new dir doesn't exist, we create it (same semantics as registration). The old dir is untouched.
- No port trait changes needed — the creation is a use-case-internal decision, not a pluggable behaviour.
