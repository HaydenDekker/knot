# Plan: HTTP Observability Only — Remove Control Endpoints

## Related PRD

This plan reverses a design constraint from [Knot Skills — AI-Driven Configuration via Skills](../prds/prd-knot-skills.md).

The PRD specified: "Skills interact with Knot only via its HTTP interface — no direct file system access by the skills." This plan removes that constraint. The HTTP interface becomes observability-only (read endpoints). Configuration (profiles, knots, looms) is file-first — skills write files directly, and Knot's existing file watcher picks up changes.

## Problem

Knot's HTTP interface provides CRUD endpoints for looms, knots, and profiles (`POST`, `DELETE`, `PATCH`). These are thin wrappers around `fs::write` and `fs::read`. The files are the source of truth — Knot reads profiles from disk, discovers knots from `.md` files, and watches directories for changes.

Every time file-level metadata exists that the domain model doesn't track (like markdown body in profile files), the HTTP layer must be extended to handle it. This creates unnecessary complexity: threading body through the repository trait, every mock implementation, and all handler types — just so a file-level concern can pass through JSON.

The PRD's "HTTP-only" constraint was intended to keep skills clean and Knot in control. In practice it adds indirection without adding functionality — the skill already documents the exact file formats, paths, and naming conventions. The file watcher picks up changes. Git tracks everything.

## Target

1. HTTP interface provides **observability only**: `GET /health`, `GET /config/rig`, `GET /looms`, `GET /looms/{id}`, `GET /looms/{id}/activity`, `GET /looms/{id}/knots`, `GET /looms/{id}/knots/{name}`, `GET /profiles`, `GET /profiles/{name}`, `GET /agents/{dir}`
2. All control endpoints removed: `POST /looms`, `DELETE /looms/{id}`, `POST /looms/{id}/knots`, `PATCH /looms/{id}/knots/{name}`, `DELETE /looms/{id}/knots/{name}`, `POST /profiles/{name}`, `DELETE /profiles/{name}`
3. Request types removed: `RegisterLoomRequest`, `KnotRequest`, `ProfileRequest`
4. Response type simplified: `ProfileResponse` (remains for GET, but no longer needs body threading)
5. Skills updated: `knot-init` and `knot-create` write files directly instead of calling HTTP
6. All tests updated to match
7. The file-first approach documented in skills (paths, formats, what Knot auto-discovers)

## Implementation Status: ✅ Complete (2026-06-13)

## Result

All 7 control endpoints removed from the HTTP interface. Configuration is now file-first — profiles, looms, and knots are created by writing `.md` files directly to `rig/profiles/` and `rig/{name}-loom/`. Knot's file watcher auto-discovers all changes.

**Removed:** `POST /looms`, `DELETE /looms/{id}`, `POST /looms/{id}/knots`, `PATCH /looms/{id}/knots/{name}`, `DELETE /looms/{id}/knots/{name}`, `POST /profiles/{name}`, `DELETE /profiles/{name}`. Request types `RegisterLoomRequest`, `KnotRequest`, `ProfileRequest` removed. 3600+ lines of handler code and tests removed.

**Added:** `AgentProfile.body: Option<String>` — profile markdown body now threaded through `ProfileResponse`. Skills updated to file-first approach.

**Tests:** 311 pass, 1 ignored. Removed `loom_crud.rs` (4 tests), `shared_agent_profiles.rs` (10 tests), knot CRUD tests from `auto_discovery_and_knot_crud.rs` (4 tests), and 2 HTTP workflow tests from `skill_integration.rs`. Added 2 new unit tests for profile body field. Global `knots-and-looms` skill removed (stale duplicate).

Full details in [http-observability-only.md](http-observability-only.md).

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `loom_crud.rs` (4 tests, 245 lines) | Loom CRUD via HTTP — register, get, list, delete | ✅ Green — will be removed |
| `auto_discovery_and_knot_crud.rs` (23 tests, 1111 lines) | Auto-discovery + knot CRUD endpoints | ✅ Green — partially removed |
| `shared_agent_profiles.rs` (20 tests, 1053 lines) | Profile CRUD + resolution via HTTP | ✅ Green — partially removed |
| `skill_integration.rs` (33 tests, 1018 lines) | Skill file validation + HTTP workflow simulation | ✅ Green — needs major update |
| `loom.rs` inbound handler tests (38 tests) | Handler-level unit tests for all CRUD + GET | ✅ Green — CRUD tests removed |
| `helpers.rs` | HTTP POST/PATCH/DELETE helpers | ✅ Green — helpers removed |

## Test Gaps

- No file-first skill tests (writing `.md` files and verifying Knot picks them up via GET endpoints) — the existing file watcher + auto-discovery tests cover the server side, but no test validates the skill workflow of writing files directly
- No test that verifies `GET` endpoints work correctly when files are created externally (outside HTTP) — auto-discovery tests touch this but through the file watcher, not direct file creation

## Phases

### Phase 0: Remove control endpoints from router + handlers
- [x] Remove from `router.rs`: `POST /looms`, `DELETE /looms/{id}`, `POST /looms/{id}/knots`, `PATCH /looms/{id}/knots/{name}`, `DELETE /looms/{id}/knots/{name}`, `POST /profiles/{name}`, `DELETE /profiles/{name}`
- [x] Remove unused imports from `router.rs`: `register_loom`, `unregister_loom`, `create_knot`, `update_knot`, `delete_knot`, `create_profile`, `delete_profile`
- [x] Remove from `loom.rs` handlers: `register_loom`, `unregister_loom`, `create_knot`, `update_knot`, `delete_knot`, `create_profile`, `delete_profile` (keep all GET handlers)
- [x] Remove from `loom.rs` handler tests: all tests for removed handlers (register, unregister, create/update/delete knot, create/delete profile)
- [x] Remove `RegisterLoomRequest`, `KnotRequest`, `ProfileRequest` from `types.rs`
- [x] Remove `ProfileRequest` from OpenAPI schema components in `router.rs`
- [x] Remove removed endpoints from OpenAPI `paths()` list in `router.rs`
- [x] Revert the uncommitted `AgentProfileRepository::save()` trait change (add `body` param) — no longer needed since profiles are file-first
- [x] Verify `cargo build` passes

