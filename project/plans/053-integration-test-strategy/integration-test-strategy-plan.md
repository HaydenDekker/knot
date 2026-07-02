# Plan: Integration Test Strategy

## Problem

The integration test suite has three interconnected problems that make it unreliable and slow:

### 1. Mock agent identity race (process-global PATH/env vars)

`tests/helpers.rs` mock creation helpers (`create_mock_pi`, `create_mock_agent`) set `PATH` or `KNOT_TEST_CLI_PATH` via `std::env::set_var()`, which is process-global. When tests run in parallel (`--test-threads > 1`, the default), one test's PATH modification overwrites another's.

This manifests as:
- Tests picking up the wrong mock binary → unexpected behaviour (success instead of failure, hangs, wrong output)
- Flaky failures that pass under `--test-threads=1`
- `TEST_MUTEX` serialisation in 11 test files as a workaround

### 2. Test suite too slow (~205s total)

| Test | Current duration | Why |
|------|-----------------|-----|
| `process_strand_retry_exhausted_fails` (unit) | 60s+ | 10 retries × 10s delay = 100s worst case |
| `test_session_resume_success` | 60s+ timeout | Mock agent hangs (wrong mock picked up) |
| Multiple pipeline/agent tests | 5-15s each | Full Knot lifecycle per test |

### 3. Composition-heavy tests

Most integration tests spin up the full Knot runtime (notify watching, debounce, state writer, subprocess agent) to test application logic. This conflates two concerns: testing business logic against trait contracts, and testing that adapters wire correctly.

### Current state (2026-07-02)

**475 passed, 1 failed (unit), ~158s total (integration not fully run):**

| Suite | Passed | Failed | Duration |
|-------|--------|--------|----------|
| Unit tests (`lib.rs`) | 475 | **1** | 100.15s |
| `agent_integration` | 25 | 0 | 45s |
| `pipeline` | 30 | 0 | 112s |
| Other suites | ~60 | 0 | ~10s |

The one failing unit test (`runner_passes_event_metadata`) is a mock path collision: two parallel unit tests both write to `/tmp/knot-test-mock-stdio`.

## Target

After this plan:
- **~100 tests total, all fully parallel, no `TEST_MUTEX`**
- **Full test suite under 30s**
- **Three test tiers** — application (mock ports), adapter (real I/O + `tempfile`), smoke (full composition + mock agent via `cli_path`)
- **No process-global env manipulation** — mock helpers return paths, never call `std::env::set_var()`
- **ADR-011** documents the strategy

## Design

See [ADR-011: Hexagonal Test Strategy](../adrs/adr-011-hexagonal-test-strategy.md).

### Test Tiers

| Tier | Count | Strategy | Duration |
|------|-------|----------|----------|
| Application | ~90 | All ports mocked (`TrackingTieOffSink`, `TrackingAgentRunner` etc.) | ~0ms per test |
| Adapter | 8 | One per adapter, real I/O + `tempfile::tempdir()` | ~1s per test |
| Smoke | 2 | Full composition, mock agent via `cli_path` injection | ~5s per test |

## Phases

### Phase 0: Smoke tests and composition wiring

**Goal:** Establish the smoke tests first so they always pass as we strip back integration tests.

- Add `cli_path: Option<PathBuf>` to `AppConfig` and wire it into `build_app_context()`
- Add `with_cli_path()` method to `AppConfig`
- Create `tests/smoke.rs` with two tests: `composition_smoke_stdio` and `composition_smoke_json`
- Each uses `tempfile::tempdir()` + mock agent script + `start_knot()` with `cli_path`
- Verify: both smoke tests pass under parallel execution

### Phase 1: Unit test mock path isolation

**Goal:** Fix the remaining unit test failures from shared `/tmp` paths.

- `make_mock_path()` in `src/adapters/pi_stdio.rs` uses `tempfile::tempdir().join("mock-pi")`
- `make_blocking_mock_path()` same pattern
- `make_json_mock_path()` in `src/adapters/pi_json.rs` same pattern
- `make_json_blocking_mock_path()` same pattern
- Test function keeps `tempdir` handle alive (in scope or stored in runner)
- Verify: `cargo test --lib` passes under default parallel execution

### Phase 2: Adapter test extraction

