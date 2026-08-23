# Phase 1: `clear` / `clear_all` port methods with adapter tests (TDD)

**Plan:** [Clear Loom-Logs and Rig-Log at Startup](startup-log-clear-plan.md)

## Checklist
- [x] Write failing unit tests: `rig_log_clear_truncates`, `rig_log_clear_missing_file_is_noop`, `loom_log_clear_all_truncates_every_loom_log`, `loom_log_clear_all_includes_orphan_looms`, `loom_log_clear_all_leaves_other_files_alone` (plus `loom_log_clear_all_missing_root_is_noop` for the fresh-rig case) — written first, confirmed red (compile failure: methods missing)
- [x] Add `RigLogPort::clear()` and `LoomLogPort::clear_all()` to `src/application/ports.rs`
- [x] Implement `clear()` in `FileSystemRigLog` (`src/adapters/outbound/rig_log.rs`) — `fs::File::create` in-place truncate, no-op when missing
- [x] Implement `clear_all()` in `FileSystemLoomLog` (`src/adapters/outbound/loom_log.rs`) — enumerate `derive_runtime_root` subdirs, truncate only top-level `*/.loom-log`, no-op on missing root
- [x] Update all mock implementations (ports.rs ×2, test_fixtures.rs ×2, session_resume.rs, write_state.rs) so the suite compiles
- [x] Compile and verify all Phase 1 tests green — 6/6 clear tests pass

## Deviations
- `clear`/`clear_all` are **required** trait methods (no default impls), per the plan's "add the methods to every mock port implementation" — forces every future implementation to make a conscious clear decision.
- The plan lists three mock locations; a fourth exists (`MockLoomLogForState` in `write_state.rs`) and was updated too.
- `MockLoomLogPort` in `ports.rs` had a bare `Vec` field; changed to `Mutex<Vec<…>>` so `clear_all` works behind `&self` (mock is used only for object-safety/contract tests).

## Discoveries
- `FileSystemLoomLog` is constructed with the **rig dir** (derives the runtime root internally via `derive_runtime_root`), while `FileSystemRigLog` is constructed with the **runtime root** directly — `clear_all` enumerates `derive_runtime_root(&self.rig_dir)`, and `clear` uses the field as-is.
- `TestLoomLog` in `session_resume.rs` records `clear_all` call counts for assertions, per the plan.
- Pre-existing unrelated lib-test failure on clean tree: `domain::events::tests::build_listener_context_prompt_includes_do_not_edit_guidance` (listener prompt wording; fails on base commit, untouched by this plan).

## Notes
- Truncation is in-place (`fs::File::create`), not deletion: append mode reopens per write, file identity stays stable (matches plan rationale).
- `clear_all` never descends into loom subdirs — a `.loom-log` nested in a dispatch dir is left untouched (pinned by `loom_log_clear_all_leaves_other_files_alone`).
