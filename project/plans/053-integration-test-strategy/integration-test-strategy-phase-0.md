# Phase 0: Fix tie-off path assertions

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Add `derive_tie_off_file(loom_id, knot_name, rig_dir)` helper to `tests/helpers.rs` that returns `rig_dir/tie-offs/{loom-id}/{knot-name}-tie-off.md`
- [ ] Update `tests/agent_integration.rs`:
  - [ ] `agent_execution_produces_tie_off` — replace `tie-offs/review-loom/review/review-tie-off.md` with helper
  - [ ] `agent_execution_append_mode_tie_offs` — replace `tie-offs/review-loom/review/review-tie-off.md` with helper
  - [ ] `tie_off_contains_agent_output` — replace `tie-offs/review-loom/review/review-tie-off.md` with helper
- [ ] Update `tests/pipeline.rs`:
  - [ ] `pipeline_processes_strand_create` — replace `tie-offs/review-loom/review/...` with helper
  - [ ] `pipeline_ignores_binary_files_and_processes_text_files` — replace with helper
  - [ ] `pipeline_processes_non_md_text_files` — replace with helper
  - [ ] `delete_event_large_tieoff_bounded_context` — replace `tie-offs/review-loom/review/...` with helper
- [ ] Check `tests/agent_integration.rs` line 42: `tie_off_dir = rig_dir.join("tie-offs/review-loom/review")` — this dir join is wrong, tie-offs are now flat. Replace with helper
- [ ] Check `tests/pipeline.rs` lines 47, 418, 480: same `tie_off_dir` pattern — replace with helper
- [ ] Run `cargo test --test agent_integration` — verify 3 previously failing tests now pass
- [ ] Run `cargo test --test pipeline` — verify 4 previously failing tie-off-path tests now pass
- [ ] Run full test suite — verify no regressions

## Deviations

## Discoveries

## Notes
