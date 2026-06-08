# Plan: lib.rs Composition Root and Inbound Adapter Tidy

## Problem

Two files have accumulated multiple concerns that should be separated:

- **`src/lib.rs`** (440 lines) — the composition root also holds two HTTP handlers (`health`, `list_agents`) and dead code (`graceful_shutdown`). The handlers are inbound adapter concerns that should live in `adapters/inbound/`, and the dead `graceful_shutdown` function is the old pre-ADR-002 shutdown pattern (takes `JoinHandle` not `JoinSet`, drops only one sender clone).

- **`src/adapters/inbound/mod.rs`** (2211 lines) — holds OpenAPI schema, DTOs, `AppContext`, 10 route handlers, router wiring, and ~1600 lines of tests all in one file.

## Target

### lib.rs after tidy (~60 lines)

```
src/lib.rs
├── module declarations (pub mod adapters, application, domain)
├── re-exports (AppContext, build_app, ShutdownSignal, start_server, etc.)
└── server module re-exports
```

lib.rs becomes a clean facade — no handler code, no dead code, no composition logic.

### New: src/server.rs (~380 lines)

Composition root logic extracted from `lib.rs`:

| Item | Role |
|------|------|
| `AppConfig` + `default_config()` | Server configuration |
| `load_rig_config()` | YAML config loader |
| `build_app_context()` | Wires all hex layers into `AppContext` |
| `start_event_pipeline()` | Spawns debounce + ProcessStrand into `JoinSet` |
| `run_startup()` | DiscoverLooms execution |
| `ShutdownSignal` enum | Ctrl+C or oneshot channel |
| `start_server()` | Public entry point |
| `start_server_with_shutdown()` | Full lifecycle with cascade shutdown |

`graceful_shutdown` is **removed** (dead code — ADR-002 notes it is unused, `start_server_with_shutdown` replaced it with the inline `JoinSet` drain loop).

### Split: src/adapters/inbound/

```
src/adapters/inbound/
├── mod.rs         (15 lines)  — pub mod + pub use re-exports
├── types.rs       (70 lines)  — AppContext, RegisterLoomRequest, KnotRequest, RigConfigResponse
├── loom.rs        (320 lines) — 8 loom handlers + helpers + ~1400 test lines
├── system.rs      (50 lines)  — 3 system handlers (health, list_agents, get_rig_config)
└── router.rs      (50 lines)  — ApiDoc, build_app()
```

### Handler concern split

| File | Handlers | Helpers | Tests |
|------|----------|---------|-------|
| `loom.rs` | `list_looms`, `get_loom`, `get_loom_activity`, `get_loom_knots`, `get_knot_status`, `register_loom`, `unregister_loom`, `discover_looms` | `generate_knot_file`, `quote_yaml_scalar`, mock ports, `TrackingEventSource` | All existing handler unit tests (Phase 0–4) |
| `system.rs` | `health`, `list_agents`, `get_rig_config` | — | None needed — trivial single-line delegations, covered by route wiring tests in `loom.rs` |

`health()` and `list_agents()` move from `lib.rs` into `inbound/system.rs`. They're HTTP handlers — inbound adapter concerns. `lib.rs` re-exports them for backwards compatibility.

## Implementation Status: ✅ Complete (2026-06-08)

## Existing Tests

| Test Source | What it covers | Status |
|-------------|---------------|--------|
| `tests/task_management.rs` | Server lifecycle, cascade shutdown, LoomStopped logging | ✅ Green — uses `start_server_with_shutdown` |
| `tests/generic_task_management.rs` | Channel-cascade pattern (10 tests, tokio only) | ✅ Green |
| `tests/http_interface.rs` | HTTP endpoints, health, agents listing | ✅ Green — exercises `health` and `list_agents` |
| `src/adapters/inbound/mod.rs` tests | Handler unit tests (mock ports, routing) | ✅ Green — ~1600 lines |
| `tests/helpers.rs` | `spawn_server`, `spawn_server_with_shutdown` | ✅ Green — shared infra |

## Test Gaps