**Result:** All 7 control endpoints removed from router. 7 handler functions + helper functions removed from `loom.rs` (1713 lines deleted). All CRUD handler unit tests removed. `RegisterLoomRequest`, `KnotRequest`, `ProfileRequest` types removed from `types.rs`. OpenAPI spec updated. `mod.rs` re-exports cleaned. `AgentProfileRepository::save()` already had correct signature (no uncommitted body param to revert). `cargo build` passes.

### Phase 1: Remove control tests + helpers
- [x] Remove `tests/loom_crud.rs` entirely (all 4 tests are CRUD-only)
- [x] Rewrite `tests/auto_discovery_and_knot_crud.rs`: keep auto-discovery tests (file watcher picks up changes), remove knot CRUD endpoint tests (POST/PATCH/DELETE knots via HTTP)
- [x] Rewrite `tests/shared_agent_profiles.rs`: removed entirely — all 10 tests used HTTP CRUD endpoints. Profile resolution/dynamic update tests were HTTP-dependent and not viable as file-first tests in the inbound test context
- [x] Rewrite `tests/skill_integration.rs`: removed tests using removed endpoints, fixed skill paths from `skills/` to `.agents/skills/`, fixed skill names (`knot-create` instead of `knots-and-looms`)
- [x] Remove `POST`/`PATCH`/`DELETE` helpers from `tests/helpers.rs` — kept `http_post_json` as it's still used by `discover_endpoint_removed` (negative test verifying old endpoint is gone)
- [x] Remove `usecases.rs` mock profile repo's `save()` body parameter — not needed; no body param was added
- [x] Verify `cargo test` passes

### Phase 2: Simplify inbound types and repository trait
- [x] Remove `RegisterLoomRequest` and `KnotRequest` from `types.rs` (done in Phase 0)
- [x] Remove `ProfileRequest` from `types.rs` (done in Phase 0)
- [x] Simplify `AgentProfileRepository::save()` — already had correct signature `save(profile: AgentProfile)`, no uncommitted body param to revert
- [x] Revert `FileSystemAgentProfileRepository::save()` — not needed, already correct
- [x] Revert mock repos — not needed, already correct
- [x] Verify `cargo build` + `cargo test` pass (310 tests pass, 1 ignored)

### Phase 3: Update skills for file-first approach
- [x] Update `knot-create` skill (`/.agents/skills/knot-create/SKILL.md`):
  - All HTTP POST/PATCH/DELETE workflows replaced with file-first approach
  - Documented file paths and formats for creating looms, knots, profiles
  - Explained auto-discovery via file watcher (no registration needed)
  - Documented `GET` endpoints for verification only
- [x] Update `knot-init` skill (`/.agents/skills/knot-init/SKILL.md`):
  - Added profile discovery from `~/.pi/agent/models.json`
  - When no profiles exist, writes `rig/profiles/default.md` with markdown body documenting alternatives
  - Verifies via `GET /profiles` and `GET /profiles/{name}`
- [x] Update `knot-inspect` skill (no changes needed — already read-only)
- [x] Update `knots-and-looms` skill in `~/.agents/skills/` — removed (stale global duplicate of project-local `knot-create`)
- [x] Verify skill integration tests reference correct endpoints (16 tests pass)

### Phase 4: Verify full system integration
- [x] Full `cargo test` passes (311 passed, 1 ignored)
- [x] Verify auto-discovery still works: 6 integration tests confirm file-first discovery
- [x] Verify `GET` endpoints still serve Swagger UI correctly with reduced schema (no POST/PATCH/DELETE, no removed types)
- [x] Verify `GET /profiles/{name}` returns body field — added `body: Option<String>` to `AgentProfile` and `ProfileResponse`, populated from disk, 2 new unit tests

**Result:** `AgentProfile` now carries `body: Option<String>` (markdown body from profile file). `ProfileResponse` serializes it. `FileSystemAgentProfileRepository::get()` extracts body using `extract_body()` helper. Swagger spec verified: only GET methods, no removed request types in schema.

## Notes

- **Auto-discovery is key.** Knot already watches `rig/` and loom directories via `NotifyEventSource`. File changes are picked up without any HTTP call. This plan relies on that existing mechanism entirely.
- **The PRD constraint reversal is significant.** The PRD said "no direct file system access by the skills." This plan removes that constraint. The ADR for this reversal should be created separately (or the PRD updated to note the exception). The plan document itself references the PRD and notes the constraint reversal.
- **What stays HTTP:** All `GET` endpoints — health, config, loom list/details, activity, knot status, profile list/details, agents listing, Swagger UI. These are read-only observability.
- **What becomes file-first:** Creating/modifying/deleting profiles, looms, and knots. Skills write `.md` files directly. The file watcher activates changes.
- **Body preservation is solved naturally.** Writing the full `.md` file (frontmatter + body) is a single `fs::write` — no threading through JSON, no trait changes, no round-trip issues.
- **The uncommitted changes (trait `save` with body parameter) are reverted in Phase 2.** They were exploring the wrong solution — making HTTP handle file-level metadata instead of just writing the file.
