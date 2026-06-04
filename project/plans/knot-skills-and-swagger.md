# Plan: Knot Skills and Swagger UI

## Related PRD

This plan contributes to [Knot Skills — AI-Driven Configuration via Skills](../prds/prd-knot-skills.md).

This plan delivers all five success criteria from the PRD: the three skills (`knot-init`, `knots-and-looms`, `knot-inspect`), the Swagger UI served by Knot, and the OpenAPI spec generated from code via `utoipa`. The skills use Knot's HTTP interface exclusively and together cover the full configuration lifecycle: init, loom/knot CRUD, and rig inspection.

## Problem

Knot has a working HTTP API but no OpenAPI documentation. There are no AI skills to let a user configure Knot through natural language. A user must manually craft API calls and edit files to set up rigs, looms, and knots.

## Target

When this plan is done:

1. Knot serves a Swagger UI at `/swagger-ui` with an auto-generated OpenAPI spec from `utoipa` annotations.
2. Three skill files exist (`knot-init`, `knots-and-looms`, `knot-inspect`) that instruct an AI agent to configure Knot via its HTTP API.
3. Integration tests verify the skills work end-to-end by calling `pi` in a test directory.

## Implementation Status: ✅ Complete

## Existing Tests

| Test File | What it covers | Status |
|-----------|---------------|--------|
| `tests/http_interface.rs` | HTTP endpoint smoke tests | ✅ Green — verifies endpoint shapes |
| `tests/integration.rs` | Full integration: loom CRUD, processing, state | ✅ Green — 2188 lines, comprehensive |
| `tests/filesystem_interface.rs` | Filesystem operations | ✅ Green — basic file IO checks |
| `src/adapters/inbound/mod.rs` (module tests) | Route wiring, handler dispatch | ✅ Green — unit tests for all handlers |

## Test Gaps

- No tests for OpenAPI/Swagger — no `utoipa` annotations to derive from yet
- No skill integration tests — no skills exist yet
- No test that validates the API contract matches what a skill expects (Swagger is the bridge)

## Phases

### Phase 1: OpenAPI/Swagger Integration

Add `utoipa` and `utoipa-swagger-ui` to Knot so the API is self-documenting.

- [x] Add `utoipa` and `utoipa-swagger-ui` dependencies to `Cargo.toml`
- [x] Annotate all existing request/response types with `#[derive(utoipa::ToSchema)]` (e.g. `RigAgentConfig`, `Loom`, `KnotId`, `KnotState`, loom list DTOs)
- [x] Annotate all handler functions with `#[utoipa::path]` covering the 12 existing routes (`/health`, `/agents/{dir}`, `/config/rig`, `/looms`, `/looms/discover`, `/looms/{id}`, `/looms/{id}/activity`, `/looms/{id}/knots`, `/looms/{id}/knots/{knot_name}`)
- [x] Wire `utoipa-swagger-ui` into the router to serve Swagger UI at `/swagger-ui`
- [x] Add a test that `GET /swagger-ui` returns 200 and `GET /swagger-ui/openapi.json` returns valid OpenAPI JSON
- [x] Compile and run full test suite — all existing tests still pass

### Phase 2: Skills and Integration Tests

Create the three skill files and verify them by calling `pi` in a test directory.

- [x] Create skill file `knot-init` — instructs agent to: detect if rig exists, start Knot if not running, create rig directory structure, verify via `GET /config/rig`
- [x] Create skill file `knots-and-looms` — instructs agent to: create/modify/delete looms and knots via `POST /looms`, `DELETE /looms/{id}`, and loom config file manipulation
- [x] Create skill file `knot-inspect` — instructs agent to: inspect rig state via `GET /config/rig`, `GET /looms`, `GET /looms/{id}`, `GET /looms/{id}/activity`, `GET /looms/{id}/knots`, `GET /looms/{id}/knots/{knot_name}`
- [x] Each skill references the OpenAPI spec URL (`http://localhost:3000/swagger-ui/openapi.json`) so the agent can validate API shapes before calling
- [x] Create integration test harness:
  - Test directory: `~/workspace/ai/knot-test-skill`
  - Start Knot server in background (via `cargo run` or binary)
  - Execute `pi` in the test directory with `--skill knot-init` (and similar for other skills)
  - Capture `pi` output, verify expected API calls were made (e.g. rig config created, loom registered)
  - Kill Knot process after each test
- [x] Add the integration test as a new test file (e.g. `tests/skill_integration.rs`) or within `tests/integration.rs`
- [x] Run full test suite — all tests pass

## Notes

- The skills themselves are instruction files read by `pi` — they are not Rust code. Testing them is meta: we call `pi` (which reads the skill) and verify the resulting HTTP calls to Knot.
- The test directory `~/workspace/ai/knot-test-skill` is an isolated workspace — not the Knot project itself. This avoids polluting the Knot rig during tests.
- `pi` is available on the global path at `/home/hayden/.nvm/versions/node/v26.2.0/bin/pi`.
- Skills interact with Knot only via HTTP — no direct file access. This matches the existing constraint that Knot manages its own files.
