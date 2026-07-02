# Phase 3: Application test migration — core use cases

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Rewrite `tests/agent_integration.rs` against mock ports:
  - [ ] `agent_execution_produces_tie_off` → `ProcessStrand` with `TrackingTieOffSink`
  - [ ] `agent_execution_append_mode_tie_offs` → verify multiple append calls
  - [ ] `agent_execution_updates_state_file` → verify state transitions
  - [ ] `agent_failure_records_error_in_state` → `TrackingAgentRunner` returning `Err(AgentExecutionFailed)`
  - [ ] `agent_failure_records_loom_log_entry` → `TrackingLoomLog` captures error event
  - [ ] `tie_off_contains_agent_output` → verify tie-off content matches agent output
  - [ ] `agent_handles_deleted_strand` → deleted event flow
  - [ ] `agent_handles_multiple_looms_independently` → two ProcessStrand calls
  - [ ] `agent_state_transitions_through_processing` → verify state progression
  - [ ] `strand_processed_no_error_on_success` → happy path
- [ ] Remove from `tests/agent_integration.rs`:
  - [ ] `TEST_MUTEX` and `acquire_test_lock()`
  - [ ] `start_knot()` calls
  - [ ] `create_mock_pi()` / `create_stub_pi_agent()` calls
  - [ ] `wait_for_state_field()` / `wait_for_knot_status_in_state()` calls
  - [ ] `set_var("PATH")` calls
- [ ] Verify: all 10 scenarios pass as mock-port tests, <1s total
- [ ] Run full test suite — verify no regressions, old integration tests still pass alongside new ones

## Deviations

## Discoveries

## Notes
