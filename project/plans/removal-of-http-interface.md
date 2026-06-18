# Plan: Removal of HTTP Interface — Full File-First

## Problem

Knot currently exposes an HTTP interface (Axum) for observability — listing looms, knots, profiles, and rig config via REST endpoints. This adds framework dependency (`axum`, `utoipa`, `utoipa-swagger-ui`, `tower`) and test infrastructure (raw TCP HTTP helpers, server spawning) to a project that is already file-first for configuration and control. The single `POST /config/reload` endpoint is the only write operation and can be removed entirely since the file watcher handles discovery.

The HTTP interface is a half-measure: all configuration is file-driven, but all *reading* state requires an HTTP client. External consumers (skills, scripts, other tools) should just read a file, not talk HTTP.

## Target

- No HTTP server. No `axum`, `utoipa`, `utoipa-swagger-ui`, or `tower` dependencies.
- All runtime state written to `rig/state.json` on a 5-second poll cycle.
- Skills read `rig/state.json` instead of calling HTTP endpoints.
- All HTTP-related test files and helpers removed or rewritten to use file-based verification.
- The binary is lighter, simpler, and truly file-first end-to-end.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test File | What it covers | HTTP Dependency |
|-----------|---------------|-----------------|
| `tests/http_interface.rs` (3) | Health, list_agents endpoints | ✅ Entirely HTTP — remove |
| `tests/swagger_ui.rs` (2) | Swagger UI + OpenAPI spec endpoints | ✅ Entirely HTTP — remove |
| `tests/axum_server_test_integration.rs` (1) | Axum server spawn pattern reference | ✅ Entirely HTTP — remove |
| `tests/skill_integration.rs` (10) | Skills + API endpoint verification | ✅ HTTP endpoints — rewrite |
| `tests/helpers.rs` | Shared infrastructure (HTTP GET/POST, spawn_server, poll helpers) | ✅ HTTP helpers — remove/replace |
| `tests/discovery.rs` (4) | Loom discovery via GET /looms | ✅ Rewrite to file poll |
| `tests/pipeline.rs` (7) | Strand processing, debounce, activity log | ✅ Uses `poll_knot_status`, `http_get_retry` |
| `tests/agent_integration.rs` (10) | Agent execution, error handling | ✅ Uses `http_get` for verification |
| `tests/auto_discovery_and_knot_crud.rs` (9) | File-watcher auto-discovery | ✅ Heavy HTTP usage (59 calls) |
| `tests/git_versioning.rs` (3) | Git commit on tie-off | ✅ Uses `spawn_server`, `http_get` |
| `tests/multi_loom.rs` (0) | Multi-loom scenarios | ✅ Uses `spawn_server`, `http_get` |
| `tests/profile_timeout.rs` (0) | Agent timeout handling | ✅ Uses `spawn_server`, `http_get` |
| `tests/rig_lifecycle.rs` (5) | Rig creation, loom scanning, config endpoints | ✅ Uses `spawn_server`, `http_get` |
| `tests/rig_log.rs` (0) | Rig-log event recording | ✅ Uses `spawn_server`, `http_get` |
| `tests/server_startup_smoke.rs` (0) | Server bind/listen smoke tests | ✅ Uses `spawn_server`, `http_get` — remove |
| `tests/shutdown.rs` (2) | Graceful shutdown cascade | ✅ Uses `spawn_server_with_shutdown` — rewrite |
| `tests/skill_e2e.rs` (0) | File-first CRUD + skill workflows | ✅ Uses `spawn_server`, HTTP helpers |
| `tests/tie_off.rs` (0) | Tie-off file output | ✅ Uses `spawn_server` |
| `tests/task_management.rs` (0) | Task management | ✅ Uses `spawn_server` |
| `tests/composition.rs` (0) | Composition root wiring | ⚠️ May reference `start_server` |
| `tests/demo.rs` (0) | Demo workflow | ✅ Uses `spawn_server`, HTTP helpers |
| `tests/rig_cli.rs` (3) | CLI argument parsing | ⚠️ Minimal HTTP usage |
| `tests/filesystem_interface.rs` (0) | Filesystem operations | ❌ No HTTP dependency — keep as-is |
| `tests/generic_task_management.rs` (10) | Channel cascade shutdown pattern | ❌ No HTTP dependency — keep as-is |
| `tests/rig_discovery.rs` (0) | Rig discovery logic | ❌ No HTTP dependency — keep as-is |

## Test Gaps

- No test for the new state writer task (write cycle, atomic write, error handling)
- No test for `rig/state.json` schema and content correctness
- No test for skills reading from `rig/state.json` directly
- File-based polling helpers needed in `tests/helpers.rs` to replace HTTP-based verification

