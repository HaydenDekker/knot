# Plan: Consolidate `process_strand.rs` Test Modules

## Problem

`process_strand.rs` is 3,862 lines — the largest source file in the project by a wide margin. Production code is ~500 lines, but ~3,358 lines of tests are inlined across 7 test modules. The file is unwieldy to navigate and contains significant duplication:

- `build_knot()` is defined **5 times** across test modules
- `build_loom()` is defined **3 times**
- `build_process_strand()` is defined **2 times**
- `MockAgentRunner` is redefined locally in `phase3_profile_resolution_tests` despite already existing in `test_fixtures.rs`
- `TrackingAgentRunner` is defined separately in both `phase7` and `phase9`
- `MockLoomRepository` is redefined in `phase4_tests` despite existing in `test_fixtures.rs`
- `TrackingTieOffSink` duplicates `MockTieOffSink` just to add append-tracking
- `phase3_tests` and `phase4_tests` are empty stubs with zero tests
- Module names (`phase3_tests`, `phase4_tests`, etc.) are phase-numbered remnants from old implementation phases — they say nothing about what they test

## Target

After this refactor:

- Test modules are renamed to describe what they test (not phase numbers)
- Empty dead-code stubs are removed
- Shared mocks and helpers are consolidated into `test_fixtures.rs`
- Duplicate local definitions are removed in favour of fixture imports
- Tests remain inline in `process_strand.rs` following Rust convention — unit tests stay with the code they test

## Implementation Status: ✅ Complete

### Phase 0: Baseline Verification (Done 2026-06-29)
- **File:** `process_strand.rs` — 3,862 lines
- **Tests:** 38 passing, 0 failing
- **9 test modules** total (2 dead-code stubs: `phase3_tests`, `phase4_tests`)
- **Duplication confirmed:** `build_knot()` ×6, `build_loom()` ×5, `build_process_strand()` ×4, local `MockAgentRunner` ×5, `TrackingTieOffSink` ×3, `TrackingAgentRunner` ×2, dead `MockLoomRepository` ×1
- **Existing fixtures** already provide: `build_knot(id)`, `build_knot_with_strand_dir`, `build_loom`, `MockAgentRunner`, `MockTieOffSink`, `MockLoomRepository`, `MockLoomLogPort`, `MockRigLogPort`, `MockProfileRepository`, `MockGitVersioningPort`, `MockStrandFileChecker`, `TrackingEventSource`

## Existing Tests

| Test Location | What it covers | Status |
|---------------|---------------|--------|
| `process_strand.rs` — `phase3_tests` | Empty stub (no tests) | ⚠️ Dead code |
| `process_strand.rs` — `phase4_tests` | Empty stub + local `MockLoomRepository` (no tests) | ⚠️ Dead code |
| `process_strand.rs` — `phase3_profile_resolution_tests` | Profile resolution (resolve_agent_config, profile not found, multiple knots, dynamic pickup, no --system-prompt) — 5 tests | ✅ Green |
| `process_strand.rs` — `phase6_timeout_tests` | Timeout handling, non-timeout error, success path, deleted events, deleted history, session resume, session title — 9 tests | ✅ Green |
| `process_strand.rs` — `phase7_timeout_resolution_tests` | Profile timeout resolution (timeout from profile, none timeout, execute passes timeout) — 4 tests | ✅ Green |
| `process_strand.rs` — `phase8_git_versioning_tests` | Git versioning (commit on success, skip when disabled, continue on error) — 3 tests | ✅ Green |
| `process_strand.rs` — `phase9_session_title_tests` | Session title (--name flag, title formats) — 7+ tests | ✅ Green |
| `test_fixtures.rs` | Shared mocks (MockAgentRunner, MockLoomLogPort, MockTieOffSink, etc.) and builders (build_knot, build_loom) | ✅ Used across usecases |

## Test Gaps

None — this is a pure structural refactor. All existing tests are preserved and verified green after each phase. The goal is zero behaviour change.

## Phases

