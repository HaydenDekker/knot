# Phase 1: `clear` / `clear_all` port methods with adapter tests (TDD)

**Plan:** [Clear Loom-Logs and Rig-Log at Startup](startup-log-clear-plan.md)

## Checklist
- [ ] Write failing unit tests: `rig_log_clear_truncates`, `rig_log_clear_missing_file_is_noop`, `loom_log_clear_all_truncates_every_loom_log`, `loom_log_clear_all_includes_orphan_looms`, `loom_log_clear_all_leaves_other_files_alone`
- [ ] Add `RigLogPort::clear()` and `LoomLogPort::clear_all()` to `src/application/ports.rs`
- [ ] Implement `clear()` in `FileSystemRigLog` (`src/adapters/outbound/rig_log.rs`)
- [ ] Implement `clear_all()` in `FileSystemLoomLog` (`src/adapters/outbound/loom_log.rs`)
- [ ] Update all mock implementations (ports.rs, test_fixtures.rs, session_resume.rs, write_state.rs) so the suite compiles
- [ ] Compile and verify all Phase 1 tests green

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
