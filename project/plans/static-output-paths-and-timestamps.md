# Plan: Static Output Paths and Log Timestamps

## Problem

Two issues with the current output and logging design:

1. **Tie-off path is unnecessarily configurable.** Every knot requires a `tie-off-dir` in its YAML frontmatter. The value is always "somewhere for this loom's output" — there's no reason to let users vary it. This adds boilerplate to every knot definition and makes output location unpredictable.

2. **Loom-log lives inside `rig/` as a non-loom directory.** The loom-log writes to `rig/{loom-id}/.loom-log`, creating a directory structure that pollutes the rig with non-`-loom` directories. The rig should contain only looms (workflow definitions), not their outputs.

3. **No timestamps on logs.** Console logs (`[KNOT][TAG] ...`) and loom-log JSONL events have no date/time. Tracing when things happened requires guessing or cross-referencing file modification times.

## Target

### New output directory structure

All project outputs live under `rig/output/`, separated from the workflow definitions in `rig/*-loom/`:

```
rig/
  output/                          ← new top-level output directory
    {loom-id}/
      .loom-log                    ← loom activity log (moved from rig/{loom-id}/)
      {knot-name}/
        output.md                  ← tie-off output (one file per knot, appending)
  planning-loom/                   ← workflow definitions (unchanged)
    review-knot.md
    summary-knot.md
```

### Tie-off path

- **Before:** `knot.tie_off_dir / <strand-name>.output` (configured per-knot)
- **After:** `rig/output/{loom-id}/{knot-name}/output.md` (statically derived)

Each knot writes a single `output.md` file that appends all processing events. The file is shared across all strands for that knot (the event metadata in each section identifies which strand was processed).

### Loom-log path

- **Before:** `rig/{loom-id}/.loom-log`
- **After:** `rig/output/{loom-id}/.loom-log`

### Timestamps

- **Console logs:** Prepend ISO 8601 UTC timestamp: `[2026-06-10T12:00:00Z] [KNOT][TAG] ...`
- **Loom-log events:** Add `"timestamp"` field to all `LoomEvent` variants (populated at event creation time).

### Knot file changes

- **Remove** `tie-off-dir` from knot YAML frontmatter (no longer required)
- **Remove** `MissingTieOffDir` error variant from `KnotFileError`
- **Remove** `tie_off_dir` field from `KnotFile` and `Knot` domain entities
- `strand-dir` remains required (strands live in project source, not in `rig/output/`)

## Implementation Status: ⬜ Draft

## Existing Tests

| Test File | What it covers | Status |
|-----------|---------------|--------|
| `src/domain/knot_file.rs` (unit) | Knot YAML parsing, including `tie-off-dir` validation | ⚠️ Must update — `tie-off-dir` removed |
| `src/adapters/outbound/tieoff_sink.rs` (unit) | Tie-off write/append/derive filename | ✅ Green — path derivation moves to usecase |
| `src/adapters/outbound/loom_log.rs` (unit) | Loom-log write/read/concurrent | ⚠️ Must update — log path changes |
| `src/domain/entities.rs` (unit) | `Knot`/`Loom`/`TieOff` construction | ⚠️ Must update — `tie_off_dir` removed |
| `src/adapters/outbound/loom_repository.rs` (unit) | Rig scan, path resolution | ⚠️ Must update — `tie_off_dir` removed from scan |
| `tests/*.rs` (integration) | Full pipeline, tie-off output, loom-log | ⚠️ Must update — output paths change |

## Test Gaps

- No test for the new static path derivation function (`derive_tieoff_path`)
- No test for timestamp formatting helper
- No test for `rig/output/` directory auto-creation

## Phases

### Phase 0: Domain — Remove `tie_off_dir`, Add Timestamp to LoomEvent

Hex layer: **Domain**

