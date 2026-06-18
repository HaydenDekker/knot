# ADR-008: Full File-First State — Removal of HTTP Interface

## Status

✅ Accepted (2026-06-19)

## Context

Knot maintained an HTTP interface (Axum) for observability — listing looms, knots, profiles, and rig config via REST endpoints. Configuration was already file-first (skills write `.md` files, file watcher picks up changes), but **reading** state required an HTTP client.

This created a half-measure: write via files, read via HTTP. External consumers (skills, scripts, other tools) needed to maintain HTTP helpers and handle server lifecycle just to inspect state.

The HTTP stack brought framework dependencies (`axum`, `utoipa`, `utoipa-swagger-ui`, `tower`) and test infrastructure (TCP helpers, server spawning, port polling) that added complexity without adding capability — the state was always file-backed.

## Decision

Remove the HTTP server entirely. Replace all observability with `rig/state.json`, a single JSON file written atomically on a 5-second poll cycle.

**What was removed:**
- `src/adapters/inbound/` module (router, handlers, types) — ~3000+ lines
- `axum`, `utoipa`, `utoipa-swagger-ui`, `tower` dependencies from `Cargo.toml`
- HTTP-based test infrastructure (`spawn_server`, `http_get`, `poll_knot_status`, etc.)
- Swagger UI and OpenAPI spec generation
- `AppContext` (Axum `State`) and `bind_addr` from `AppConfig`

**What was added:**
- `RigState` domain type — JSON-serializable snapshot of all runtime state
- `StateWriter` background task — polls every 5 seconds, writes atomically (write-to-tmp + rename)
- File-based test helpers — `wait_for_state_field`, `wait_for_loom_in_state`, `wait_for_knot_status_in_state`
- Updated skills read `rig/state.json` instead of calling HTTP endpoints

**Why this approach:**
- External consumers just read a file — no server lifecycle, no HTTP client needed
- Atomic writes guarantee readers never see partial state
- 5-second staleness is acceptable for observability (not real-time control)
- Removes ~3000 lines of handler code and 4 framework dependencies
- Aligns the entire system with the file-first philosophy already established by ADR-006

## Consequences

**Positive:**
- Simpler codebase — no HTTP server, no framework dependency
- Lighter binary — fewer dependencies, no port binding
- Simpler tests — no server spawning or HTTP helpers needed
- Skills and scripts can inspect state without running Knot (read a file)
- Consistent with file-first approach (write via files, read via files)

**Negative:**
- No real-time state — consumers must poll or accept 5-second staleness
- No programmatic API — no `curl` or browser for quick inspection
- No Swagger UI for API documentation (no longer needed)
- Loss of structured query capability — consumers must parse the full JSON

**Mitigations:**
- State writer uses atomic writes (tmp + rename) for read safety
- Skills use file-based helpers with retry logic for staleness tolerance
- `rig/state.json` schema documented in plan and skill files

## Related

- [ADR-006: File-First Configuration](adr-006-file-first-configuration.md) — established file-first for configuration
- [Plan 38: Removal of HTTP Interface](../plans/removal-of-http-interface.md) — implementation plan
- [Plan 26: HTTP Observability Only](../plans/http-observability-only.md) — previous plan that removed control endpoints but kept GET endpoints
