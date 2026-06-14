# Plan: Knot Modification Observability and Path Resolution Consistency

## Problem

Two issues with knot configuration changes:

1. **`KnotModified` events are invisible** — editing a knot `.md` file on disk updates the in-memory store, but there is no observable confirmation. The `ConfigEventHandler` logs to stderr only. No `LoomEvent` is appended to the loom-log, so `GET /looms/{id}/activity` shows nothing. There is no `LoomEvent::KnotUpdated` variant.

2. **Path resolution inconsistency** — `FileSystemLoomRepository::scan()` resolves `strand_dir`/`tie_off_dir` to absolute paths on initial load, but `NotifyEventSource` (processing `KnotAdded`/`KnotModified` events) used raw relative paths from the parsed `KnotFile`. This meant the in-memory knot after a file edit could have relative paths while the original had absolute paths, causing watcher mismatches.

3. **Silent parse failures** — when `knot_file::parse()` fails on a file watched by the notify watcher, the error is silently dropped with no log output.

## Target

1. `KnotModified` events append `LoomEvent::KnotUpdated` to the loom-log (visible via `GET /looms/{id}/activity`).
2. Parse failures log to stderr so malformed frontmatter is not silently ignored.
3. Path resolution in `NotifyEventSource` matches `FileSystemLoomRepository` (both resolve to absolute) — ensuring the in-memory knot is consistent whether loaded at startup or via file watcher.

## Implementation Status: ✅ Complete (2026-06-15)

## Existing Tests

| Test | What it covers | Status |
|------|---------------|--------|
| `event_source` module tests (in `event_source.rs`) | Strand events, config events (KnotAdded, KnotModified, KnotDeleted), watch/unwatch | ✅ Green — 11 tests |
| `loom_repository` module tests (in `loom_repository.rs`) | `scan()`, path resolution, knot parsing from files | ✅ Green — 20+ tests |
| `config_handler_tests` (in `usecases.rs`) | `ConfigEventHandler` for all 4 ConfigEvent variants | ✅ Green — 8 tests |
| `knot_file` module tests (in `knot_file.rs`) | Frontmatter parsing, validation, error cases | ✅ Green — 16 tests |
| `tests/auto_discovery_and_knot_crud.rs` | Integration: loom discovery, knot CRUD via filesystem | ✅ Green — 9 tests |
| `tests/pipeline.rs` | Integration: full pipeline (watch → debounce → process) | ✅ Green |

## Test Gaps

- No test for `LoomEvent::KnotUpdated` (variant does not exist yet)
- No test verifying `ConfigEventHandler` appends loom-log on `KnotModified`
- No test verifying parse failure produces stderr output
- No integration test for path consistency between initial load and file-watcher update
- No unit test for `resolve_path` in `event_source.rs` (duplicates `loom_repository` logic)

## Phases

### Phase 0: Path Resolution Consistency ✅ Done

Resolve `strand_dir`/`tie_off_dir` to absolute paths in `NotifyEventSource` during event mapping, matching `FileSystemLoomRepository::resolve_path`.

- [x] Add `resolve_path` helper to `event_source.rs` (mirrors `FileSystemLoomRepository::resolve_path`)
- [x] Add `project_root` field to `InnerState` so event mapping can resolve paths
- [x] Update `NotifyEventSource::new()` to accept `project_root` parameter
- [x] Use `resolve_path` in `map_rig_event` and `map_loom_event` when building `Knot` from `KnotFile`
- [x] Update `build_app_context` in `server.rs` to pass `project_root`
- [x] Update existing tests in `event_source.rs` to provide `project_root` (`PathBuf::from("/tmp")`)

**Result:** Uncommitted changes in `src/adapters/outbound/event_source.rs` and `src/server.rs`. `resolve_path` handles absolute paths (uses as-is), relative paths (joins to project_root), and non-existent paths (manual normalisation).

### Phase 1: `LoomEvent::KnotUpdated` and loom-log entry on `KnotModified`

Add a `LoomEvent::KnotUpdated` variant and append it in `ConfigEventHandler::handle_knot_modified`, so the change is visible through the activity log endpoint.

- [ ] Add `LoomEvent::KnotUpdated { loom_id, knot_id }` variant to `domain/events.rs`
- [ ] Add serialization round-trip test for the new variant in `events.rs` tests
- [ ] In `ConfigEventHandler::handle_knot_modified`, append `LoomEvent::KnotUpdated` after updating the store
- [ ] Add unit test in `config_handler_tests` verifying the loom-log entry appears
- [ ] Run full test suite and verify no regressions

### Phase 2: Parse failure logging

Log parse failures from `knot_file::parse()` to stderr so malformed frontmatter is not silently ignored.

- [ ] In `event_source.rs`, change the `parse().map_err(|_| None)` pattern to log the error to stderr via `logging::log_config_event()` or a new helper
- [ ] Log includes: file path, error type (`KnotFileError` variant), and error message
- [ ] Add unit test in `event_source.rs` tests verifying that a malformed `.md` file does NOT emit a config event (existing test covers this — verify it still passes)
- [ ] Run full test suite and verify no regressions

### Phase 3: Integration test for path consistency and visibility

End-to-end test verifying that filesystem edits produce consistent absolute paths and visible loom-log entries.

- [ ] Create test: start server with a loom and knot, verify initial knot has absolute `strand_dir`/`tie_off_dir` via `GET /looms/{id}`
- [ ] Edit the knot file (change model), verify `GET /looms/{id}` reflects the change with absolute paths preserved
- [ ] Check `GET /looms/{id}/activity` for `KnotUpdated` event
- [ ] Run full test suite and verify no regressions

### Phase 4: Final verification

- [ ] `cargo build` succeeds with no warnings
- [ ] `cargo test` — full suite passes (all existing + new tests)
- [ ] `cargo test --test pipeline` — integration pipeline tests pass
- [ ] `cargo test --test auto_discovery_and_knot_crud` — auto-discovery integration tests pass
- [ ] Verify no new clippy warnings: `cargo clippy -- -D warnings`

## Notes

Phase 0 is already implemented as uncommitted changes. The remaining phases add observability and test coverage.

### Phase 0 Bugfix (2026-06-15)

The initial Phase 0 implementation added `resolve_path` to `NotifyEventSource` and wired a `project_root` parameter — but `project_root` was incorrectly set to the rig directory (`config.base_dir`) instead of the project root (parent of the rig). This meant relative `strand_dir` values in knot frontmatter resolved to `<rig>/<strand_dir>` instead of `<project>/<strand_dir>`, causing `No path was found` watcher errors when the strand directory lived alongside the rig (e.g. `project/prds`).

**Fix applied across 7 source files + 17 test files:**
- `server.rs`: `project_root` now derives from `config.rig_dir.parent()` matching `FileSystemLoomRepository::scan()`
- Full rename `base_dir` → `rig_dir` everywhere (`AppConfig`, `AppContext`, `FileSystemLoomLog`, `FileSystemRigLog`, `SharedLoomLog`, `SharedRigLog`, `ProcessStrand`, all constructors, all callers) to eliminate ambiguity between "rig directory" and "project root"
- All tests pass (`cargo test` green across 100+ tests)
