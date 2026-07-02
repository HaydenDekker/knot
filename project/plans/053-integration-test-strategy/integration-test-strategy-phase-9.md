# Phase 9: Application test migration — startup, lifecycle and skill integration

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Remove `writes_discovered_looms_to_state_file` from `tests/discovery.rs` (1 composition test)
- [ ] Keep 3 adapter tests in `discovery.rs`: `discovers_looms_at_startup`, `ignores_non_loom_directories`, `discovers_multiple_looms`
- [ ] Remove `tests/rig_lifecycle.rs` (5 composition tests):
  - [ ] `rig_directory_auto_created` → covered by smoke test
  - [ ] `looms_scanned_on_startup` → covered by `discovery.rs` adapter tests + smoke test
  - [ ] `empty_rig_produces_valid_state` → covered by `build_state_empty_rig` unit test + smoke test
  - [ ] `profiles_loaded_into_state` → covered by smoke test
  - [ ] `state_file_has_required_schema` → covered by `build_state_*` unit tests
- [ ] Remove `tests/skill_integration.rs` (10 tests):
  - [ ] 5 state schema tests → covered by `write_state` unit tests
  - [ ] 3 file convention tests → trivial assertions, no runtime needed
  - [ ] 1 tie-off path test → covered by smoke test
  - [ ] 1 loom-log path test → covered by smoke test + `loom_log` adapter tests
- [ ] Remove `tests/shutdown.rs` (2 composition tests):
  - [ ] `shutdown_writes_loom_stopped` → covered by smoke test
  - [ ] `shutdown_drains_pipeline_before_loom_stopped` → covered by smoke test
- [ ] Verify: `cargo test` passes, 0 regressions

## Deviations

## Discoveries

## Notes
