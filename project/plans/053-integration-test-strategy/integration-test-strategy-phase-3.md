# Phase 3: Consolidate mock helpers and verify full suite

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Audit `tests/helpers.rs` for unused/mock functions:
  - [ ] `create_mock_agent()` — is it used anywhere? If not, remove
  - [ ] `create_stub_pi_agent()` — is it used anywhere? If not, remove
  - [ ] `create_mock_pi_capturing_stdin()` — is it used anywhere? If not, remove
- [ ] Consolidate mock creation:
  - [ ] Single `create_mock_pi(rig_dir, response) -> PathBuf` that returns path and writes config
  - [ ] Optional: `create_mock_pi_with_stdin_capture(rig_dir, response, capture_path) -> PathBuf`
  - [ ] Document that callers must pass returned path to `start_knot(rig_dir, Some(path))`
- [ ] Verify all test files use consolidated helpers:
  - [ ] `tests/adapter_integration.rs`
  - [ ] `tests/agent_integration.rs`
  - [ ] `tests/pipeline.rs`
  - [ ] `tests/session_resume.rs`
  - [ ] `tests/multi_loom.rs`
  - [ ] `tests/profile_timeout.rs`
- [ ] Remove any remaining `std::env::set_var("PATH", ...)` or `std::env::set_var("KNOT_TEST_CLI_PATH", ...)` from test files
- [ ] Run `cargo test --test-threads=1` — verify serial execution passes
- [ ] Run `cargo test` (default parallel) — verify all tests pass
- [ ] Record total test suite duration — target <60s
- [ ] Run `cargo clippy` — verify no new warnings
- [ ] Verify `--test-threads=1` and default parallel produce identical results (same pass/fail)

## Deviations

## Discoveries

## Notes
