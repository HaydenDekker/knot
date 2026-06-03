# Plan: System Integration and Wiring

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan wires all layers together — bootstrapping the system with real adapters, connecting the debounce engine to the processing pipeline, and verifying the full flow end-to-end. No new business logic; only integration.

## Problem

Knot has all five hex layers implemented (domain, application, outbound adapters, inbound adapter), but they are not wired together. The binary still starts a bare axum router with `/health` and `/agents/{dir}`. There is no `main` that creates `AppContext`, starts file watchers, runs the debounce engine, and feeds events into the processing pipeline. Without wiring, the system does not function.

## Target

- `main.rs` bootstraps the full system: creates adapters, wires use cases, starts watchers, spawns debounce + processing tasks
- Debounce engine connects: raw events from `NotifyEventSource` → `DebounceEngine` → `ProcessStrand` use case
- Loom discovery runs at startup and registers all looms
- HTTP endpoints work against real adapters (not in-memory mocks)
- Full end-to-end integration test: create strand → debounce → CLI → tie-off → HTTP status check
- Graceful shutdown: abort watcher tasks, close channels

## Implementation Status: ⬜ Draft

## Hex Layer: Wiring

Not a hex layer itself — this plan connects all layers. `main.rs` is the composition root. No business logic is added here.

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |
| `domain::*` tests | Domain entities, value objects, events | Plan 1 |
| `application::*` tests | Use cases with mock ports | Plan 2 |
| `adapters::outbound::*` tests | Outbound adapter IO | Plan 3 |
| `adapters::inbound::*` tests | HTTP handlers with TestClient | Plan 4 |

## Test Gaps

- No integration test that wires all real adapters together.
- No test that verifies the full pipeline: file creation → watch → debounce → CLI → tie-off.
- No test that verifies HTTP endpoints against the running system with real IO.
- No test for graceful shutdown.
- No test for multiple looms operating independently.

## Phases

### Phase 0: Composition Root (main.rs)
**Failing tests created:** `integration::tests::app_starts_and_serves_health`, `integration::tests::app_loads_workspace_agent_config`

- [x] Failing test: `integration::tests::app_starts_and_serves_health` — `main()` starts HTTP server, `GET /health` returns `200 ok`
- [x] Failing test: `integration::tests::app_loads_workspace_agent_config` — `WorkspaceAgentConfig` is loaded (defaults: `pi` CLI); accessible in `AppContext`
- [x] Refactor `main.rs` to:
  1. Load `WorkspaceAgentConfig` (defaults or from a config file — keep simple for now)
  2. Create outbound adapter instances (`FileSystemLoomRepository`, `FileSystemKnotStateStore`, `FileSystemLoomLog`, `NotifyEventSource`, `SubprocessAgentRunner`, `FileSystemTieOffSink`)
  3. Create `LoomStore`, create use cases with port instances
  4. Create `AppContext` holding store + ports + use cases
  5. Build axum router with `build_app(AppContext)`
  6. Bind to `127.0.0.1:3000`, start server
- [x] **Alert:** `main.rs` is the composition root — it knows about all layers. This is the only place where all layers meet.

### Phase 1: Startup Discovery and Watcher Boot
**Failing tests created:** `integration::tests::startup_discovers_looms`, `integration::tests::startup_starts_watchers`, `integration::tests::startup_creates_state_files`

- [x] Failing test: `integration::tests::startup_discovers_looms` — given a workspace with loom dirs, startup discovers them and registers in `LoomStore`
- [x] Failing test: `integration::tests::startup_starts_watchers` — after startup, `NotifyEventSource` is watching all loom source directories
- [x] Failing test: `integration::tests::startup_creates_state_files` — after startup, loom-log and knot-state files exist on disk for each loom/knot
- [x] Wire startup sequence: `DiscoverLooms` use case runs → looms registered → watchers started per loom → state files created
- [x] Loom-log entries: `LoomStarted`, `KnotRegistered` for each knot

### Phase 2: Event Pipeline Wiring
**Failing tests created:** `integration::tests::event_flows_through_pipeline`, `integration::tests::debounce_prevents_duplicate_processing`

- [x] Failing test: `integration::tests::event_flows_through_pipeline` — create a file in watched dir → raw event emitted → debounced → `ProcessStrand` invoked → knot-state updated to `processing` → `completed`
- [x] Failing test: `integration::tests::debounce_prevents_duplicate_processing` — rapid edits (3 writes in 50ms) → only one `ProcessStrand` invocation → one tie-off produced
- [x] Wire the event pipeline:
  ```
  NotifyEventSource → mpsc::Sender<StrandEvent> → DebounceEngine → mpsc::Sender<StrandEvent> → ProcessStrand loop
  ```
- [x] `ProcessStrand` loop runs as a `tokio::task`, reading from debounce output channel
- [x] Each event triggers the use case with real adapter instances

### Phase 3: End-to-End Integration Test
**Failing tests created:** `integration::tests::full_pipeline_create_modify_delete`, `integration::tests::full_pipeline_http_observable`, `integration::tests::multiple_looms_independent`

- [ ] Failing test: `integration::tests::full_pipeline_create_modify_delete` — using a mock agent CLI (`echo "processed"`):
  1. Create strand → tie-off file created with content
  2. Modify strand → tie-off overwritten with new content
  3. Delete strand → tie-off reports deletion (file still exists, never deleted)
- [ ] Failing test: `integration::tests::full_pipeline_http_observable` — same flow as above, but verify via HTTP:
  1. `GET /looms` → loom listed
  2. `GET /looms/:id/knots/:knot_name` → status is `idle` before event, `processing` during, `completed` after
  3. `GET /looms/:id/activity` → contains `StrandProcessed` entry
- [ ] Failing test: `integration::tests::multiple_looms_independent` — two looms with different source dirs and tie-off points:
  1. Create strand in loom A → tie-off in A's point only
  2. Create strand in loom B → tie-off in B's point only
  3. No cross-interference (A's knots don't process B's strands)
- [ ] Mock agent CLI: a simple shell script or binary that echoes its input (avoid real `pi` calls in tests)
- [ ] Tests use `tempfile` for workspace, loom dirs, source dirs, tie-off points

### Phase 4: Graceful Shutdown
**Failing tests created:** `integration::tests::graceful_shutdown_stops_watchers`, `integration::tests::shutdown_logs_loom_stopped`

- [ ] Failing test: `integration::tests::graceful_shutdown_stops_watchers` — send shutdown signal; watcher tasks abort, channels close, no new events processed
- [ ] Failing test: `integration::tests::shutdown_logs_loom_stopped` — shutdown writes `LoomStopped` to each loom-log
- [ ] Implement graceful shutdown in `main.rs`:
  - Listen for `Ctrl+C` via `tokio::signal`
  - Abort all watcher `JoinHandle`s
  - Close debounce engine sender
  - Drain processing channel (finish in-flight events)
  - Write `LoomStopped` to each loom-log
  - Shutdown axum server

## Notes