## Phases

### Phase 0: State File Schema and Writer Task

Build the replacement before removing what it replaces. Define the `rig/state.json` format and the background task that writes it.

**Hex Layer:** Domain → Application → Outbound Adapter

- [x] Define `RigState` domain type (JSON-serializable):
  ```json
  {
    "rig_path": "/path/to/rig",
    "looms": [
      {
        "id": "my-loom",
        "knots": [
          {
            "id": "review-knot",
            "status": "idle",
            "last_strand_path": null,
            "last_tie_off_path": null,
            "last_error": null,
            "last_event_at": null
          }
        ]
      }
    ],
    "profiles": [
      {
        "name": "fast",
        "provider": "openai",
        "model": "gpt-4o"
      }
    ],
    "updated_at": "2026-06-18T12:00:00Z"
  }
  ```
- [x] Create `StateWriter` use case (application layer) that:
  - Reads `LoomStore` + `AgentProfileRepository` + loom logs
  - Serialises to `RigState` JSON
  - Writes atomically to `rig/state.json` (write to `.state.json.tmp`, rename)
- [x] Create `StateWriterPort` trait + `FileSystemStateWriter` adapter
- [x] Integrate into `server.rs` as a background `tokio::task` spawned into the JoinSet, polling every 5 seconds
- [x] Wire into composition root (`build_app_context` / `start_event_pipeline` or new `start_state_writer`)
- [x] Tests: unit tests for `RigState` serialisation, atomic write correctness, error on bad permissions
- [x] Tests: integration test — start Knot, create a loom, verify `rig/state.json` updates within poll window

### Phase 1: Remove HTTP Server from Source

Remove the HTTP server and all its infrastructure from the binary.

**Hex Layer:** Inbound Adapter (deletion), Composition Root

- [x] Remove `src/adapters/inbound/` module entirely (`router.rs`, `system.rs`, `loom.rs`, `types.rs`, `mod.rs`)
- [x] Remove from `src/server.rs`:
  - `start_server()` / `start_server_with_shutdown()`
  - `ShutdownSignal` enum
  - `bind_addr` from `AppConfig`
  - `TcpListener` bind, `axum::serve`, graceful shutdown HTTP logic
  - Keep: `build_app_context`, `start_event_pipeline`, `start_config_pipeline`, `run_startup`, `AppConfig` (minus `bind_addr`)
- [x] Rewrite `server.rs` lifecycle:
  - No HTTP serve call — main task now just waits for Ctrl+C
  - Background tasks: event pipeline, config pipeline, state writer
  - On shutdown: drain JoinSet, write `LoomStopped` to loom-logs, exit
- [x] Update `src/lib.rs`: remove all HTTP re-exports (`build_app`, `AppContext`, `health`, `list_agents`, `start_server`, `start_server_with_shutdown`, `ShutdownSignal`). Re-export `start_knot` (renamed from `start_server`) and `AppConfig` (without `bind_addr`).
- [x] Remove from `Cargo.toml`: `axum`, `utoipa`, `utoipa-swagger-ui`, `tower` (dev-dep)
- [x] Update `src/main.rs` — no longer calls `start_server`, calls `start_knot` which blocks on Ctrl+C
- [x] Verify: `cargo build`, `cargo test`, `cargo clippy` all pass

### Phase 2: Remove HTTP Tests, Rewrite Helpers

Remove HTTP test files and replace `tests/helpers.rs` infrastructure.

- [x] Delete: `tests/http_interface.rs`
- [x] Delete: `tests/swagger_ui.rs`
- [x] Delete: `tests/axum_server_test_integration.rs`
- [x] Delete: `tests/server_startup_smoke.rs` (was only testing HTTP bind)
- [x] Rewrite `tests/helpers.rs`:
  - Remove: `http_get`, `http_post_json`, `http_patch_json`, `http_delete`, `read_response`
  - Remove: `wait_for_port`, `spawn_server`, `spawn_server_with_shutdown`
  - Remove: `wait_for_loom_discovery`, `wait_for_knot_count`, `poll_knot_status`
  - Keep: `make_knot_content`, `create_fast_profile`, `create_mock_agent`, `create_stub_pi_agent`, `init_git_repo`, `get_latest_commit`, `count_commits`
  - Add: `start_knot(rig_dir)` — spawns `start_knot()` in a tokio task, returns `(JoinHandle, oneshot::Sender)` for shutdown
  - Add: `wait_for_state_field(path: &str, selector: &str, expected: &str)` — polls `rig/state.json` and checks a JSON path matches expected value
  - Add: `wait_for_loom_in_state(rig_dir, loom_id, expected_knots)` — polls state file for loom registration
  - Add: `wait_for_knot_status_in_state(rig_dir, loom_id, knot_id, status)` — polls state file for knot status
