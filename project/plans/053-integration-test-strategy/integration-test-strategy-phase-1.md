# Phase 1: Unit test mock path isolation

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] `src/adapters/pi_stdio.rs` — `make_mock_path()`:
  - [ ] Change from `std::env::temp_dir().join("knot-test-mock-stdio")` to `tempfile::tempdir().unwrap()`
  - [ ] Return `(PathBuf, TempDir)` or store `TempDir` in runner for lifetime management
  - [ ] Caller (`make_mock_runner()`) keeps `TempDir` handle alive
- [ ] `src/adapters/pi_stdio.rs` — `make_blocking_mock_path()`:
  - [ ] Same pattern: unique temp dir per call
- [ ] `src/adapters/pi_json.rs` — `make_json_mock_path()`:
  - [ ] Same pattern: unique temp dir per call
- [ ] `src/adapters/pi_json.rs` — `make_json_blocking_mock_path()`:
  - [ ] Same pattern: unique temp dir per call
- [ ] All callers updated to keep `TempDir` handle alive (in test scope or stored in runner struct)
- [ ] Run `cargo test --lib adapters::pi_stdio` — verify all tests pass under parallel execution
- [ ] Run `cargo test --lib adapters::pi_json` — verify all tests pass under parallel execution
- [ ] Run `cargo test --lib` — verify all 476 unit tests pass, 0 failures
- [ ] Run full test suite — verify no regressions

## Deviations

## Discoveries

## Notes