- [x] Remove `tie_off_dir: PathBuf` from `Knot` entity in `src/domain/entities.rs`
- [x] Add `timestamp: String` field to all `LoomEvent` variants in `src/domain/events.rs`
- [x] Update `KnotFileError`: remove `MissingTieOffDir` variant
- [x] Update `KnotFile` struct: remove `tie_off_dir` field
- [x] Update `parse()` in `knot_file.rs`: remove `tie-off-dir` parsing/validation
- [x] Update `RawFrontmatter`: remove `tie_off_dir` optional field
- [x] Update all domain layer tests

**Rationale:** Domain is the core — change entities and events first, everything else follows.

### Phase 1: Application — Static Path Derivation, Timestamped Logging

Hex layer: **Application**

- [ ] Add `derive_tieoff_path(loom_id: &LoomId, knot_id: &KnotId, rig: &Path) -> PathBuf` helper in `usecases.rs`
  - Returns `rig/output/{loom-id}/{knot-name}/output.md`
- [ ] Add `derive_loom_log_path(loom_id: &LoomId, rig: &Path) -> PathBuf` helper
  - Returns `rig/output/{loom-id}/.loom-log`
- [ ] Update `ProcessStrand::compute_tie_off_path()` to use `derive_tieoff_path` instead of `knot.tie_off_dir`
- [ ] Add `format_timestamp() -> String` helper for ISO 8601 UTC timestamps
- [ ] Update all `LoomEvent` construction sites in usecases to include timestamp
- [ ] Update application layer unit tests

### Phase 2: Outbound Adapters — Loom Log Path, Console Log Timestamps

Hex layer: **Outbound Adapters**

- [ ] Update `FileSystemLoomLog::log_path()` to use `rig/output/{loom-id}/.loom-log`
- [ ] Update `FileSystemLoomLog::open_file()` to create `rig/output/{loom-id}/` directory
- [ ] Update `SharedLoomLog::open_file()` similarly
- [ ] Update `adapters/logging.rs`: prepend ISO 8601 timestamp to all `eprintln!` calls
- [ ] Update outbound adapter unit tests
- [ ] Update `loom_repository.rs`: remove `tie_off_dir` from `Knot` construction, remove `tie_off_dir` from `resolve_path` loop

### Phase 3: Integration Tests and Composition Root

Hex layer: **Integration**

- [ ] Update `server.rs` `build_app_context()`: no path changes needed (uses `base_dir` which is still `rig`)
- [ ] Update integration tests that check output paths:
  - Tie-off files now at `rig/output/{loom-id}/{knot-name}/output.md`
  - Loom-log at `rig/output/{loom-id}/.loom-log`
- [ ] Update integration tests that create knot YAML files: remove `tie-off-dir` from test fixtures
- [ ] Update integration tests that check loom-log content: verify timestamp fields
- [ ] Verify full test suite passes

### Phase 4: Documentation Updates

- [ ] Update `project/domain-glossary.md`:
  - **Tie-off Directory**: Change to "Statically derived path under `rig/output/{loom-id}/{knot-name}/`."
  - **Loom-log**: Change path to `<rig>/output/<loom-id>/.loom-log`
  - **Tie-off**: Mention single `output.md` per knot
- [ ] Update PRD `prd-ai-driven-file-generation.md` glossary section to match
- [ ] Update term relationships diagram

## Notes

- This is a **breaking change** for existing knot definitions that include `tie-off-dir`. The parser will ignore it (once the field is removed from `RawFrontmatter`). Users will need to remove `tie-off-dir` from their knot YAML files.
- The `strand-dir` field remains required and configurable because strands live in project source directories (e.g., `project/prds/`), which genuinely varies per use case.
- The `tie_off_sink` in `server.rs` is constructed with `base_dir` (the rig path). The sink's internal `tie_off_dir` is only used by `resolve_path()` which is not called during processing. The actual path comes from the usecase. After this change, the sink's constructor arg is still useful (it provides the rig root for `derive_tieoff_path`), so the constructor signature can stay the same.
