# Plan: Integration Test Refactor — Split `integration.rs` by Feature Responsibility

## Problem

`tests/integration.rs` is a 3272-line monolithic file containing ~31 tests that span multiple concerns: rig lifecycle, startup discovery, event pipeline wiring, full E2E pipelines, agent integration, tie-off lifecycle, loom CRUD via HTTP, graceful shutdown, multi-loom isolation, per-knot source directories, and demo verification. This makes the test suite hard to navigate, slow to debug, and unclear on coverage boundaries.

Specific issues:

1. **3272 lines, ~31 tests in one file** — no feature isolation. A change to one feature requires scanning the entire file to find related tests.
2. **Heavy setup duplication** — every test creates a temp dir, writes a loom + knot, spawns a server, waits for a port, and registers a mock agent. This pattern repeats ~20 times with minor variations.
3. **Inconsistent phase numbering** — "Phase 0" through "Phase 5" appear multiple times with different meanings (e.g. two "Phase 1" sections, three "Phase 3" sections).
4. **Mixed test levels** — composition root unit test (`build_app_context_wires_layers`) sits alongside full E2E tests that spawn servers.
5. **HTTP helpers defined late** — `http_post_json` and `http_delete` are defined at the bottom of the file but used by tests in the middle, making the file hard to read linearly.
6. **Duplicate full-pipeline tests** — `full_pipeline_create_modify_delete`, `full_pipeline_subdirectory_rig`, `full_pipeline_external_source_with_agent_error`, `full_pipeline_external_source_with_mock_agent_success`, and `event_flows_through_pipeline` all test the same core flow with slightly different configurations.
7. **No clear feature-to-test mapping** — it is not obvious which tests cover which subsystem (discovery vs. pipeline vs. tie-off vs. shutdown).

## Target

Split `tests/integration.rs` into feature-focused modules under `tests/`, each with a clear responsibility. Extract shared infrastructure (HTTP helpers, test server spawning, knot fixture creation) into `tests/helpers.rs`. Every test should be findable by feature area, and the test file structure should mirror the system's architecture phases.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test File | Tests | What it covers | Status |
|-----------|-------|---------------|--------|
| `tests/integration.rs` | ~31 | Everything: rig lifecycle, discovery, pipeline, agents, tie-off, CRUD, shutdown, multi-loom, demo | ⚠️ Monolithic — all concerns mixed |
| `tests/http_interface.rs` | 3 | Health endpoint, list agents (axum Router, no server) | ✅ Clean — isolated HTTP handler tests |
| `tests/filesystem_interface.rs` | 3 | Basic fs create/list/roundtrip | ✅ Clean — minimal, focused |
| `tests/swagger_ui.rs` | 2 | Swagger UI HTML + OpenAPI JSON spec (mock ports) | ✅ Clean — uses mock ports |
| `tests/skill_integration.rs` | ~15 | Skill file validation + API contract tests (mock ports) | ✅ Clean — uses mock ports |

### Current test inventory in `integration.rs`

