# Phase 6: Composition root and helper cleanup

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] `src/server.rs` — `AppConfig`:
  - [ ] `cli_path: Option<PathBuf>` field present
  - [ ] `with_cli_path()` method present
  - [ ] `build_app_context()` uses `cli_path` when `Some`
- [ ] `tests/helpers.rs` cleanup:
  - [ ] Remove `TEST_MUTEX` / `acquire_test_lock()` (no longer needed)
  - [ ] Remove `create_mock_pi()` / `create_mock_pi_capturing_stdin()` / `create_mock_agent()` / `create_stub_pi_agent()` — replaced by adapter test helpers
  - [ ] Remove `derive_tie_off_file()` — move to domain or inline
  - [ ] Remove `wait_for_state_field()`, `wait_for_knot_status_in_state()`, `wait_for_loom_in_state()` — used only by smoke tests, inline them
  - [ ] Remove `wait_for_loom_log_event()` — used only by smoke tests, inline
  - [ ] Keep: `start_knot()` (smoke test helper), `make_knot_content()`, `create_knot_file()`, `create_fast_profile()`, `create_loom_dir()`, `create_strand()`, `read_state_file()`, `read_loom_log()`
  - [ ] `start_knot()` simplified — no debounce env var overrides (smoke tests use fast debounce via profile or explicit config)
- [ ] Remove `set_var("PATH")` from all remaining test files
- [ ] Remove `set_var("KNOT_TEST_CLI_PATH")` from all remaining test files
- [ ] Verify: all test files compile, no references to removed helpers
- [ ] Run full test suite — verify no regressions

## Deviations

## Discoveries

## Notes