**Goal:** One test per outbound adapter, verifying the I/O contract in isolation.

- Create `tests/adapters.rs` (or `tests/adapter_<name>.rs` modules)
- `PiStdioAgentRunner` test: subprocess spawn, stdin/stdout capture, exit code, timeout
- `PiJsonAgentRunner` test: JSON-L parsing, session ID extraction, `stopReason` filtering
- `FileSystemTieOffSink` test: write, append, read_content, directory creation
- `FileSystemLoomLog` test: open, JSONL append, read_all
- `FileSystemStateWriter` test: atomic write (`.tmp` + `rename`), valid JSON
- `FileSystemLoomRepository` test: scan rig, parse knot files, save, parse warnings
- `FileSystemAgentProfileRepository` test: profile CRUD from `.md` files
- `NotifyEventSource` test: watch/unwatch, event emission on file changes
- Each adapter test uses `tempfile::tempdir()`, unique mock paths, fully parallel

### Phase 3: Application test migration — core use cases

**Goal:** Migrate `process_strand` integration tests from `start_knot()` to mock-port tests.

- `tests/agent_integration.rs` → rewrite against `TrackingTieOffSink`, `TrackingAgentRunner`, `TrackingLoomLog`
- Remove `start_knot()` calls, `create_mock_pi()` PATH manipulation, `TEST_MUTEX`/`acquire_test_lock()`
- Each test: construct `ProcessStrand` with mocks, call `execute()`, assert on mock state
- Files removed: `tests/agent_integration.rs` (replaced), `acquire_test_lock()` removed
- Verify: all previously-covered scenarios pass, faster execution

### Phase 4: Application test migration — pipeline and file-based tests

**Goal:** Migrate `pipeline.rs` and remaining file-based integration tests.

- `tests/pipeline.rs` → debounce + process flow tests against mock ports
- `tests/tie_off.rs` → tie-off append/read tests against `TrackingTieOffSink`
- `tests/rig_log.rs` → rig-log append tests against `TrackingRigLog`
- `tests/git_versioning.rs` → git commit tests against `TrackingGitVersioningPort`
- Remove `start_knot()` calls, `TEST_MUTEX`, `set_var("PATH")`
- Verify: all scenarios pass, no regressions vs adapter tests

### Phase 5: Application test migration — session resume and edge cases

**Goal:** Migrate session resume and remaining edge case tests.

- `tests/session_resume.rs` → retry loop tests against `TrackingAgentRunner` that fails then succeeds
- Session resume application tests: verify retry count, delay, `--session-id` injection, exhaustion
- Session resume adapter test: verify `PiStdioAgentRunner` surfaces `session_id` in errors
- `tests/profile_timeout.rs` → timeout enforcement against `TrackingAgentRunner`
- `tests/multi_loom.rs` → multi-loom independence (this remains a composition test with 2 looms + mock agents)
- Remove `TEST_MUTEX`, `acquire_test_lock()`
- Verify: session resume tests <5s each

### Phase 6: Composition root and helper cleanup

**Goal:** Remove dead code and consolidate remaining helpers.

- `AppConfig` gains `cli_path` field (used by smoke tests)
- `start_knot()` simplified to smoke-test helper only (no debounce env var overrides needed if smoke tests use fast debounce)
- Remove `TEST_MUTEX` / `acquire_test_lock()` from all remaining files
- Remove `create_mock_pi()`, `create_mock_pi_capturing_stdin()` (replaced by adapter test helpers)
- Remove `derive_tie_off_file()` from `tests/helpers.rs` (moved to domain or removed)
- Remove `wait_for_state_field()`, `wait_for_knot_status_in_state()` etc. (used only by smoke tests, inline them)
- Verify: `cargo test` passes, all files clean

### Phase 7: Verify and record

**Goal:** Full suite verification and documentation.

- Run `cargo test` — target: 0 failures, <30s total
- Run `cargo test --test-threads=4` — verify identical results
- Run `cargo clippy` — verify no new warnings
- Verify `master-plan.md` updated, plan marked complete
- Record final test suite duration and test counts

### Phase 8: Application test migration — auto-discovery and CRUD

**Goal:** Remove `auto_discovery_and_knot_crud.rs` (9 composition tests) whose scenarios are fully covered by ConfigEventHandler unit tests + NotifyEventSource adapter tests.

