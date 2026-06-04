# Plan: Knot-Per-Strand Config and Loom-Log State Consolidation

## Problem

Two simplifications are needed to reduce indirection and file sprawl:

1. **Loom-level config is too coarse.** Currently `.loom-config.yaml` defines `source_dir` and `tie_off_dir` at the loom level. Every knot in a loom shares the same watch directory and output directory. A knot that needs to watch a different source directory or write to a different output directory cannot be configured — the loom's single config applies to all knots.

2. **Knot-state files are redundant.** Each knot has a `.state` file under `<loom_dir>/.knots/<knot-name>.state` that records processing events and status. The same information is already tracked in the loom-log (`.loom-log`) as `LoomEvent` entries. Maintaining two files for the same lifecycle information is unnecessary indirection.

## Target

1. **Per-knot source and tie-off directories.** Each knot can define its own `source_dir` and `tie_off_dir` in the knot definition file (YAML frontmatter). The loom-level `.loom-config.yaml` is removed — knot config is self-contained. The `NotifyEventSource` watches each knot's source directory independently and routes events to the correct knot.

2. **Knot-state events append to loom-log.** Knot lifecycle events (created, processing, completed, failed) are appended to `.loom-log` as `LoomEvent` entries instead of writing separate `.state` files. The `KnotStatePort` is removed. The HTTP endpoint `GET /looms/:id/knots/:knot_name` derives status from the loom-log.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class / Module | What it covers | Status |
|---|---|---|
| `src/adapters/outbound/loom_repository.rs` | Loom config parsing (`.loom-config.yaml`), `source_dir`/`tie_off_dir` resolution, knot file scanning | ✅ Green — 19 tests |
| `src/adapters/outbound/knot_state.rs` | Per-knot `.state` file CRUD (create, update, get) | ✅ Green — 5 tests |
| `src/adapters/outbound/loom_log.rs` | `.loom-log` JSONL append/read, concurrent writes | ✅ Green — 6 tests |
| `src/adapters/outbound/event_source.rs` | `NotifyEventSource` watching, event mapping (create/modify/delete), filtering | ✅ Green — 7 tests |
| `src/application/ports.rs` | Port trait contracts, `KnotStatePort`, `KnotState` struct | ✅ Green — 14 tests |
| `src/application/usecases.rs` | `ProcessStrand`, `DiscoverLooms`, `RegisterLoom`, `GetKnotStatus` | ✅ Green — 20+ tests |
| `src/adapters/inbound/mod.rs` | HTTP handlers for looms/knots, route wiring | ✅ Green — 15 tests |
| `tests/integration.rs` | Full server startup, file watcher, loom discovery, event pipeline | ✅ Green — 15+ tests |
| `tests/http_interface.rs` | HTTP endpoint smoke tests | ✅ Green |
| `tests/filesystem_interface.rs` | Filesystem adapter tests | ✅ Green |

## Test Gaps

- No test verifying that multiple knots in one loom can have different `source_dir` values
- No test for knot-state events appearing in the loom-log (they are separate systems today)
- No test for deriving knot status from loom-log entries (currently uses `.state` file)
- No test for `NotifyEventSource` with per-knot source directories

## Architecture Impact

This plan touches all hexagonal layers:

- **Domain** — `Knot` entity gains `source_dir` and `tie_off_dir` fields; `KnotFile` parser gains new frontmatter fields
- **Application** — `KnotStatePort` is removed; `GetKnotStatus` derives status from `LoomLogPort`; `ProcessStrand` uses per-knot paths
- **Outbound adapters** — `knot_state.rs` is removed; `loom_repository.rs` stops reading `.loom-config.yaml`; `event_source.rs` watches per-knot directories; `loom_log.rs` may need new event variants for knot lifecycle
- **Inbound adapter** — `get_knot_status` handler derives status from loom-log instead of knot-state store
- **Composition root** — `AppContext` loses `knot_state_port`; `build_app_context` no longer creates `FileSystemKnotStateStore`

## Phases

### Phase 0: Domain — Add source_dir and tie_off_dir to Knot

