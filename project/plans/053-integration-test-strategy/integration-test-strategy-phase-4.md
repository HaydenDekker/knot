# Phase 4: Application test migration — pipeline and file-based tests

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [x] Rewrite `tests/pipeline.rs` against mock ports:
  - [x] Debounce + process flow tests (events → debounce → process → tie-off)
  - [x] Binary file skip → `TrackingAgentRunner` never called for binary content
  - [x] Non-md text file processing → agent called
  - [x] Delete event context extraction → verify prompt content
  - [x] Large tie-off bounded context → verify context truncation
  - [x] Strand skip (known temp file) → verify skipped silently
  - [x] Strand skip (unknown missing file) → verify loom-log entry
  - [x] Remove `TEST_MUTEX`, `acquire_test_lock()`, `start_knot()`
- [x] Rewrite `tests/tie_off.rs` against mock ports:
  - [x] Tie-off append mode → `TrackingTieOffSink`
  - [x] Context extraction → verify read_content calls
  - [x] Remove `TEST_MUTEX`, `acquire_test_lock()`
- [x] Rewrite `tests/rig_log.rs` against mock ports:
  - [x] Rig-log append → `TrackingRigLog`
  - [x] Queue idle detection → verify rig-log event
  - [x] Remove `TEST_MUTEX`, `acquire_test_lock()`
- [x] Rewrite `tests/git_versioning.rs` against mock ports:
  - [x] Git commit → `TrackingGitVersioningPort`
  - [x] Graceful skip (non-git dir) → verify no error
  - [x] Remove `TEST_MUTEX`, `acquire_test_lock()`
- [x] Remove `set_var("PATH")` from all migrated files
- [x] Verify: all previously-covered scenarios pass as mock-port tests
- [x] Run full test suite — verify no regressions

## Deviations

## Discoveries

## Notes
