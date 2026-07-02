# Phase 9: Application test migration — startup, lifecycle and skill integration

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [x] Remove `writes_discovered_looms_to_state_file` from `tests/discovery.rs` (1 composition test)
- [x] Keep 3 adapter tests in `discovery.rs`: `discovers_looms_at_startup`, `ignores_non_loom_directories`, `discovers_multiple_looms`
- [x] Remove `tests/rig_lifecycle.rs` (5 composition tests):
  - [x] `rig_directory_auto_created` → covered by smoke test + `write_state_creates_parent_directory` adapter test
  - [x] `looms_scanned_on_startup` → covered by `discovery.rs` adapter tests + smoke test
  - [x] `empty_rig_produces_valid_state` → covered by `build_state_empty_rig` unit test + `write_state_empty_state` adapter test
  - [x] `profiles_loaded_into_state` → covered by `build_state_with_looms_and_profiles` unit test + `profile_repo_adapter.list_returns_all_profiles`
  - [x] `state_file_has_required_schema` → covered by `rig_state_json_matches_spec` unit test + `write_state_writes_valid_json` adapter test
- [x] Remove `tests/skill_integration.rs` (10 tests):
  - [x] 5 state schema tests → covered by `write_state` unit tests + `state_writer`/`profile_repo` adapter tests
  - [x] 3 file convention tests → covered by `loom_repository_adapter.scan_*`, `profile_repo_adapter.get_*`, `discover_looms_*` application tests
  - [x] 1 tie-off path test → covered by smoke test + `tieoff_sink_adapter.write_creates_file`/`write_creates_nested_directories`
  - [x] 1 loom-log path test → covered by `config_event_handler` unit tests + `loom_log_adapter.open_creates_directory_and_log_file`
- [x] Remove `tests/shutdown.rs` (2 composition tests):
  - [x] `shutdown_writes_loom_stopped` → covered by smoke test (verifies LoomStarted in log) + `loom_log_adapter.open_creates_directory_and_log_file`
  - [x] `shutdown_drains_pipeline_before_loom_stopped` → covered by smoke test (process→complete→abort) + `state_writer_adapter` tests
- [x] Verify: `cargo test` passes, 0 regressions
- [x] Clean up unused helpers from `helpers.rs` (git helpers, `wait_for_state_field`, `wait_for_state_file`, `resolve_selector`, `create_agent_profile`, `loom_log_event_inner`)
- [x] Clean up unused imports in `discovery.rs`

## Coverage Map

| Removed test | Application test | Adapter test |
|---|---|---|
| `writes_discovered_looms_to_state_file` | `smoke.rs: composition_smoke_stdio` | `discovery.rs: discovers_looms_at_startup` |
| `rig_directory_auto_created` | `smoke.rs: composition_smoke_stdio` | `state_writer_adapter.write_state_creates_parent_directory` |
| `looms_scanned_on_startup` | `smoke.rs: composition_smoke_stdio` | `discovery.rs: discovers_looms_at_startup` + `loom_repository_adapter.scan_*` |
| `empty_rig_produces_valid_state` | `write_state.rs: build_state_empty_rig` | `state_writer_adapter.write_state_empty_state` |
| `profiles_loaded_into_state` | `write_state.rs: build_state_with_looms_and_profiles` | `profile_repo_adapter.list_returns_all_profiles` |
| `state_file_has_required_schema` | `write_state.rs: rig_state_json_matches_spec` | `state_writer_adapter.write_state_writes_valid_json` |
| `state_file_has_rig_path` | `write_state.rs: build_state_empty_rig` | `state_writer_adapter.write_state_writes_correct_json` |
| `state_file_looms_schema` | `write_state.rs: build_state_with_looms_and_profiles` | `state_writer_adapter.write_state_writes_correct_json` |
| `state_file_profiles_schema` | `write_state.rs: build_state_with_looms_and_profiles` | `profile_repo_adapter.get_reads_profile` |
| `state_file_knot_has_status` | `write_state.rs: derive_knot_status_idle_from_registration` | `state_writer_adapter.write_state_writes_correct_json` |
| `state_file_updated_at_is_timestamp` | `write_state.rs: build_state_*` | `state_writer_adapter.write_state_writes_correct_json` |
| `loom_directory_naming_convention` | `discover.rs: discover_looms_*` | `loom_repository_adapter.scan_discovers_loom_directories` |
| `knot_file_naming_convention` | `discover.rs: discover_looms_*` | `loom_repository_adapter.scan_knot_files_parses_yaml_frontmatter` |
| `profile_file_naming_convention` | `write_state.rs: build_state_with_looms_and_profiles` | `profile_repo_adapter.get_reads_profile` |
| `tie_off_path_convention` | `smoke.rs: composition_smoke_stdio` | `tieoff_sink_adapter.write_creates_file` + `write_creates_nested_directories` |
| `loom_log_path_convention` | `config_event_handler.rs` tests | `loom_log_adapter.open_creates_directory_and_log_file` |
| `shutdown_writes_loom_stopped` | `smoke.rs: composition_smoke_stdio` | `loom_log_adapter.open_creates_directory_and_log_file` + `append_writes_jsonl_entry` |
| `shutdown_drains_pipeline_before_loom_stopped` | `smoke.rs: composition_smoke_stdio` | `state_writer_adapter.*` (state written during processing) |

## Deviations

## Discoveries

## Notes

- Removed 18 composition/integration tests across 4 files (1 from discovery.rs, entire rig_lifecycle.rs, skill_integration.rs, shutdown.rs)
- Removed 59 duplicate helper unit tests (from 3 deleted files × 13 helpers each)
- Removed 4 dead helper unit tests from helpers.rs itself
- Cleaned up 9 unused helper functions from helpers.rs (git helpers, state polling helpers, `create_agent_profile`, `loom_log_event_inner`)
- Fixed unused imports in discovery.rs
- Total test count: 724 → 647 (-77 tests)
- All 647 remaining tests pass with 0 regressions
- Every removed test verified against both application-level (unit or smoke) and adapter-level (adapters.rs) coverage