- [ ] Add `source_dir: Option<PathBuf>` and `tie_off_dir: Option<PathBuf>` to the `Knot` entity
- [ ] Add new frontmatter fields to `KnotFile` parser (`source-dir`, `tie-off-dir`) with validation
- [ ] Update `Knot` construction tests in `entities.rs`
- [ ] Update `KnotFile` parse tests in `knot_file.rs`
- [ ] **TDD**: Write a failing test that parses a knot file with `source-dir`/`tie-off-dir` fields → implement parsing

### Phase 1: Domain — New LoomEvent variants for knot lifecycle

- [ ] Add new `LoomEvent` variants: `KnotProcessing`, `KnotCompleted`, `KnotFailed` — each carrying `knot_id`, `strand_path`, `tie_off_path`, and optional error
- [ ] These replace the information currently tracked in `KnotState`
- [ ] **TDD**: Write tests for the new event variants (construction, serialization round-trip)

### Phase 2: Application — Remove KnotStatePort, derive status from loom-log

- [ ] Remove `KnotStatePort` trait from `ports.rs` (and `KnotState`, `ProcessingStatus`, `KnotEventType` structs if no longer needed as port types — they become loom-log event data)
- [ ] Update `GetKnotStatus` use case: scan loom-log for latest event for a given `knot_id`, derive status from that
- [ ] Update `ProcessStrand` use case: append `LoomEvent` entries instead of calling `KnotStatePort::update()`
- [ ] Update `DiscoverLooms` use case: remove `KnotStatePort::create()` calls
- [ ] Update `RegisterLoom` use case: remove `KnotStatePort::create()` calls
- [ ] **TDD**: Write tests for `GetKnotStatus` deriving status from loom-log entries

### Phase 3: Outbound Adapters — Per-knot config, remove knot-state adapter

- [ ] Remove `FileSystemKnotStateStore` adapter (`knot_state.rs`)
- [ ] Remove `.loom-config.yaml` parsing from `FileSystemLoomRepository`; resolve `source_dir`/`tie_off_dir` per-knot (from knot definition, defaulting to loom directory)
- [ ] Update `FileSystemLoomRepository::scan()` — each knot gets its own paths
- [ ] Update `NotifyEventSource` — support watching multiple source directories with per-directory knot ID mapping (already partially supported via `with_loom_ids`)
- [ ] **TDD**: Tests verify that `FileSystemLoomRepository::scan()` produces knots with correct per-knot paths

### Phase 4: Inbound Adapter — Update handlers

- [ ] Remove `knot_state_port` from `AppContext`
- [ ] Update `get_knot_status` handler to use `GetKnotStatus` backed by `LoomLogPort`
- [ ] Update `get_loom_activity` handler (no change needed — already uses `LoomLogPort`)
- [ ] Update handler tests to use new `GetKnotStatus` signature
- [ ] **TDD**: Write handler tests that verify knot status is derived from loom-log

### Phase 5: Composition Root — Re-wire

- [ ] Remove `KnotStatePort` from `AppContext`
- [ ] Remove `FileSystemKnotStateStore` creation from `build_app_context`
- [ ] Update `start_server_with_shutdown` — no knot-state cleanup needed
- [ ] Update `run_startup` — no knot-state creation needed
- [ ] Update `NotifyEventSource` wiring — watch per-knot source directories
- [ ] **TDD**: Integration test verifies server starts with per-knot source dirs

### Phase 6: Integration Tests and Verification

- [ ] Update `tests/integration.rs` tests to reflect new behaviour
- [ ] Update `tests/http_interface.rs` tests
- [ ] Update `tests/filesystem_interface.rs` tests
- [ ] Verify full compile: `cargo build`
- [ ] Verify full test suite: `cargo test`
- [ ] Verify no regressions in existing endpoint behaviour

## Notes

- The `KnotState` struct from the application layer may be preserved as a query DTO for the HTTP endpoint (renamed or kept as-is), even though the `.state` file adapter is removed. The data is derived from loom-log, not stored.
- The `ProcessingStatus` and `KnotEventType` enums from `ports.rs` can be moved to `LoomEvent` variants or kept as query response types depending on how the API contract is structured.
- The `.loom-config.yaml` file is fully removed. Knot definitions become self-contained — all configuration lives in the knot file frontmatter.
- Default paths: if a knot omits `source-dir`, it defaults to the loom directory. If it omits `tie-off-dir`, it defaults to `<loom_dir>/.knot-output`. This preserves backward compatibility for simple looms.
