# Phase 4: Application test migration — pipeline and file-based tests

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Rewrite `tests/pipeline.rs` against mock ports:
  - [ ] Debounce + process flow tests (events → debounce → process → tie-off)
  - [ ] Binary file skip → `TrackingAgentRunner` never called for binary content
  - [ ] Non-md text file processing → agent called
  - [ ] Delete event context extraction → verify prompt content
  - [ ] Large tie-off bounded context → verify context truncation
  - [ ] Strand skip (known temp file) → verify skipped silently
  - [ ] Strand skip (unknown missing file) → verify loom-log entry
  - [ ] Remove `TEST_MUTEX`, `acquire_test_lock()`, `start_knot()`
- [ ] Rewrite `tests/tie_off.rs` against mock ports:
  - [ ] Tie-off append mode → `TrackingTieOffSink`
  - [ ] Context extraction → verify read_content calls
  - [ ] Remove `TEST_MUTEX`, `acquire_test_lock()`
- [ ] Rewrite `tests/rig_log.rs` against mock ports:
  - [ ] Rig-log append → `TrackingRigLog`
  - [ ] Queue idle detection → verify rig-log event
  - [ ] Remove `TEST_MUTEX`, `acquire_test_lock()`
- [ ] Rewrite `tests/git_versioning.rs` against mock ports:
  - [ ] Git commit → `TrackingGitVersioningPort`
  - [ ] Graceful skip (non-git dir) → verify no error
  - [ ] Remove `TEST_MUTEX`, `acquire_test_lock()`
- [ ] Remove `set_var("PATH")` from all migrated files
- [ ] Verify: all previously-covered scenarios pass as mock-port tests
- [ ] Run full test suite — verify no regressions

## Deviations

## Discoveries

## Notes
