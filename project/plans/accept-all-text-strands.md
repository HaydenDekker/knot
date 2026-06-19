# Plan: Accept All Text Files as Strands

## Related PRD

This plan contributes to [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md).

This plan extends the strand input model so knots can operate on any text file — source code, config, plain text — not just `.md`. This matches the PRD's intent that Knot triggers agent workflows on project files.

## Problem

Knot currently only watches `.md` files in strand directories. The filter in `NotifyEventSource::map_strand_event()` silently drops all non-`.md` files. A user might strand a directory of source files (`.py`, `.rs`, `.js`), config files (`.json`, `.yaml`), or plain text (`.txt`) — and none of them trigger processing.

Knot is not expected to handle binary formats (images, PDFs, archives). Users strand text-based project files. The `.md`-only filter is an oversight, not a design decision.

## Target

Knot accepts **all text files** as strands. Binary/non-text files are silently skipped with a warning written to both the loom-log (`LoomEvent::StrandIgnored`) and stderr (system log at warn level).

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `event_source.rs` unit tests | Notify event mapping (create/modify/delete for `.md` files, directory filtering) | ✅ Green — 14 tests covering current `.md`-only behaviour |
| `tests/discovery.rs` | Full rig discovery + loom registration + watcher lifecycle | ✅ Green |
| `tests/composition.rs` | Composition root, event pipeline wiring | ✅ Green |
| `tests/pipeline.rs` | End-to-end strand processing | ✅ Green |
| `tests/auto_discovery_and_knot_crud.rs` | Auto-discovery + knot CRUD | ✅ Green |
| `tests/tie_off.rs` | Tie-off write lifecycle | ✅ Green |
| `tests/rig_log.rs` | Rig-log operational events | ✅ Green |
| `debounce.rs` unit tests | Debounce engine + InspectQueue | ✅ Green |

## Test Gaps

- No test for non-`.md` files being ignored (because it's an oversight, not intentional)
- No test for binary file handling
- No integration test verifying loom-log warnings for ignored files

## Phases

### Phase 0: Text Detection Utility and Domain Event
- [x] Add `content_inspector` crate to `Cargo.toml` (probes first bytes for null bytes)
- [x] Add `is_text_file(path: &Path) -> Result<bool, PortError>` as a pure utility in `domain/` layer (or `adapters/outbound/` since it reads files — this is an IO operation so it's an adapter concern)
- [x] Add `LoomEvent::StrandIgnored` variant with fields: `loom_id`, `knot_id`, `strand_path`, `reason`, `timestamp`
- [x] Add domain unit tests for the new `LoomEvent` variant (JSON round-trip)
- [x] **Compile + tests pass**

### Phase 1: Remove `.md` Extension Filter from Event Source
- [x] In `event_source.rs::map_strand_event()`, remove the `.md` extension check — accept all files
- [x] Update existing `event_source.rs` unit tests: rename/refactor `.md`-specific tests to be extension-agnostic
- [x] Add new `event_source.rs` unit tests:
  - `.txt` file create → StrandEvent::Created emitted
  - `.rs` file modify → StrandEvent::Modified emitted
  - Binary file create → StrandEvent::Created emitted (filtering happens downstream)
- [x] **Compile + tests pass**

### Phase 2: Text Check in ProcessStrand
- [x] In `ProcessStrand::execute()`, after looking up the loom/knot and before agent execution:
  - For `Created`/`Modified` events: call `is_text_file(&strand_path)`
  - If binary: write `LoomEvent::StrandIgnored` to loom-log, write `eprintln!` warn to stderr, return `Ok(())` (skip agent)
  - For `Deleted` events: skip text check (file is gone), process normally
- [x] Add `usecases.rs` unit tests using `MockLoomLogPort` for:
  - Binary file → StrandIgnored in loom-log, no agent execution
  - Text file → normal processing path (regression guard)
  - Deleted event → normal processing path (no text check needed)
- [x] **Compile + tests pass**

### Phase 3: Integration Tests
- [x] Add integration test in `tests/pipeline.rs`:
  - Create a binary file (with null bytes) in a strand directory
  - Verify: no tie-off produced, `StrandIgnored` appears in loom-log, stderr contains warning
  - Create a `.txt` text file in the same strand directory
  - Verify: normal processing (tie-off produced, `KnotCompleted` in loom-log)
- [x] Add integration test:
  - Create a `.rs` source file
  - Verify: normal processing (tie-off produced, `KnotCompleted` in loom-log, no `StrandIgnored`)
- [x] **Compile + all tests pass**

## Notes

- `content_inspector::inspect()` reads the first 8096 bytes and checks for null (`0x00`) bytes. This is the standard heuristic used by Unix `file` command and is reliable for distinguishing text from binary.
- The text check happens at **processing time** (in `ProcessStrand`), not at the event source level. This keeps the adapter layer simple (just remove the `.md` filter) and puts the business logic where it belongs (application layer).
- For `Deleted` events, the file no longer exists so we can't probe it. We process deletions normally since the tie-off cleanup is still relevant.