### Phase 1: Remove dead-code test stubs ✅
- [x] Remove `phase3_tests` — empty module with zero tests
- [x] Remove `phase4_tests` — empty module (plus its locally-defined `MockLoomRepository` which is dead code)
- [x] Verify: `cargo test process_strand` — 38 tests pass (unchanged)
- File reduced from 3,862 → 3,794 lines (−68 lines)
- [ ] Remove `phase3_tests` — empty module with zero tests
- [ ] Remove `phase4_tests` — empty module (plus its locally-defined `MockLoomRepository` which is dead code)
- [ ] Verify: `cargo test` — same test count

### Phase 2: Rename test modules by concern ✅
- [x] Rename `phase3_profile_resolution_tests` → `profile_resolution_tests`
- [x] Rename `phase6_timeout_tests` → `execution_tests`
- [x] Rename `phase7_timeout_resolution_tests` → `profile_timeout_tests`
- [x] Rename `phase8_git_versioning_tests` → `git_versioning_tests`
- [x] Rename `phase9_session_title_tests` → `session_title_tests`
- [x] Rename `phase2_text_check_tests` → `text_check_tests`
- [x] Rename `phase2_file_existence_tests` → `file_existence_tests`
- [x] Update cross-reference comment in `pi_stdio.rs`
- [x] Verify: `cargo test process_strand` — 38 tests pass (unchanged)

### Phase 3: Consolidate shared helpers into `test_fixtures.rs` ✅
- [x] Add `TrackingTieOffSink` (with append-tracking + content storage) to `test_fixtures.rs`
- [x] Add `TrackingAgentRunner` (captures ExecutionContext) to `test_fixtures.rs`
- [x] Add `build_knot_with_profile(id, profile)` variant to fixtures
- [x] Add `default_profile()` helper to fixtures
- [x] Remove locally-defined mocks that already exist in `test_fixtures.rs`:
  - local `MockAgentRunner` from `profile_resolution_tests`
  - local `TrackingAgentRunner` from `profile_timeout_tests` and `session_title_tests`
  - local `MockAgentRunner`, `MockTieOffSink`, `MockRigLogPort`, `MockGitVersioningPort`, `MockProfileRepository`, `MockLoomLogPort` from `git_versioning_tests`
  - local `TrackingTieOffSink`, `MockAgentRunner` etc. from `text_check_tests` and `file_existence_tests`
- [x] Remove duplicate `build_knot`, `build_loom`, `build_process_strand` definitions from individual test modules — replaced with imports from `test_fixtures`
- [x] `execution_test_shared` now re-exports from `test_fixtures` instead of defining its own copies
- [x] Verify: `cargo test process_strand` — 38 tests pass (unchanged)

### Phase 4: Split execution sub-concerns into focused modules ✅
- [x] Extract shared infrastructure into `execution_test_shared`:
  - `TrackingTieOffSink`, `build_knot`, `build_process_strand`, `default_profile`
- [x] Split `execution_tests` into focused sub-modules:
  - `execution_tests` — happy path and error handling (timeout, non-timeout error, success)
  - `execution_deleted_tests` — deleted event tests (no @file, deletion notice, strand history, no history)
  - `session_resume_tests` — retry tests (transparent success, exhausted, no-retry stdio)
- [x] Verify: `cargo test process_strand` — 38 tests pass (unchanged)

### Phase 5: Cleanup and verify ✅
- [x] Run `cargo clippy -- -D warnings` — no new warnings (37 pre-existing warnings in other files)
- [x] Run `cargo test` — 461 tests pass (unchanged)
- [x] Run `cargo test -- --test-threads=1` — 38 process_strand tests pass, no ordering dependencies

## Notes
- This is a pure structural refactor — no behaviour change, no API change, no version bump needed
- Tests remain inline in `process_strand.rs` following Rust convention (unit tests stay with the code they test)
- The existing `usecases-refactor.md` (#48) plan split `usecases.rs` into isolated modules. This plan cleans up the test hygiene within the largest resulting module.
