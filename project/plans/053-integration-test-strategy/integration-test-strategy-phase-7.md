# Phase 7: Verify and record

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Run `cargo test` — target: 0 failures, <30s total
- [ ] Record total test suite duration
- [ ] Run `cargo test --test-threads=4` — verify identical results (same pass/fail)
- [ ] Run `cargo clippy` — verify no new warnings
- [ ] Verify test tier counts:
  - [ ] Application tests: ~90 (mock ports)
  - [ ] Adapter tests: 8 (one per adapter)
  - [ ] Smoke tests: 2 (stdio + json)
  - [ ] Total: ~100
- [ ] Verify no `TEST_MUTEX` / `acquire_test_lock()` in any test file
- [ ] Verify no `std::env::set_var("PATH")` in any test file
- [ ] Verify no `std::env::set_var("KNOT_TEST_CLI_PATH")` in any test file
- [ ] Update `master-plan.md` — mark plan complete
- [ ] Record final test suite duration and test counts in plan completion notes

## Deviations

## Discoveries

## Notes
