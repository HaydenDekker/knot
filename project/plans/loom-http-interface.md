# Plan: Loom HTTP Interface

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan adds HTTP endpoints for loom and knot observability and management. All state is sourced from the filesystem (loom-log and knot-state files). It addresses Story 1 (confirm knot active), Story 3 (multiple looms), Story 5 (observe status), and all HTTP-related success criteria.

## Problem

Knot has a file-based observability layer (loom-log, knot-state) but no HTTP interface to query it. Users cannot see which looms are active, which knots are registered, what events fired, what tie-offs were produced, or what errors occurred. Without HTTP endpoints, the system is unobservable.

## Target

- HTTP endpoints for listing looms, listing knots, getting loom activity, and getting knot status.
- All endpoint responses are sourced from the filesystem (loom-log and knot-state files).
- Looms can be registered and unregistered via HTTP without restarting the service.
- Existing routes (`/health`, `/agents/{dir}`) remain functional.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |

## Test Gaps

- No HTTP tests for loom listing endpoint.
- No HTTP tests for knot listing endpoint.
- No HTTP tests for loom-log query endpoint.
- No HTTP tests for knot-state query endpoint.
- No HTTP tests for loom registration/unregistration.
- No integration test: register loom via HTTP → see loom in listing → create strand → see knot-state update via HTTP.

## Proposed Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/looms` | List all active looms (id, source dir, tie-off point, knot count) |
| `GET` | `/looms/:id` | Get loom details (knots, source dir, tie-off point) |
| `GET` | `/looms/:id/activity` | Get loom-log entries for a loom |
| `GET` | `/looms/:id/knots` | List knots in a loom |
| `GET` | `/looms/:id/knots/:knot_name` | Get knot-state for a specific knot |
| `POST` | `/looms` | Register a new loom (body: `{ "source_dir": "...", "tie_off_point": "..." }`) |
| `DELETE` | `/looms/:id` | Unregister a loom (stop watching, remove from active set) |

## Phases

### Phase 0: Loom State Store
- [ ] Implement `LoomStore` — in-memory registry of active looms backed by filesystem state files
- [ ] `LoomStore` holds the mapping of loom ID → `Loom` model + watcher handle
- [ ] Methods: `list()`, `get(id)`, `register(loom)`, `unregister(id)`
- [ ] Integration with `LoomScanner` for initial discovery at startup
- [ ] Unit tests: register loom, list looms, get loom by id, unregister loom, get non-existent loom returns 404

### Phase 1: Loom Listing and Details Endpoints
- [ ] `GET /looms` — returns JSON array of loom summaries
- [ ] `GET /looms/:id` — returns full loom details including knots
- [ ] Wire into axum `Router` under `/api` prefix or at root level
- [ ] Use `State<LoomStore>` extractor for shared state
- [ ] Unit tests via `tower::test::TestClient`: list empty, list with looms, get by id, get non-existent

### Phase 2: Activity and Knot-State Endpoints
- [ ] `GET /looms/:id/activity` — reads loom-log file, returns JSON array of log entries
- [ ] `GET /looms/:id/knots` — lists knots in a loom (from loom model)
- [ ] `GET /looms/:id/knots/:knot_name` — reads knot-state file, returns current state
- [ ] All responses sourced from filesystem files (not in-memory cache)
- [ ] Unit tests: activity returns log entries, knot-state returns current state, missing loom returns 404, missing knot returns 404

### Phase 3: Loom Management Endpoints
- [ ] `POST /looms` — register a new loom: validate config, create state files, start file watcher
- [ ] `DELETE /looms/:id` — unregister: stop watcher, mark loom-log with `loom_stopped`
- [ ] Validation: source dir exists, tie-off point is writable, loom ID is unique
- [ ] Unit tests: register valid loom, register loom with missing source dir returns 400, register duplicate ID returns 409, unregister active loom, unregister non-existent loom returns 404

### Phase 4: Integration Test
- [ ] End-to-end test: register loom via HTTP → confirm loom appears in listing → create strand file → wait for processing → query knot-state via HTTP → verify status is completed → verify tie-off exists on disk
- [ ] Use a mock agent CLI (simple binary that echoes input) for the integration test
- [ ] Verify all endpoints return expected responses

## Notes
