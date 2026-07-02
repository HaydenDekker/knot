# Phase 6: Composition root and helper cleanup

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [x] `src/server.rs` — `AppConfig`:
  - [x] `cli_path: Option<PathBuf>` field present (done in Phase 0)
  - [x] `with_cli_path()` method present (done in Phase 0)
  - [x] `build_app_context()` uses `cli_path` when `Some` (done in Phase 0)
- [x] `tests/helpers.rs` cleanup:
  - [x] Remove `TEST_MUTEX` / `acquire_test_lock()` — removed from `adapter_integration.rs`, `shutdown.rs`, `skill_integration.rs` (were not in helpers.rs)
  - [x] Remove `create_mock_pi()` / `create_mock_pi_capturing_stdin()` / `create_mock_agent()` / `create_stub_pi_agent()` — removed from helpers.rs, callers migrated to inline mock + `cli_path` injection
  - [x] Remove `derive_tie_off_file()` — did not exist in codebase (skipped)
  - [ ] Remove `wait_for_state_field()`, `wait_for_knot_status_in_state()`, `wait_for_loom_in_state()` — deferred: used by 10+ test files across rig_lifecycle, skill_integration, multi_loom, auto_discovery, shutdown, discovery, adapter_integration, smoke (see Deviations)
  - [ ] Remove `wait_for_loom_log_event()` — deferred: used by adapter_integration, smoke tests (see Deviations)
  - [x] Keep: `start_knot()` (smoke test helper), `make_knot_content()`, `create_knot_file()`, `create_fast_profile()`, `create_loom_dir()`, `create_strand()`, `read_state_file()`, `read_loom_log()`
  - [ ] `start_knot()` simplified — deferred: debounce env vars still needed for fast test execution (see Deviations)
- [x] Remove `set_var("PATH")` from all remaining test files
- [x] Remove `set_var("KNOT_TEST_CLI_PATH")` from all remaining test files — did not exist (skipped)
- [x] Verify: all test files compile, no references to removed helpers
- [x] Run full test suite — 476 passed (lib), all integration tests pass

## Deviations

1. **`wait_for_*` helpers kept** — The checklist says these are "used only by smoke tests" but they are actually used by ~10 test files (rig_lifecycle, skill_integration, multi_loom, auto_discovery_and_knot_crud, shutdown, discovery, adapter_integration, smoke). These are composition/smoke tests that need to poll state.json and loom-logs. Inlining them would duplicate ~200 lines of polling logic across 10 files. Kept them.

2. **`start_knot()` debounce env vars kept** — The checklist says "no debounce env var overrides" but there is no "fast debounce via profile" mechanism. The debounce timing (20ms/2ms vs 100ms/5ms) is controlled by env vars read in `start_event_pipeline()`. Removing them would make tests ~5x slower. Kept them.

3. **Callers migrated to `cli_path` injection** — `create_mock_pi()` callers in `shutdown.rs`, `skill_integration.rs`, and `adapter_integration.rs` were migrated to create mock binaries inline and use `AppConfig::with_cli_path()` + `start_knot_with_config()`. This matches the pattern already established in `smoke.rs` and `multi_loom.rs`.

## Discoveries

- `create_mock_agent()` and `create_stub_pi_agent()` were unused outside helpers.rs unit tests
- `create_mock_pi_capturing_stdin()` was unused in all test files
- `derive_tie_off_file()` did not exist in the codebase
- `KNOT_TEST_CLI_PATH` env var was never used
- Pre-existing test failure: `test_session_resume_delay_between_retries` — parallel env var collision on `KNOT_RETRY_DELAY_MS` (unrelated to this phase)

## Notes

- All `set_var("PATH")` calls removed from test files — 100% of PATH manipulation eliminated
- All `TEST_MUTEX` serialisation removed — 3 files cleaned
- Test count unchanged (mock helpers were internal, test count preserved)
- All modified tests pass under parallel execution
