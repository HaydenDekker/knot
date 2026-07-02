# Phase 10: Remove adapter integration tests and dead helpers

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [x] Remove `tests/adapter_integration.rs` (3 composition tests):
  - [x] `test_json_invocation_full_pipeline` → covered by `smoke.rs` + `adapters.rs` PiJson adapter tests
  - [x] `test_stdio_invocation_full_pipeline` → covered by `smoke.rs` + `adapters.rs` PiStdio adapter tests
  - [x] `test_json_invocation_timeout_captures_session_id` → covered by `adapters.rs` PiJson adapter test
- [x] Remove dead helpers from `tests/helpers.rs`:
  - [x] `start_knot(rig_dir: PathBuf)` — no longer used (smoke/multi_loom use `start_knot_with_config`)
  - [x] `wait_for_loom_log_event_with_deadline()` — only used by removed `adapter_integration.rs`
  - [x] `wait_for_loom_log_event()` — only used by removed `adapter_integration.rs`
  - [x] `init_git_repo()`, `get_latest_commit()`, `count_commits()`, `run_git()`, `build_profile_with_timeout()` — already removed in prior phases
- [x] Keep helpers used by remaining files:
  - [x] `start_knot_with_config()` — used by `smoke.rs`, `multi_loom.rs`
  - [x] `create_loom_dir()`, `create_knot_file()`, `create_fast_profile()`, `create_strand()` — used by `smoke.rs`, `multi_loom.rs`, `discovery.rs`
  - [x] `wait_for_loom_in_state()`, `wait_for_knot_status_in_state()` — used by `smoke.rs`, `multi_loom.rs`
  - [x] `read_loom_log()`, `loom_log_event_type()` — used by `multi_loom.rs`
  - [x] `read_state_file()` — used by `multi_loom.rs`
- [x] Verify: `cargo test` passes, `cargo clippy` clean

## Deviations

## Discoveries

## Notes
- `init_git_repo()`, `get_latest_commit()`, `count_commits()`, `run_git()`, `build_profile_with_timeout()` were already removed in prior phases — not present in `helpers.rs` at time of this phase.
- `wait_for_loom_log_event()` (non-deadline variant) was also dead — only called from `adapter_integration.rs` — removed alongside it.
- `wait_for_state_file()` was listed in "keep" but doesn't exist in helpers.rs — never existed.

## Deviations

## Discoveries

## Notes
