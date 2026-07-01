# Phase 2: Fast session resume for tests

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Add test-only env var support to `src/application/session_resume.rs`:
  - [ ] Read `KNOT_TEST_RETRY_DELAY_MS` for retry delay (default: production value, e.g. 10000ms)
  - [ ] Read `KNOT_TEST_MAX_RETRIES` for max retries (default: production value, 10)
  - [ ] Use `#[cfg(test)]` or env var with `unwrap_or` to keep production defaults unchanged
- [ ] Update `process_strand_retry_exhausted_fails` unit test in `src/application/session_resume.rs`:
  - [ ] Set `KNOT_TEST_RETRY_DELAY_MS=50` and `KNOT_TEST_MAX_RETRIES=3` before test
  - [ ] Verify test completes in <1s instead of 60s+
- [ ] Update `tests/session_resume.rs` integration tests:
  - [ ] Set reduced retry parameters via env vars before `start_knot()`
  - [ ] Or use profile-level timeout to limit budget (e.g. 5s profile timeout)
  - [ ] Verify `test_session_resume_success` completes in <5s
  - [ ] Verify `test_session_resume_transparent_on_success` completes in <5s
  - [ ] Verify `test_session_resume_delay_between_retries` completes in <5s
  - [ ] Verify `test_session_resume_exhausted` completes in <5s
- [ ] Run `cargo test --lib session_resume` — verify unit tests pass and are fast
- [ ] Run `cargo test --test session_resume` — verify integration tests pass and are fast
- [ ] Run full test suite — verify total duration is under 60s

## Deviations

## Discoveries

## Notes