| # | Test Name | Target Module | What it verifies |
|---|-----------|---------------|-----------------|
| 1 | `rig_directory_auto_created` | `rig_lifecycle.rs` | Rig dir auto-created on startup |
| 2 | `rig_directory_scanned` | `rig_lifecycle.rs` | Looms discovered from existing rig dir |
| 3 | `app_starts_and_serves_health` | `rig_lifecycle.rs` | Health endpoint returns 200 |
| 4 | `app_loads_rig_agent_config` | `rig_lifecycle.rs` | `/config/rig` returns defaults |
| 5 | `api_register_then_discover_after_restart` | `rig_lifecycle.rs` | Persistence across restart |
| 6 | `build_app_context_wires_layers` | `composition.rs` | Hex layers wired correctly (no HTTP) |
| 7 | `startup_discovers_looms` | `discovery.rs` | Looms + knots discovered at startup |
| 8 | `discovery_ignores_non_loom_directories` | `discovery.rs` | Non-`-loom` dirs ignored |
| 9 | `startup_starts_watchers` | `discovery.rs` | File watcher active, server survives file creation |
| 10 | `startup_logs_knot_registration` | `discovery.rs` | `.loom-log` contains KnotRegistered + LoomStarted |
| 11 | `event_flows_through_pipeline` | `pipeline.rs` | Notify → debounce → ProcessStrand → tie-off |
| 12 | `debounce_prevents_duplicate_processing` | `pipeline.rs` | Rapid edits coalesce to one event |
| 13 | `full_pipeline_create_modify_delete` | `pipeline.rs` | Create → modify → delete strand lifecycle |
| 14 | `full_pipeline_subdirectory_rig` | `pipeline.rs` | Pipeline with subdir rig |
| 15 | `full_pipeline_external_source_with_mock_agent_success` | `pipeline.rs` | Happy path with external dirs |
| 16 | `full_pipeline_http_observable` | `pipeline.rs` | Same lifecycle, observable via HTTP |
| 17 | `full_pipeline_with_pi_agent` | `agent_integration.rs` | Stub `pi` CLI receives correct args |
| 18 | `pi_agent_receives_system_prompt_and_strand` | `agent_integration.rs` | Nonexistent model → failed, error logged |
| 19 | `full_pipeline_agent_error_in_state_and_log` | `agent_integration.rs` | Nonexistent CLI → knot-state `failed`, loom-log error |
| 20 | `full_pipeline_external_source_with_agent_error` | `agent_integration.rs` | Same error scenario with external dirs |
| 21 | `full_tie_off_history` | `tie_off.rs` | Append mode: modify → modify → delete, sections grow |
| 22 | `tie_off_sections_readable` | `tie_off.rs` | Parse tie-off markdown, verify section structure |
| 23 | `http_register_then_process_strand` | `loom_crud.rs` | POST /looms → process strand |
| 24 | `discover_then_process_strand` | `loom_crud.rs` | Discover endpoint |
| 25 | `unregister_stops_processing` | `loom_crud.rs` | DELETE /looms/:id → no processing |
| 26 | `graceful_shutdown_stops_watchers` | `shutdown.rs` | No processing after shutdown signal |
| 27 | `shutdown_logs_loom_stopped` | `shutdown.rs` | `.loom-log` contains LoomStopped |
| 28 | `multiple_looms_independent` | `multi_loom.rs` | Two looms, no cross-interference |
| 29 | `server_starts_with_per_knot_source_dirs` | `multi_loom.rs` | Two knots, separate source dirs |
| 30 | `demo_knot_test_processes_sample_document` | `demo.rs` | Demo happy path |
| 31 | `demo_knot_test_with_tools` | `demo.rs` | Demo with tools configured |

## Test Gaps

