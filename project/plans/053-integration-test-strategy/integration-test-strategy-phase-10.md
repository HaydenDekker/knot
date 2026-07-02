# Phase 10: Remove adapter integration tests and dead helpers

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Remove `tests/adapter_integration.rs` (3 composition tests):
  - [ ] `test_json_invocation_full_pipeline` → covered by `smoke.rs` + `adapters.rs` PiJson adapter tests
  - [ ] `test_stdio_invocation_full_pipeline` → covered by `smoke.rs` + `adapters.rs` PiStdio adapter tests
  - [ ] `test_json_invocation_timeout_captures_session_id` → covered by `adapters.rs` PiJson adapter test
- [ ] Remove dead helpers from `tests/helpers.rs`:
  - [ ] `start_knot(rig_dir: PathBuf)` — no longer used (smoke/multi_loom use `start_knot_with_config`)
  - [ ] `init_git_repo()` — dead code since `git_versioning.rs` uses mock ports
  - [ ] `get_latest_commit()` — dead code
  - [ ] `count_commits()` — dead code
  - [ ] `run_git()` — dead code
  - [ ] `build_profile_with_timeout()` — dead code (`session_resume.rs` has its own)
  - [ ] `wait_for_loom_log_event_with_deadline()` — only used by removed `adapter_integration.rs`
- [ ] Keep helpers used by remaining files:
  - [ ] `start_knot_with_config()` — used by `smoke.rs`, `multi_loom.rs`
  - [ ] `create_loom_dir()`, `create_knot_file()`, `create_fast_profile()`, `create_strand()` — used by `smoke.rs`, `multi_loom.rs`, `discovery.rs`
  - [ ] `wait_for_loom_in_state()`, `wait_for_knot_status_in_state()`, `wait_for_state_file()` — used by `smoke.rs`, `multi_loom.rs`
  - [ ] `read_loom_log()`, `loom_log_event_type()` — used by `smoke.rs`, `multi_loom.rs`
- [ ] Verify: `cargo test` passes, `cargo clippy` clean

## Deviations

## Discoveries

## Notes
