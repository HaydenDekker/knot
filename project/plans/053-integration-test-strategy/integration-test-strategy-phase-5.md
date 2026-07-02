# Phase 5: Application test migration — session resume and edge cases

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Rewrite `tests/session_resume.rs` against mock ports:
  - [ ] `test_session_resume_success` → `TrackingAgentRunner` fails call 1 (with `session_id`), succeeds call 2 → verify 2 calls, `--session-id` injected
  - [ ] `test_session_resume_transparent_on_success` → verify tie-off written normally on retry success
  - [ ] `test_session_resume_delay_between_retries` → `TrackingAgentRunner` with clock verification (or reduced delay via env var)
  - [ ] `test_session_resume_exhausted` → `TrackingAgentRunner` fails all retries → verify max retry count, error returned
  - [ ] `test_session_resume_non_resumable_error` → `TrackingAgentRunner` returns non-resumable error → verify no retry
  - [ ] Session resume adapter test → `PiStdioAgentRunner` with mock that returns JSON containing `session_id` → verify error carries it
  - [ ] Remove `TEST_MUTEX`, `acquire_test_lock()`, `start_knot()`
- [ ] Rewrite `tests/profile_timeout.rs` against mock ports:
  - [ ] Timeout enforcement → `TrackingAgentRunner` with timeout verification
  - [ ] Remove `TEST_MUTEX`, `acquire_test_lock()`, `set_var("PATH")`
- [ ] Migrate `tests/multi_loom.rs`:
  - [ ] This remains a composition test (two looms watching, events don't cross)
  - [ ] But uses `cli_path` injection instead of `set_var("PATH")`
  - [ ] Remove `TEST_MUTEX`, `acquire_test_lock()` — unique `tempfile::tempdir()` per test
- [ ] Verify: session resume tests <5s each, multi-loom <5s
- [ ] Run full test suite — verify no regressions

## Deviations

## Discoveries

## Notes