- No test for **invalid knot definition** (malformed YAML frontmatter) discovery rejection
- No test for **concurrent strand creation** (race conditions in pipeline)
- No test for **large strand files** (memory/buffer limits)
- No test for **knot config hot-reload** (edit knot file while server runs)
- `full_pipeline_http_observable` (#16) and `full_pipeline_create_modify_delete` (#13) test the same lifecycle — #16 adds HTTP assertions but could be merged as a single test with clear HTTP vs. filesystem assertion sections

## Target Module Structure

```
tests/
├── helpers.rs                        # Shared test infrastructure (NEW)
├── rig_lifecycle.rs                  # Rig directory lifecycle (NEW)
├── composition.rs                    # Composition root wiring (NEW)
├── discovery.rs                      # Loom discovery and filtering (NEW)
├── pipeline.rs                       # Event pipeline and debounce (NEW)
├── agent_integration.rs              # Agent execution: mock, pi stub, errors (NEW)
├── tie_off.rs                        # Tie-off lifecycle and parsing (NEW)
├── loom_crud.rs                      # HTTP loom CRUD (NEW)
├── shutdown.rs                       # Graceful shutdown (NEW)
├── multi_loom.rs                     # Multi-loom isolation + per-knot source dirs (NEW)
├── demo.rs                           # Demo workflow verification (NEW)
├── http_interface.rs                 # (unchanged) HTTP handler unit tests
├── filesystem_interface.rs           # (unchanged) Filesystem unit tests
├── swagger_ui.rs                     # (unchanged) Swagger UI + OpenAPI spec
├── skill_integration.rs              # (unchanged) Skill file + API contract tests
└── integration.rs                    # (removed at end)
```

### Shared Infrastructure (`tests/helpers.rs`)

Extract into `tests/helpers.rs`:

| Item | Current Location | Purpose |
|------|-----------------|---------|
| `make_knot_content_with_dirs()` | top of `integration.rs` | Create valid knot YAML + dirs |
| `make_knot_content()` | top of `integration.rs` | Create knot YAML without dir side-effects |
| `create_mock_agent()` | top of `integration.rs` | Create shell script mock agent |
| `create_stub_pi_agent()` | top of `integration.rs` | Create stub `pi` CLI script |
| `http_get()` | helpers section | Raw TCP HTTP GET |
| `http_get_retry()` | helpers section | HTTP GET with retry |
| `http_post_json()` | bottom of `integration.rs` | Raw TCP HTTP POST with JSON body |
| `http_delete()` | bottom of `integration.rs` | Raw TCP HTTP DELETE |
| `wait_for_port()` | helpers section | TCP port readiness |
| `spawn_server()` | helpers section | Spawn server in background thread |
| `poll_knot_status()` | Phase 2 section | Poll knot endpoint for terminal state |

### Consolidation Plan (post-extraction)

After all tests are moved to feature modules, consolidate overlapping tests within those modules:

| Module | Merge | Rationale |
|--------|-------|-----------|
| `pipeline.rs` | `full_pipeline_http_observable` → `full_pipeline_create_modify_delete` | Same lifecycle; add HTTP assertions to the same test |
| `pipeline.rs` | `full_pipeline_subdirectory_rig` → `full_pipeline_create_modify_delete` | Same flow; rig layout is a config variant |
| `pipeline.rs` | `full_pipeline_external_source_with_mock_agent_success` → `full_pipeline_create_modify_delete` | Same flow; external dirs is a config variant |
| `agent_integration.rs` | `full_pipeline_external_source_with_agent_error` → `full_pipeline_agent_error_in_state_and_log` | Same error; external dirs is a config variant |

This reduces ~31 tests to ~27. If any merged test exceeds ~120 lines, split into parameterised sub-tests rather than keeping separate tests.

## Phases

Each phase moves **one feature module** from `integration.rs` to its own file. The pattern for every phase is the same:

1. Create new test file, copy tests + any needed helpers
2. Run `cargo test --test <module>` — verify the new file passes
3. Remove the moved tests from `integration.rs`
4. Run `cargo test --test integration` — verify remaining tests still pass
5. Commit with message: `test: extract <module> from integration.rs`

This ensures the full suite passes at every step and any regression is isolated to a single module.

### Phase 0: Extract shared helpers

**Goal:** Move all shared infrastructure out of `integration.rs` so every future module can import from `tests/helpers.rs`.

- [x] Create `tests/helpers.rs` containing: `make_knot_content_with_dirs`, `make_knot_content`, `create_mock_agent`, `create_stub_pi_agent`, `http_get`, `http_get_retry`, `http_post_json`, `http_delete`, `wait_for_port`, `spawn_server`, `poll_knot_status`
- [x] Update `integration.rs` to `mod helpers;` and use `helpers::*` — no logic changes
- [x] `cargo test --test integration` — all 31 tests still pass
- [x] Commit: `test: extract shared helpers from integration.rs`

### Phase 1: Extract `rig_lifecycle.rs` (5 tests)

**Goal:** Move rig directory lifecycle and server bootstrap tests.

- [x] Create `tests/rig_lifecycle.rs` with `mod helpers; use helpers::*;`
- [x] Copy from `integration.rs`: `rig_directory_auto_created`, `rig_directory_scanned`, `app_starts_and_serves_health`, `app_loads_rig_agent_config`, `api_register_then_discover_after_restart`
- [x] `cargo test --test rig_lifecycle` — 5 tests pass
- [x] Remove those 5 tests from `integration.rs`
- [x] `cargo test --test integration` — remaining 26 tests pass
- [x] Commit: `test: extract rig_lifecycle from integration.rs`

### Phase 2: Extract `composition.rs` (1 test)

**Goal:** Move composition root wiring test (only non-HTTP test).

- [x] Create `tests/composition.rs`
- [x] Copy from `integration.rs`: `build_app_context_wires_layers`
- [x] `cargo test --test composition` — 1 test passes
- [x] Remove that test from `integration.rs`
- [x] `cargo test --test integration` — remaining 25 tests pass
- [x] Commit: `test: extract composition from integration.rs`

### Phase 3: Extract `discovery.rs` (4 tests)

**Goal:** Move loom discovery, filtering, watcher boot, and registration logging.

- [ ] Create `tests/discovery.rs` with `mod helpers; use helpers::*;`
- [ ] Copy from `integration.rs`: `startup_discovers_looms`, `discovery_ignores_non_loom_directories`, `startup_starts_watchers`, `startup_logs_knot_registration`
- [ ] `cargo test --test discovery` — 4 tests pass
- [ ] Remove those 4 tests from `integration.rs`
- [ ] `cargo test --test integration` — remaining 21 tests pass
- [ ] Commit: `test: extract discovery from integration.rs`

### Phase 4: Extract `pipeline.rs` (6 tests)

**Goal:** Move event pipeline, debounce, and strand lifecycle tests.

- [ ] Create `tests/pipeline.rs` with `mod helpers; use helpers::*;`
- [ ] Copy from `integration.rs`: `event_flows_through_pipeline`, `debounce_prevents_duplicate_processing`, `full_pipeline_create_modify_delete`, `full_pipeline_subdirectory_rig`, `full_pipeline_external_source_with_mock_agent_success`, `full_pipeline_http_observable`
- [ ] `cargo test --test pipeline` — 6 tests pass
- [ ] Remove those 6 tests from `integration.rs`
- [ ] `cargo test --test integration` — remaining 15 tests pass
- [ ] Commit: `test: extract pipeline from integration.rs`

### Phase 5: Extract `agent_integration.rs` (4 tests)

**Goal:** Move agent execution tests (mock agent, pi stub, error paths).

- [ ] Create `tests/agent_integration.rs` with `mod helpers; use helpers::*;`
- [ ] Copy from `integration.rs`: `full_pipeline_with_pi_agent`, `pi_agent_receives_system_prompt_and_strand`, `full_pipeline_agent_error_in_state_and_log`, `full_pipeline_external_source_with_agent_error`
- [ ] `cargo test --test agent_integration` — 4 tests pass
- [ ] Remove those 4 tests from `integration.rs`
- [ ] `cargo test --test integration` — remaining 11 tests pass
- [ ] Commit: `test: extract agent_integration from integration.rs`

### Phase 6: Extract `tie_off.rs` (2 tests)

**Goal:** Move tie-off lifecycle and section parsing tests.

- [ ] Create `tests/tie_off.rs` with `mod helpers; use helpers::*;`
- [ ] Copy from `integration.rs`: `full_tie_off_history`, `tie_off_sections_readable`
- [ ] `cargo test --test tie_off` — 2 tests pass
- [ ] Remove those 2 tests from `integration.rs`
- [ ] `cargo test --test integration` — remaining 9 tests pass
- [ ] Commit: `test: extract tie_off from integration.rs`

### Phase 7: Extract `loom_crud.rs` (3 tests)

**Goal:** Move HTTP loom CRUD tests.

- [ ] Create `tests/loom_crud.rs` with `mod helpers; use helpers::*;`
- [ ] Copy from `integration.rs`: `http_register_then_process_strand`, `discover_then_process_strand`, `unregister_stops_processing`
- [ ] `cargo test --test loom_crud` — 3 tests pass
- [ ] Remove those 3 tests from `integration.rs`
- [ ] `cargo test --test integration` — remaining 6 tests pass
- [ ] Commit: `test: extract loom_crud from integration.rs`

### Phase 8: Extract `shutdown.rs` (2 tests)

**Goal:** Move graceful shutdown tests.

- [ ] Create `tests/shutdown.rs` with `mod helpers; use helpers::*;`
- [ ] Copy from `integration.rs`: `graceful_shutdown_stops_watchers`, `shutdown_logs_loom_stopped`
- [ ] `cargo test --test shutdown` — 2 tests pass
- [ ] Remove those 2 tests from `integration.rs`
- [ ] `cargo test --test integration` — remaining 4 tests pass
- [ ] Commit: `test: extract shutdown from integration.rs`

### Phase 9: Extract `multi_loom.rs` (2 tests)

**Goal:** Move multi-loom isolation and per-knot source directory tests.

- [ ] Create `tests/multi_loom.rs` with `mod helpers; use helpers::*;`
- [ ] Copy from `integration.rs`: `multiple_looms_independent`, `server_starts_with_per_knot_source_dirs`
- [ ] `cargo test --test multi_loom` — 2 tests pass
- [ ] Remove those 2 tests from `integration.rs`
- [ ] `cargo test --test integration` — remaining 2 tests pass
- [ ] Commit: `test: extract multi_loom from integration.rs`

### Phase 10: Extract `demo.rs` (2 tests)

**Goal:** Move demo workflow verification tests.

- [ ] Create `tests/demo.rs` with `mod helpers; use helpers::*;`
- [ ] Copy from `integration.rs`: `demo_knot_test_processes_sample_document`, `demo_knot_test_with_tools`
- [ ] `cargo test --test demo` — 2 tests pass
- [ ] Remove those 2 tests from `integration.rs`
- [ ] `cargo test --test integration` — `integration.rs` is now empty (only `mod helpers;` remaining)
- [ ] Commit: `test: extract demo from integration.rs`

### Phase 11: Consolidate pipeline tests

**Goal:** Merge overlapping pipeline variants in `pipeline.rs` into fewer, clearer tests.

- [ ] In `pipeline.rs`, merge `full_pipeline_http_observable` into `full_pipeline_create_modify_delete` — add HTTP assertion block to the existing test
- [ ] In `pipeline.rs`, merge `full_pipeline_subdirectory_rig` as a section of `full_pipeline_create_modify_delete` or rename to `full_pipeline_with_subdirectory_rig` keeping its own assertions
- [ ] In `pipeline.rs`, merge `full_pipeline_external_source_with_mock_agent_success` into the main pipeline test as a section or rename to `full_pipeline_with_external_dirs`
- [ ] `cargo test --test pipeline` — merged tests pass (expect ~3 tests instead of 6)
- [ ] Commit: `test: consolidate pipeline tests`

### Phase 12: Consolidate agent integration tests

**Goal:** Merge overlapping agent error variants in `agent_integration.rs`.

- [ ] In `agent_integration.rs`, merge `full_pipeline_external_source_with_agent_error` into `full_pipeline_agent_error_in_state_and_log` (external dirs is a config variant — add a section or parameterise)
- [ ] `cargo test --test agent_integration` — merged tests pass (expect ~3 tests instead of 4)
- [ ] Commit: `test: consolidate agent_integration tests`

### Phase 13: Remove `integration.rs` and verify

**Goal:** Final cleanup — remove the empty file and verify the full suite.

- [ ] Delete `tests/integration.rs` (empty after Phase 10)
- [ ] Run full test suite: `cargo test --test '*'`
- [ ] Verify final test count: expect ~27 tests across 11 integration modules + 4 existing modules (total ~57 integration tests)
- [ ] Verify total test time is not significantly increased (same parallelism)
- [ ] Commit: `test: remove empty integration.rs`

## Notes

- The existing `http_interface.rs`, `filesystem_interface.rs`, `swagger_ui.rs`, and `skill_integration.rs` files are already well-structured and do not need changes.
- Each phase is a safe, reversible commit. If a test fails after extraction, the issue is isolated to that one module — rollback is a single `git revert`.
- The consolidation phases (11–12) come after all extraction is done, so the merge work is contained within a single module rather than spread across the extraction process.
