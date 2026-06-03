# Plan: Loom Discovery and State Files

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan implements loom discovery from the filesystem and the loom-log / knot-state file lifecycle. It addresses Story 1 (confirm a knot is active) and the success criteria for loom-log and knot-state files.

## Problem

Knot can define domain types for knots and looms, but it cannot yet discover them from the filesystem. There is no mechanism to scan a workspace directory for loom directories, read knot definition files, and register them. Additionally, the loom-log and knot-state files — the filesystem-backed observability layer — do not exist yet.

## Target

- Knot scans the workspace directory and discovers loom directories (directories containing `.md` knot files).
- Each discovered loom is registered with its knots parsed from definition files.
- A `loom-log` file is created per loom, recording loom-level events (knots detected, loom events).
- A `knot-state` file is created per knot, recording processing events (event type, strand path, tie-off path, status, errors).
- State files are written atomically and are queryable by subsequent HTTP endpoints.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |

## Test Gaps

- No tests for loom directory scanning.
- No tests for knot file discovery within a loom.
- No tests for loom-log creation and appending.
- No tests for knot-state creation and updates.
- No integration test for the full discovery flow (workspace → looms → knots).

## Phases

### Phase 0: Loom Directory Scanner
- [ ] Implement `LoomScanner` that walks the workspace directory looking for loom directories
- [ ] A loom directory is identified by containing `.md` files (knot definition files)
- [ ] For each loom, discover its knot definition files and parse them using `KnotFileParser`
- [ ] Return a `Vec<Loom>` with registered knots
- [ ] Unit tests: scan workspace with one loom, scan with multiple looms, scan with no looms, scan with invalid knot files (skipped with error in log)

### Phase 1: Loom-Log File
- [ ] Define loom-log file format (append-only, one event per line or JSONL)
- [ ] Create `LoomLog` struct with `open(loom_id)`, `append(event)`, `read_all()` methods
- [ ] Log entries: `knot_detected`, `knot_registered`, `knot_error`, `strand_processed`, `loom_started`, `loom_stopped`
- [ ] Write loom-log to a fixed location (e.g. `<loom-dir>/.loom-log`)
- [ ] Unit tests: create new loom-log, append events, read back all events, handle concurrent writes

### Phase 2: Knot-State File
- [ ] Define knot-state file format (JSON, overwritten per update or append-only JSONL)
- [ ] Create `KnotState` struct with `open(knot_id)`, `update(state)`, `read()` methods
- [ ] State fields: `event_type` (created/modified/deleted), `strand_path`, `tie_off_path`, `status` (idle/processing/completed/failed), `error` (optional), `last_updated`
- [ ] Write knot-state to a fixed location (e.g. `<loom-dir>/.knots/<knot-name>.state`)
- [ ] Unit tests: create new knot-state, update state, read current state, status transitions (idle → processing → completed, idle → processing → failed)

### Phase 3: Discovery Integration
- [ ] Integrate `LoomScanner` with `LoomLog` — on discovery, log all detected knots
- [ ] Integrate `KnotState` creation — on knot registration, create initial knot-state file with `idle` status
- [ ] Integration test: given a workspace with loom directories, scanner discovers looms, logs are created, knot-state files are initialised

## Notes
