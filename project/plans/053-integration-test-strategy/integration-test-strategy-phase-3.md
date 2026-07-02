# Phase 3: Application test migration — core use cases

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [x] Rewrite `tests/agent_integration.rs` against mock ports:
  - [x] `agent_execution_produces_tie_off` → `ProcessStrand` with `TrackingTieOffSink`
  - [x] `agent_execution_append_mode_tie_offs` → verify multiple append calls
  - [x] `agent_execution_updates_state_file` → verify state transitions
  - [x] `agent_failure_records_error_in_state` → `TrackingAgentRunner` returning `Err(AgentExecutionFailed)`
  - [x] `agent_failure_records_loom_log_entry` → `TrackingLoomLog` captures error event
  - [x] `tie_off_contains_agent_output` → verify tie-off content matches agent output
  - [x] `agent_handles_deleted_strand` → deleted event flow
  - [x] `agent_handles_multiple_looms_independently` → two ProcessStrand calls
  - [x] `agent_state_transitions_through_processing` → verify state progression
  - [x] `strand_processed_no_error_on_success` → happy path
- [x] Remove from `tests/agent_integration.rs`:
  - [x] `TEST_MUTEX` and `acquire_test_lock()`
  - [x] `start_knot()` calls
  - [x] `create_mock_pi()` / `create_stub_pi_agent()` calls
  - [x] `wait_for_state_field()` / `wait_for_knot_status_in_state()` calls
  - [x] `set_var("PATH")` calls
- [x] Verify: all 10 scenarios pass as mock-port tests, <1s total (0.00s)
- [x] Run full test suite — verify no regressions, old integration tests still pass alongside new ones

## Deviations

None. Implementation matched the plan exactly.

## Discoveries

- `test_fixtures` module needed to be made `pub` (was `#[cfg(test)]`) so integration tests can import the mock types (`MockLoomLogPort`, `MockAgentRunner`, `TrackingTieOffSink` etc.) from `knot::application::usecases::test_fixtures`.
- `StrandPath::should_process()` checks `PathBuf::exists()` for Created/Modified events, so tests must create real files on disk (via `tempfile::tempdir()`) even though the port adapters are mocked. Deleted events skip this check.
- `ProcessStrand` is re-exported from `knot::application::usecases` (not from the private `process_strand` module), so integration tests import it as `knot::application::usecases::ProcessStrand`.
- `RigAgentConfig` is re-exported from `knot` root (not `knot::domain`), so import as `knot::RigAgentConfig`.

## Notes

- All 10 tests complete in 0.00s (previously ~45s with full Knot lifecycle per test).
- No `TEST_MUTEX`, no PATH manipulation, no `start_knot()` calls — tests run fully parallel.
- Tests use `tempfile::tempdir()` to create real strand files on disk (required by `StrandPath::should_process()` file existence check).
