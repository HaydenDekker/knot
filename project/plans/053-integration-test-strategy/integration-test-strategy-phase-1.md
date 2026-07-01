# Phase 1: Per-test mock agent isolation

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Add `cli_path: Option<PathBuf>` parameter to `start_knot()` in `tests/helpers.rs`
- [ ] Pass `cli_path` through `AppConfig` to the composition root — add `with_cli_path()` to `AppConfig` (or test-only constructor)
- [ ] Update composition root (`server.rs`) to use `AppConfig.cli_path` when creating `PiStdioAgentRunner` / `PiJsonAgentRunner` — prefer explicit path over `resolve_cli_path()` env var lookup
- [ ] Refactor `create_mock_pi()` in `tests/helpers.rs`:
  - [ ] Return the mock binary path instead of setting `PATH` env var
  - [ ] Remove `std::env::set_var("PATH", ...)` call
  - [ ] Still write `.workspace-agent-config.yaml` (adapter selection is needed)
- [ ] Refactor `create_mock_pi_capturing_stdin()` — same pattern: return path, don't set PATH
- [ ] Update all callers in `tests/agent_integration.rs`:
  - [ ] Pass mock path to `start_knot(rig_dir, Some(mock_path))`
  - [ ] Remove `acquire_test_lock()` calls
- [ ] Update all callers in `tests/pipeline.rs` — same pattern
- [ ] Update all callers in `tests/session_resume.rs` — same pattern (these create inline mocks that set PATH)
- [ ] Update `tests/profile_timeout.rs` — remove `acquire_test_lock()`
- [ ] Remove `TEST_MUTEX` and `acquire_test_lock()` from all test files
- [ ] Run `cargo test --test-threads=4` — verify all tests pass under parallel execution
- [ ] Run `cargo test` (default threads) — verify full parallel execution passes
- [ ] Run full test suite — verify no regressions

## Deviations

## Discoveries

## Notes