The ConfigEventHandler already has 17 unit tests covering: `config_handler_loom_added`, `config_handler_loom_added_already_registered`, `config_handler_knot_added`, `config_handler_knot_added_duplicate`, `config_handler_knot_added_loom_not_found`, `config_handler_knot_modified`, `config_handler_knot_modified_same_strand_dir`, `config_handler_knot_modified_not_found`, `config_handler_knot_modified_new_knot_registers`, `config_handler_knot_modified_warns_on_recovery`, `config_handler_knot_deleted`, `config_handler_knot_deleted_not_found`, `config_handler_knot_deleted_loom_not_found`, `config_handler_knot_added_missing_strand_dir`, `config_handler_knot_modified_missing_strand_dir`, `config_handler_loom_added_scans_specific_dir`, `config_handler_loom_added_dir_missing`.

The NotifyEventSource adapter tests in `adapters.rs` cover: `loom_dir_new_knot_emits_config_event`, `loom_dir_edit_knot_emits_config_event`, `loom_dir_delete_knot_emits_config_event`, `rig_dir_new_loom_emits_config_event`.

Per ADR-011: "if every adapter satisfies its trait contract (Tier 2), and every use case works against mocks (Tier 1), the smoke test (Tier 3) only needs to prove the happy-path wiring."

- Remove `tests/auto_discovery_and_knot_crud.rs`
- Verify: no uncovered scenarios (all 9 tests map to existing unit/adapter tests)
- Verify: `cargo test` passes, 0 regressions

### Phase 9: Application test migration — startup, lifecycle and skill integration

**Goal:** Remove remaining composition tests whose scenarios are fully covered by existing tiers.

**`tests/discovery.rs`** (4 tests → 3):
- Keep 3 adapter tests: `discovers_looms_at_startup`, `ignores_non_loom_directories`, `discovers_multiple_looms` — these use `build_app_context()` + `run_startup()` directly (adapter-level, no `start_knot()`)
- Remove 1 composition test: `writes_discovered_looms_to_state_file` — uses `start_knot()` + `wait_for_state_file()`. State.json writing is covered by smoke tests + `write_state` unit tests.

**`tests/rig_lifecycle.rs`** (5 tests → removed):
- `rig_directory_auto_created` — covered by smoke test (rig created at startup)
- `looms_scanned_on_startup` — covered by `discovery.rs` adapter tests + smoke test
- `empty_rig_produces_valid_state` — covered by `build_state_empty_rig` unit test + smoke test
- `profiles_loaded_into_state` — covered by smoke test
- `state_file_has_required_schema` — covered by `build_state_*` unit tests (10 tests)

**`tests/skill_integration.rs`** (10 tests → removed):
- 5 state schema tests (`state_file_has_rig_path`, `state_file_looms_schema`, `state_file_profiles_schema`, `state_file_knot_has_status`, `state_file_updated_at_is_timestamp`) — covered by `write_state` unit tests
- 3 file convention tests (`loom_directory_naming_convention`, `knot_file_naming_convention`, `profile_file_naming_convention`) — trivial assertions, no runtime needed
- 1 tie-off path test (`tie_off_path_convention`) — covered by smoke test (tie-off exists at correct path)
- 1 loom-log path test (`loom_log_path_convention`) — covered by smoke test + `loom_log` adapter tests

**`tests/shutdown.rs`** (2 tests → removed):
- `shutdown_writes_loom_stopped` — verifies LoomStarted written + clean abort. Covered by smoke test (full pipeline completes, loom-log has events).
- `shutdown_drains_pipeline_before_loom_stopped` — verifies processing completes before abort. Covered by smoke test (waits for KnotCompleted before teardown).

- Verify: `cargo test` passes, 0 regressions

### Phase 10: Remove adapter integration tests and dead helpers

**Goal:** Remove `adapter_integration.rs` (already superseded per plan notes) and clean up dead code.

