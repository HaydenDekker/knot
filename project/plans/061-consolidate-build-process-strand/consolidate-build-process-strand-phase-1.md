# Phase 1: Replace local `build_process_strand` in all 8 test files

**Plan:** [Consolidate `build_process_strand` Test Helpers](consolidate-build-process-strand-plan.md)

## Checklist
- [x] Update `tie_off.rs` — simplest case, just `ProcessStrandBuilder::new(loom, runner).build()`
- [x] Update `rig_log.rs` — simplest case, just `ProcessStrandBuilder::new(loom, runner).build()`
- [x] Update `agent_integration.rs` — simplest case, just `ProcessStrandBuilder::new(loom, runner).build()`
- [x] Update `profile_timeout.rs` — add `.with_profile(custom_profile)`
- [x] Update `session_resume.rs` — add `.with_profile(custom_profile)`
- [x] Update `git_versioning.rs` — add `.with_tracking_git()`
- [x] Update `event_enforcement.rs` — add `.with_looms(vec![...])` + `.with_tracking_event_dispatcher()`
- [x] Update `pipeline.rs` — add `.with_tracking_git()` + `.with_tracking_file_checker()`
- [x] Remove all 8 local `fn build_process_strand` definitions
- [x] Verify: `cargo test` — all 64 integration tests still pass, clippy clean

## Deviations

None.

## Discoveries

- `ProcessStrandResult` fields `git_port`, `git_commits`, `file_checker`, and `event_dispatcher` are `Option` types (present only when tracking is enabled). Tests that need these fields must use `.as_ref().expect("...")` to extract a reference before use, since `let Some(x)` is a refutable pattern in a `let` binding.
- `ProcessStrandBuilder::new()` requires a single `Loom` parameter, but `.with_looms(Vec<Loom>)` overrides it. For tests with no single primary loom (like `event_enforcement.rs`), a dummy loom `Loom { id: LoomId(String::new()), knots: vec![] }` is passed to `new()` and immediately overridden.

## Notes
