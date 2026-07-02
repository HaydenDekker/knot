# Phase 1: Unit test mock path isolation

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [x] `src/adapters/pi_stdio.rs` — `make_mock_path()`:
  - [x] Change from `std::env::temp_dir().join("knot-test-mock-stdio")` to `tempfile::tempdir().unwrap()`
  - [x] Return `(PathBuf, TempDir)` or store `TempDir` in runner for lifetime management
  - [x] Caller (`make_mock_runner()`) keeps `TempDir` handle alive
- [x] `src/adapters/pi_stdio.rs` — `make_blocking_mock_path()`:
  - [x] Same pattern: unique temp dir per call
- [x] `src/adapters/pi_json.rs` — `make_json_mock_path()`:
  - [x] Same pattern: unique temp dir per call
- [x] `src/adapters/pi_json.rs` — `make_json_blocking_mock_path()`:
  - [x] Same pattern: unique temp dir per call
- [x] All callers updated to keep `TempDir` handle alive (in test scope or stored in runner struct)
- [x] Run `cargo test --lib adapters::pi_stdio` — verify all tests pass under parallel execution (14/14)
- [x] Run `cargo test --lib adapters::pi_json` — verify all tests pass under parallel execution (16/16)
- [x] Run `cargo test --lib` — verify all 476 unit tests pass, 0 failures
- [x] Run full test suite — verify no regressions (3 pre-existing `session_resume` integration failures; no new regressions)

## Deviations

## Discoveries

## Notes

- Pattern: `make_*_mock_path()` returns `(PathBuf, tempfile::TempDir)`, and `make_*_runner()` returns `(Runner, tempfile::TempDir)`.
- All test callers destructure the tuple as `let (runner, _dir) = make_*_runner()` — `_dir` lives in test scope and is dropped when the test function returns, cleaning up the temp dir.
- Each call gets a unique temp directory, eliminating the `/tmp/knot-test-mock-*` collision that caused `runner_passes_event_metadata` to fail under parallel execution.
- Pre-existing `session_resume` integration test failures (mock identity race via `tests/helpers.rs` `PATH` manipulation) are untouched by this phase — those are addressed in Phase 5.