- No test that exercises `graceful_shutdown` directly (it's dead code — removing it is safe)
- No test that validates `start_server` (CtrlC variant) — always tested via `start_server_with_shutdown` + channel variant
- Handler unit tests live in `inbound/mod.rs` — they'll move with handlers to `loom.rs`, no gap introduced
- `system.rs` has no tests — `health`, `list_agents`, `get_rig_config` are single-line delegations; their behaviour is covered by the route wiring test (`full_route_wiring`) in `loom.rs` and by integration tests in `tests/http_interface.rs`

## Phases

### Phase 0: Remove dead code from lib.rs
- [x] Remove `graceful_shutdown` function (lines 276-310) — dead code per ADR-002, replaced by inline `JoinSet` drain in `start_server_with_shutdown`
- [x] Remove unused `use tokio::task::JoinHandle` import (if present)
- [x] Run `cargo test` — verify no breakage (nothing should call `graceful_shutdown`)

### Phase 1: Split inbound/mod.rs into types.rs + loom.rs + system.rs + router.rs
- [x] Create `src/adapters/inbound/types.rs` — extract `AppContext`, `RegisterLoomRequest`, `KnotRequest`, `RigConfigResponse`
- [x] Create `src/adapters/inbound/loom.rs` — extract 8 loom handlers (`list_looms`, `get_loom`, `get_loom_activity`, `get_loom_knots`, `get_knot_status`, `register_loom`, `unregister_loom`, `discover_looms`) + helpers (`generate_knot_file`, `quote_yaml_scalar`) + full test module (mock ports, `TrackingEventSource`, `build_test_context`, Phase 0–4 tests)
- [x] Create `src/adapters/inbound/system.rs` — extract `health()`, `list_agents()`, `get_rig_config()` from `lib.rs` + `mod.rs`. No test module needed (trivial single-line delegations, covered by route wiring tests in `loom.rs`)
- [x] Create `src/adapters/inbound/router.rs` — extract `ApiDoc`, `build_app()`
- [x] Update `src/adapters/inbound/mod.rs` to be thin facade:
  ```rust
  pub mod types;
  pub mod loom;
  pub mod system;
  pub mod router;

  pub use types::{AppContext, RegisterLoomRequest, KnotRequest, RigConfigResponse};
  pub use loom::{list_looms, get_loom, get_loom_activity, get_loom_knots, get_knot_status, register_loom, unregister_loom, discover_looms};
  pub use system::{health, list_agents, get_rig_config};
  pub use router::build_app;
  ```
- [x] Update `utoipa::paths()` in `ApiDoc` to use full paths:
  - Loom handlers: `crate::adapters::inbound::loom::list_looms`, etc.
  - System handlers: `crate::adapters::inbound::system::health`, etc.
- [x] Update `lib.rs` imports — `pub use adapters::inbound::{build_app, AppContext}` should still work via re-exports; add `pub use adapters::inbound::system::{health, list_agents};` to preserve `crate::health` / `crate::list_agents`
- [x] Run `cargo test` — all tests pass, including handler unit tests

### Phase 3: Extract composition root into src/server.rs
- [x] Create `src/server.rs` with:
  - `AppConfig` struct + `default_config()` impl
  - `load_rig_config()` helper (private)
  - `build_app_context()`
  - `start_event_pipeline()`
  - `run_startup()`
  - `ShutdownSignal` enum
  - `start_server()`
  - `start_server_with_shutdown()`
- [x] Update `lib.rs`:
  - Add `mod server;`
  - Re-export: `pub use server::{AppConfig, ShutdownSignal, start_server, start_server_with_shutdown, start_event_pipeline, build_app_context, run_startup};`
  - Remove the moved items
- [x] Update `src/main.rs` — imports should still resolve through `lib.rs` re-exports
- [x] Update `tests/` files — imports should still resolve through `lib.rs` re-exports (`knot::AppConfig`, `knot::start_server`, `knot::ShutdownSignal`)
- [x] Run `cargo test` — all tests pass

### Phase 4: Verify and clean
- [x] Run full test suite: `cargo test` — all passing
- [x] Run `cargo clippy` — no new warnings
- [x] Verify `lib.rs` is ~60 lines (module declarations + re-exports only)
- [x] Verify `inbound/mod.rs` is ~15 lines (pub mod + pub use only)
- [x] Verify `inbound/loom.rs` has 8 loom handlers + helpers + all handler tests
- [x] Verify `inbound/system.rs` has 3 system handlers (no tests)
- [x] Verify `inbound/types.rs` has data structures only
- [x] Verify `inbound/router.rs` has ApiDoc + build_app only
- [x] Verify `server.rs` has all composition root logic
- [x] Verify no dead code warnings (`cargo test` + `cargo build`)

## Notes

- **Hexagonal alignment**: `lib.rs` is the composition root — it should wire layers, not contain handlers. Moving `health` and `list_agents` into `inbound/` corrects this violation.
- **Handler concern split**: The 11 handlers split into two natural concerns. `loom.rs` (8 handlers) — all loom/knot CRUD, shares mock ports and `TrackingEventSource` in tests. `system.rs` (3 handlers) — `health`, `list_agents`, `get_rig_config`, trivial single-line delegations with no helpers or tests needed.
- **Backwards compatibility**: All public paths (`knot::AppConfig`, `knot::start_server`, `knot::health`, etc.) are preserved via re-exports. Tests and `main.rs` don't need changes.
- **`graceful_shutdown` removal**: ADR-002 explicitly documents this as unused. Its `JoinHandle` pattern is the old approach that `start_server_with_shutdown` replaced. Removing it eliminates a misleading API that could produce incorrect shutdown behaviour if ever called.
- **Notify thread delay**: Documented as accepted in ADR-002. The `in_flight_processing_completes_on_shutdown` test remains `#[ignore]`. No change in this plan.