- [x] Verify: `cargo test --test helpers` (or compile check)

### Phase 3: Rewrite Remaining Integration Tests

Convert all remaining integration tests from HTTP verification to file-based polling.

- [x] **`tests/skill_integration.rs`** (10 tests) — rewrite to verify state via `rig/state.json` instead of HTTP endpoints. Verify skill files still exist and reference correct paths.
- [x] **`tests/discovery.rs`** (4 tests) — use `wait_for_loom_in_state` instead of `http_get("/looms")`
- [x] **`tests/pipeline.rs`** (7 tests) — use `wait_for_knot_status_in_state` instead of `poll_knot_status`. Verify activity via loom-log file reads instead of HTTP.
- [x] **`tests/agent_integration.rs`** (10 tests) — verify agent execution results via tie-off files and `rig/state.json`
- [x] **`tests/auto_discovery_and_knot_crud.rs`** (9 tests) — rewrite to use file-based polling helpers. Heavy HTTP usage (59 calls) — largest rewrite.
- [x] **`tests/git_versioning.rs`** (3 tests) — use `start_knot` instead of `spawn_server`, verify via git log + state file
- [x] **`tests/multi_loom.rs`** — use file-based helpers
- [x] **`tests/profile_timeout.rs`** — use file-based helpers
- [x] **`tests/rig_lifecycle.rs`** (5 tests) — verify rig creation and config via filesystem reads and state file
- [x] **`tests/rig_log.rs`** — verify via filesystem reads
- [x] **`tests/shutdown.rs`** (2 tests) — rewrite with `start_knot` + oneshot shutdown, verify LoomStopped in loom-log
- [x] **`tests/skill_e2e.rs`** — rewrite with file-based helpers
- [x] **`tests/tie_off.rs`** — use `start_knot` instead of `spawn_server`
- [x] **`tests/task_management.rs`** — use `start_knot` instead of `spawn_server`
- [x] **`tests/composition.rs`** — update to use new composition root (no `start_server`)
- [x] **`tests/demo.rs`** — use file-based helpers
- [x] **`tests/rig_cli.rs`** (3 tests) — minimal changes, update `AppConfig` usage
- [x] Verify: `cargo test` passes, test count reasonable (should be similar or slightly higher than before)

### Phase 4: Update Agent Skills

Update all `.agents/skills/` files to use `rig/state.json` instead of HTTP endpoints.

- [ ] **`knot-inspect/SKILL.md`** — replace all `GET /looms`, `GET /health`, etc. with "read `rig/state.json`". Update description, API reference, and workflow steps.
- [ ] **`knot-create/SKILL.md`** — replace HTTP verification steps with "read `rig/state.json`". Update description and workflows.
- [ ] **`knot-init/SKILL.md`** — replace "check if Knot is running via `GET /health`" with file-based detection. Update description and workflows.
- [ ] **`project-documentation/SKILL.md`** — update any references to HTTP endpoints in doc generation instructions.
- [ ] Remove `api_spec` frontmatter from all skill files (no more OpenAPI spec URL)
- [ ] Verify skills still match their `USE FOR` / `DO NOT USE FOR` descriptions

### Phase 5: Domain Glossary and Final Verification

- [ ] Update `project/domain-glossary.md` — remove HTTP/API terms, add `rig/state.json` and `State Writer`
- [ ] Verify: `cargo build` passes, `cargo test` passes, `cargo clippy` clean
- [ ] Verify: `cargo run` starts, creates `rig/state.json`, updates on loom creation
- [ ] Verify: no remaining `axum`/`utoipa`/`tower` references in codebase
- [ ] Run integration test suite: `cargo test --test '*'`

## Notes

- The state writer poll cycle (5 seconds) is a pragmatic starting point. Could be tuned or made event-driven later, but polling matches the "file-first" philosophy — external consumers just read a file with known staleness bounds.
- Atomic writes (write-to-tmp + rename) are critical — external processes reading `rig/state.json` should never see a partial write.
- The shutdown sequence changes: no more HTTP graceful shutdown, but the JoinSet drain + LoomStopped logging stays the same. The main task just waits for Ctrl+C instead of `axum::serve` with `with_graceful_shutdown`.
- `AppContext` is an HTTP-bound concept (Axum `State`). It should be renamed or restructured since it no longer serves an HTTP framework. Consider renaming to `KnotContext` or inlining it into the composition root.
- The `POST /config/reload` endpoint is the only write operation. Its functionality (manual discovery recovery) can be replaced by simply writing a loom file — the file watcher will pick it up. If needed, a CLI subcommand like `knot reload` could be added later.