**`tests/adapter_integration.rs`** (3 tests → removed):
- `test_json_invocation_full_pipeline` — covered by `smoke.rs` `composition_smoke_json` + `adapters.rs` PiJson adapter tests
- `test_stdio_invocation_full_pipeline` — covered by `smoke.rs` `composition_smoke_stdio` + `adapters.rs` PiStdio adapter tests
- `test_json_invocation_timeout_captures_session_id` — covered by `adapters.rs` PiJson adapter test `timeout_enforcement_returns_timeout`

**Dead helpers in `tests/helpers.rs`:**
- `start_knot(rig_dir: PathBuf)` — only used by removed files (smoke/multi_loom use `start_knot_with_config`)
- `init_git_repo()`, `get_latest_commit()`, `count_commits()`, `run_git()` — dead code since git_versioning.rs uses mock ports
- `build_profile_with_timeout()` — dead code (session_resume.rs has its own)
- `wait_for_loom_log_event_with_deadline()` — only used by `adapter_integration.rs`

**Helpers to KEEP (used by remaining files):**
- `start_knot_with_config()` — used by `smoke.rs`, `multi_loom.rs`
- `create_loom_dir()`, `create_knot_file()`, `create_fast_profile()`, `create_strand()` — used by `smoke.rs`, `multi_loom.rs`, `discovery.rs`
- `wait_for_loom_in_state()`, `wait_for_knot_status_in_state()`, `wait_for_state_file()` — used by `smoke.rs`, `multi_loom.rs`
- `read_loom_log()`, `loom_log_event_type()` — used by `smoke.rs`, `multi_loom.rs`

- Verify: `cargo test` passes, `cargo clippy` clean

### Phase 11: Final verification

**Goal:** Confirm suite meets ADR-011 targets.

- Run `cargo test` — target: 0 failures, <30s wall clock
- Run `cargo test -- --test-threads=4` — verify identical results
- Run `cargo clippy` — verify no new warnings
- Verify remaining test structure:
  - Tier 1: ~476 unit tests (lib.rs) + ~50 application tests (mock ports)
  - Tier 2: 33 adapter tests (`adapters.rs`)
  - Tier 3: 2 smoke tests (`smoke.rs`) + 2 multi-loom composition tests (`multi_loom.rs`)
  - Adapter-level: 3 discovery tests (`discovery.rs`), 6 composition wiring tests (`composition.rs`)
  - Total: ~570 tests (down from 746, removed 177 composition tests)
- Record final test suite duration and test counts

## Notes

- Phase 0 (smoke tests) must be done first — they prove composition works and prevent us from breaking wiring during the strip-back.
- Phases 3-5 can be done in any order — each migrates a set of integration tests independently.
- `tests/adapter_integration.rs` is replaced by Phase 2's adapter tests and Phase 0's smoke tests (removed in Phase 10).
- `tests/helpers.rs` is heavily reduced by Phase 6 — most helpers become unused. Final cleanup in Phase 10.
- Phases 8-11 remove remaining composition tests superseded by the three-tier coverage. Per ADR-011: "if every adapter satisfies its trait contract (Tier 2), and every use case works against mocks (Tier 1), the smoke test (Tier 3) only needs to prove the happy-path wiring."
- ADR-011 documents the strategy; this plan tracks the migration.

## Implementation Status: ✅ Complete (2026-07-03)

## Notes
- Phases 0-10: Hexagonal test architecture implemented, composition tests stripped, helper dead-code removed.
- Phase 11: Final verification — 626 tests pass (0 failures), ~1.5s lib wall clock, ~24.6s full suite.
- Post-phase bugfix: flaky `execute_timeout_regression_no_context_override` under `--test-threads=4` (ETXTBSY from `std::fs::write` on exec'd mock binary) fixed with atomic write-to-temp + `rename()`.
- Test count: 626 total (down from 746). Structure matches ADR-011 tiers.

## Implementation Status: ✅ Complete (2026-07-03)

## Notes
- Phases 0-10: Hexagonal test architecture implemented, composition tests stripped, helper dead-code removed.
- Phase 11: Final verification — 626 tests pass (0 failures), ~1.5s lib wall clock, ~24.6s full suite.
- Post-phase bugfix: flaky `execute_timeout_regression_no_context_override` under `--test-threads=4` (ETXTBSY from `std::fs::write` on exec'd mock binary) fixed with atomic write-to-temp + `rename()`.
- Test count: 626 total (down from 746). Structure matches ADR-011 tiers.
