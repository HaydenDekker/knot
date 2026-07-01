# Phase 2: Integration Tests — Update All Tie-Off Path Assertions

**Plan:** [Flatten Tie-Off Paths](flat-tie-off-paths-plan.md)

## Checklist
- [x] Update `tests/tie_off.rs` — change all `tie-offs/review-loom/review/review-tie-off.md` to `tie-offs/review-loom/review-tie-off.md`
- [x] Update `tests/skill_integration.rs` — change all `tie-offs/review-loom/review/review-tie-off.md` to `tie-offs/review-loom/review-tie-off.md`
- [x] Update `tests/adapter_integration.rs` — change all `tie-offs/review-loom/review/review-tie-off.md` to `tie-offs/review-loom/review-tie-off.md`
- [x] Update `tests/agent_integration.rs` — change `tie-offs/review-loom/review/` to `tie-offs/review-loom/` and `tie-offs/review-loom/review/review-tie-off.md` to `tie-offs/review-loom/review-tie-off.md`
- [x] Update `tests/session_resume.rs` — change all `tie-offs/review-loom/review/review-tie-off.md` to `tie-offs/review-loom/review-tie-off.md`
- [x] Update `tests/pipeline.rs` — change `tie-offs/review-loom/review/` to `tie-offs/review-loom/` and `tie-offs/review-loom/review/review-tie-off.md` to `tie-offs/review-loom/review-tie-off.md`
- [x] Review `tests/helpers.rs` — check for any tie-off file path references (loom-log paths should be unchanged)
- [x] Review `tests/rig_cli.rs` — check share command test setup for nested tie-off directory creation
- [x] Search `tests/` for any remaining `tie-offs/.*\/.*\/.*-tie-off` pattern (nested paths)
- [x] Run `cargo test` — verify full test suite passes (individual tests pass; parallel failures are pre-existing test interference)
- [x] Run `cargo test tie_off` — verify tie-off specific tests pass
- [x] Run `cargo test skill_integration` — verify skill tests pass
- [x] Run `cargo test pipeline` — verify pipeline tests pass (all pass individually)

## Deviations

- Parallel pipeline test failures (`pipeline_handles_agent_failure`, `pipeline_ignores_binary_files_and_processes_text_files`, `pipeline_processes_non_md_text_files`, `pipeline_processes_strand_create`) are pre-existing — caused by tests mutating global `PATH` without sufficient isolation. All pass when run individually. Not caused by this change.

## Discoveries

- `tests/helpers.rs` only references loom-log paths under `tie-offs/`, not tie-off file paths — no changes needed.
- `tests/rig_cli.rs` share command test already uses flat tie-off file naming (`review.tie-off.json`) — no nested paths to update.

## Notes
