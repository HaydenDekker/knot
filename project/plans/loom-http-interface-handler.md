# Plan: Loom HTTP Interface

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan implements the inbound HTTP adapter — axum handlers and routes that call use cases from the application layer. Handlers never touch adapters directly; they delegate to use cases, which delegate to ports.

## Problem

Knot has domain types (Plan 1), application use cases (Plan 2), and outbound adapters (Plan 3), but no inbound adapter. Users cannot interact with the system via HTTP. There are no endpoints to list looms, check knot status, query activity, or register new looms. Without an inbound adapter, the system is a black box.

## Target

- HTTP handlers for all loom/knot operations (list, get, register, unregister, activity, status)
- Handlers call application-layer use cases — never touch ports or adapters directly
- Axum routes wired through `Router` with `State` extractors
- All endpoint responses sourced from use cases (which read from port-backed state files)
- Existing routes (`/health`, `/agents/{dir}`) remain functional

## Implementation Status: ⬜ Draft

## Hex Layer: Inbound Adapter

HTTP handlers are inbound adapters. They depend on use cases (application layer), never on ports or outbound adapters directly.

## Proposed Endpoints

| Method | Path | Use Case Called |
|--------|------|-----------------|
| `GET` | `/looms` | `ListLooms` |
| `GET` | `/looms/:id` | `GetLoom` |
| `GET` | `/looms/:id/activity` | `GetLoomActivity` |
| `GET` | `/looms/:id/knots` | derived from `GetLoom` response |
| `GET` | `/looms/:id/knots/:knot_name` | `GetKnotStatus` |
| `POST` | `/looms` | `RegisterLoom` |
| `DELETE` | `/looms/:id` | `UnregisterLoom` |

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |
| `domain::*` tests | Domain entities, value objects, events | Plan 1 |
| `application::*` tests | Use cases with mock ports | Plan 2 |
| `adapters::*` tests | Outbound adapter IO | Plan 3 |

## Test Gaps

- No HTTP handler tests for loom endpoints.
- No tests verify handlers call correct use cases.
- No tests for HTTP error responses (400, 404, 409).
- No tests for route wiring (correct paths, methods).
- No end-to-end test: HTTP → use case → adapter → disk → HTTP response.

## Phases

### Phase 0: Handler Module and State Wiring
**Failing tests created:** `adapters::inbound::tests::handlers_module_compiles`, `adapters::inbound::tests::state_extractor_available`

- [ ] Failing test: `adapters::inbound::tests::handlers_module_compiles` — `src/adapters/inbound/mod.rs` exists with handler stubs; `build_app()` compiles
- [ ] Failing test: `adapters::inbound::tests::state_extractor_available` — `Router` can be built with `State` containing use case dependencies (`LoomStore`, ports)
- [ ] Create `src/adapters/inbound/mod.rs` with handler stubs
- [ ] Define `AppContext` struct holding `LoomStore`, port instances, debounce engine sender
- [ ] Update `build_app()` to accept `AppContext` and wire routes
- [ ] **Alert:** handlers depend on `LoomStore` (application layer) and use cases — correct hex direction, adapters depend inward

### Phase 1: Loom Listing and Details Handlers
**Failing tests created:** `adapters::inbound::tests::get_looms_returns_json`, `adapters::inbound::tests::get_looms_empty`, `adapters::inbound::tests::get_loom_by_id`, `adapters::inbound::tests::get_loom_not_found`, `adapters::inbound::tests::get_loom_knots`

- [ ] Failing test: `adapters::inbound::tests::get_looms_returns_json` — `GET /looms` returns `200` with JSON array of loom summaries
- [ ] Failing test: `adapters::inbound::tests::get_looms_empty` — no looms registered; returns `200` with empty array `[]`
- [ ] Failing test: `adapters::inbound::tests::get_loom_by_id` — `GET /looms/:id` for registered loom returns `200` with loom details
- [ ] Failing test: `adapters::inbound::tests::get_loom_not_found` — `GET /looms/:id` for unknown ID returns `404`
- [ ] Failing test: `adapters::inbound::tests::get_loom_knots` — `GET /looms/:id/knots` returns list of knot names from loom model
- [ ] Implement `list_looms`, `get_loom`, `list_knots` handlers
- [ ] Handlers call `ListLooms`, `GetLoom` use cases via `State<AppContext>`
- [ ] Wire routes into `build_app()` with axum `Router`
- [ ] Tests use `tower::test::TestClient` with in-memory `LoomStore`

### Phase 2: Activity and Knot-State Handlers
**Failing tests created:** `adapters::inbound::tests::get_loom_activity`, `adapters::inbound::tests::get_knot_status`, `adapters::inbound::tests::get_knot_status_not_found`

- [ ] Failing test: `adapters::inbound::tests::get_loom_activity` — `GET /looms/:id/activity` returns `200` with JSON array of loom-log entries
- [ ] Failing test: `adapters::inbound::tests::get_knot_status` — `GET /looms/:id/knots/:knot_name` returns `200` with knot-state JSON
- [ ] Failing test: `adapters::inbound::tests::get_knot_status_not_found` — unknown knot name returns `404`
- [ ] Implement `get_loom_activity`, `get_knot_status` handlers
- [ ] `get_loom_activity` calls `GetLoomActivity` use case (which reads `LoomLogPort`)
- [ ] `get_knot_status` calls `GetKnotStatus` use case (which reads `KnotStatePort`)
- [ ] **Alert:** handlers call use cases which call ports — correct hex layering. Handlers never call ports directly.

### Phase 3: Loom Management Handlers
**Failing tests created:** `adapters::inbound::tests::post_loom_success`, `adapters::inbound::tests::post_loom_missing_source_dir`, `adapters::inbound::tests::post_loom_duplicate_id`, `adapters::inbound::tests::delete_loom_success`, `adapters::inbound::tests::delete_loom_not_found`

- [ ] Failing test: `adapters::inbound::tests::post_loom_success` — `POST /looms` with valid body returns `201`, loom appears in `GET /looms`
- [ ] Failing test: `adapters::inbound::tests::post_loom_missing_source_dir` — body missing `source_dir`; returns `400`
- [ ] Failing test: `adapters::inbound::tests::post_loom_duplicate_id` — register same loom twice; second returns `409`
- [ ] Failing test: `adapters::inbound::tests::delete_loom_success` — `DELETE /looms/:id` returns `204`, loom no longer in `GET /looms`
- [ ] Failing test: `adapters::inbound::tests::delete_loom_not_found` — `DELETE /looms/:id` for unknown returns `404`
- [ ] Implement `register_loom`, `unregister_loom` handlers
- [ ] Parse request body into `Loom` domain type, call `RegisterLoom`/`UnregisterLoom` use cases
- [ ] Validation: source_dir present, tie_off_point present, ID uniqueness (handled by use case)

### Phase 4: Route Integration Test
**Failing tests created:** `adapters::inbound::tests::full_route_wiring`, `adapters::inbound::tests::existing_routes_preserved`

- [ ] Failing test: `adapters::inbound::tests::full_route_wiring` — all 7 endpoints are accessible on the router; `GET` methods return `200`/`404`, `POST` returns `201`/`400`/`409`, `DELETE` returns `204`/`404`
- [ ] Failing test: `adapters::inbound::tests::existing_routes_preserved` — `/health` still returns `200 ok`, `/agents/{dir}` still works
- [ ] Integration test using `tower::test::TestClient` with full `AppContext` wired up
- [ ] Verify no route conflicts between new and existing endpoints

## Notes
