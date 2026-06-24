# Plan: Strand Missing File Handling

## Problem

When `sed -i` (macOS) or similar editors modify files in-place, they create temporary files (e.g. `sedXXXXXXX`) that trigger `notify` filesystem events. By the time Knot debounces and attempts to process the event, the temp file has already been renamed into its final name. The agent (`pi`) then fails with `"File not found: /path/to/sedXXXXXXX"`, producing a `StrandProcessed` event with an error in the loom-log.

This is expected noise — the temp file was never a real strand. But currently every missing file (temp or otherwise) produces the same error path: full agent invocation attempt, loom-log `KnotFailed` + `StrandProcessed(error)`, and a tie-off error entry.

## Target

- File existence is checked **before** agent invocation
- Known temp file patterns (initially `sed`) are silently skipped — no loom-log entry, no agent invocation
- Unknown missing files produce a new `LoomEvent::StrandSkipped` loom-log entry + console warning — the user can investigate if it's a real issue
- Deleted events skip the check (file is expected to be gone)

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `src/application/usecases.rs` — `ProcessStrand` tests | Timeout, non-timeout error, success, deleted events, git versioning, binary file handling | ✅ Green — 366+ tests |
| `src/adapters/outbound/loom_log.rs` tests | Loom-log append, path routing, JSONL format | ✅ Green |
| `tests/pipeline.rs` | End-to-end processing pipeline | ✅ Green |

## Test Gaps

- No test for file existence check before agent invocation
- No test for temp file pattern detection
- No test for unknown missing file handling (StrandSkipped event)
- No integration test for the temp file scenario

## Phases

### Phase 0: Domain — Temp File Pattern Detection

- [ ] Create `src/domain/temp_file.rs` — pure domain, zero IO
  - `fn is_known_temp_file(path: &Path) -> bool` — checks filename against known patterns
  - Initial pattern: `sed` followed by exactly 7 characters (macOS `sed -i` temp files)
  - Filename-based check (not path-based) — temp files can appear in any strand directory
- [ ] Unit tests: known sed pattern matches, non-sed files don't match, path components ignored (only filename checked)

### Phase 1: Domain — StrandSkipped Event Variant

- [ ] Add `LoomEvent::StrandSkipped` variant to `src/domain/events.rs`:
  - Fields: `loom_id`, `knot_id`, `strand_path`, `reason`, `timestamp`
  - Reason is `"missing file (unknown pattern)"` — distinguishes from binary ignore
- [ ] Update `src/adapters/outbound/loom_log.rs` — `extract_loom_id()` handles new variant
- [ ] Unit tests: serialisation round-trip, event fields, loom-log routing

### Phase 2: Application — File Existence Check in ProcessStrand

- [ ] In `ProcessStrand::execute()`, add file existence check **after** text-file check but **before** agent invocation (before step 5)
- [ ] Only applies to `Created` and `Modified` events (`Deleted` already expects file to be gone)
- [ ] If file doesn't exist:
  - Check `temp_file::is_known_temp_file(&strand_path)`
  - **Known temp file**: skip silently (debug-level console log only). No loom-log entry, no agent invocation, return `Ok(())`
  - **Unknown missing file**: log `LoomEvent::StrandSkipped` to loom-log + `eprintln!` console warning. No agent invocation, no tie-off write, return `Ok(())`
- [ ] Unit tests with `MockAgentRunner` (never called) and `MockLoomLogPort`:
  - Known temp file: no log events, no agent call, returns Ok
  - Unknown missing file: StrandSkipped in loom-log, no agent call, returns Ok
  - Existing file: passes through to normal processing path (regression guard)
  - Deleted events: no existence check applied (regression guard)

### Phase 3: Integration Test

- [ ] Add test in `tests/pipeline.rs` (or new integration test file):
  - Create a temp file with `sedXXXXXXX` name in strand directory
  - Immediately delete it (simulating `sed -i` rename)
  - Verify: no agent invocation, no loom-log error entries, no tie-off written
- [ ] Add test for unknown missing file:
  - Trigger event for a file that doesn't exist and doesn't match temp patterns
  - Verify: `StrandSkipped` in loom-log, no agent invocation

## Notes

### Why not rig-log?

The system reliability PRD (Story 6) restricts rig-log to high-signal operational events only (timeout, queue idle). Temp file noise is not operational — it's filesystem implementation detail. Unknown missing files are loom-log level (user can check if concerned). Nothing here warrants rig-log.

### Why not extend the debounce window?

Widening the debounce window is a blunt instrument — it delays all processing and doesn't guarantee the file will exist (race window still exists). The existence check is the correct place to handle it: after debouncing, before invocation.

### Extensibility

The temp file pattern list is in domain layer and can grow (e.g. `vim` swap files, `nano` temp files) without touching application or adapter layers.
