# Phase 0: Smoke tests and composition wiring

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [x] Add `cli_path: Option<PathBuf>` field to `AppConfig` struct in `src/server.rs`
- [x] Add `AppConfig::with_cli_path(path)` convenience method
- [x] Wire `cli_path` into `build_app_context()` — when `Some(path)`, create runner with explicit CLI path via `with_cli_path_and_timeout()`
- [x] Create `tests/smoke.rs` test module
- [x] `composition_smoke_stdio` test:
  - [x] `tempfile::tempdir()` for rig + strands
  - [x] Mock agent script at `{rig}/bin/pi` (echo "review complete", exit 0)
  - [x] Write `.workspace-agent-config.yaml` with `agent-adapter: pi-stdio`
  - [x] Write knot file + profile + strand file
  - [x] `start_knot()` with `cli_path` pointing to mock
  - [x] Wait for `state.json` loom knot status "completed"
  - [x] Assert tie-off file exists with expected content
- [x] `composition_smoke_json` test:
  - [x] Same rig setup, `agent-adapter: pi-json`
  - [x] Mock agent script that outputs JSON-L with session ID
  - [x] Same assertions
- [x] Run `cargo test --test smoke` — both tests pass (5.04s)
- [x] Run `cargo test --test smoke --test-threads=4` — parallel execution passes (5.06s)
- [x] Run full test suite — 476 passed, 0 failed, no regressions

## Deviations

- `PiStdioAgentRunner` and `PiJsonAgentRunner` get `with_cli_path_and_timeout(cli_path, timeout)` instead of mutating a private `cli_path` field — this keeps the existing `#[cfg(test)] with_cli_path(String)` for unit tests while providing a clean public constructor for the composition root.

## Discoveries

- `AppConfig::with_cli_path()` uses a builder pattern (`self` by value, returns `Self`) for chaining with `with_rig_dir()`.
- `tests/helpers.rs` gets `start_knot_with_config(AppConfig)` as the primary helper; existing `start_knot(rig_dir)` delegates to it.
- The smoke tests create the strand file *after* loom discovery (`wait_for_loom_in_state`) to ensure the notify watcher is active and picks up the file creation event.

## Notes
