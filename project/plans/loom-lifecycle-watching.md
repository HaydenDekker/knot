# Plan: Loom Lifecycle Watching

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan fulfils the PRD success criterion: *"Users can define, update, and remove looms and knots programmatically via Knot's HTTP interface or manually via the file system — without restarting the service."* This was attributed to Plans 2 and 4 but was never fully delivered — `POST /looms` registered looms in memory but did not start file watchers, and manually created loom directories on disk were invisible until restart.

## Problem

Knot's loom management is incomplete. Two paths exist for adding looms, both broken:

1. **HTTP (`POST /looms`)** → `RegisterLoom` stores the loom in `LoomStore` and writes log events, but never calls `EventSource::watch()`. The loom is known in memory but does not react to file changes.
2. **Filesystem** → Creating a loom directory on disk with knot `.md` files is completely invisible. `DiscoverLooms::scan()` runs once at startup and is never called again. A stub `POST /looms/discover` endpoint exists (returns 501) but is not implemented.

Additionally, `UnregisterLoom` does not remove file watchers for the loom's source directories, leaving stale `notify` watchers active.

The root cause: `RegisterLoom` and `UnregisterLoom` use cases have no access to `EventSource` (the file watcher). The watcher is created in the composition root (`lib.rs`), used only during startup, and never exposed to the application layer beyond initial configuration.

## Target

- `RegisterLoom` accepts `EventSource` as a dependency and starts watching source directories for the registered loom.
- `UnregisterLoom` accepts `EventSource` and stops watching source directories for the unregistered loom.
- `POST /looms/discover` is implemented: re-scans the rig directory, registers any new looms (including their watchers), skips already-registered looms.
- `AppContext` holds an `Arc<dyn EventSource>` so handlers can pass it to use cases.
- Startup wiring updated: discovered looms go through `RegisterLoom` (or an equivalent path) so watchers are always started consistently.
- HTTP handler tests verify watchers are started (mock `EventSource` tracks `watch()` calls).

## Implementation Status: ⬜ Draft

## Hex Layer: Application → Inbound Adapter → Composition Root

- **Application layer** — `RegisterLoom`, `UnregisterLoom`, and a new `DiscoverLoomsRuntime` use case accept `EventSource`.
- **Inbound adapter** — `register_loom`, `unregister_loom`, and `discover_looms` handlers pass `EventSource` from `AppContext`.
- **Composition root** — `NotifyEventSource` is created once and shared via `AppContext`.

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `adapters::inbound::tests::post_loom_success` | `POST /looms` returns 201, loom appears in `GET /looms` | ✅ Green — but does not verify watchers start |
| `adapters::inbound::tests::delete_loom_success` | `DELETE /looms/:id` returns 204 | ✅ Green — but does not verify watchers stop |
| `application::usecases::tests::register_loom_creates_state_files` | `RegisterLoom` writes log entries, stores loom | ✅ Green — no `EventSource` involved |
| `application::usecases::tests::discover_looms_success` | `DiscoverLooms` scans and registers looms | ✅ Green — no `EventSource` involved |
| `integration::tests::startup_discovers_looms` | Startup discovers looms and starts watchers | ✅ Green — startup-only path |

## Test Gaps

- No test verifies `RegisterLoom` starts file watchers.
- No test verifies `UnregisterLoom` stops file watchers.
- No test for `POST /looms/discover` (endpoint returns 501).
- No integration test: register loom via HTTP → create strand → tie-off produced.
- No test for `DiscoverLooms` skipping already-registered looms.
- `AppContext` does not carry `EventSource` — handler tests cannot verify watch/unwatch calls.

## Phases

### Phase 0: Rig Directory Discovery

**Failing tests created:** `integration::tests::rig_directory_auto_created`, `integration::tests::rig_directory_scanned`

- [ ] Failing test: `integration::tests::rig_directory_auto_created` — start Knot in empty dir; `./rig/` created automatically
- [ ] Failing test: `integration::tests::rig_directory_scanned` — start Knot in dir with `./rig/` containing loom subdirectories; looms discovered and registered
- [ ] Change `AppConfig::default_config()` to set `base_dir: PathBuf::from("./rig")`
- [ ] In `run_startup()` or `build_app_context()`: if `./rig/` doesn't exist, create it with `std::fs::create_dir_all()`
- [ ] `FileSystemLoomLog`, `FileSystemTieOffSink`, `load_rig_config` all operate relative to `./rig/`
- [ ] `POST /looms/discover` uses `./rig/` as its scan root
- [ ] `POST /looms` validates that source/tie-off paths are within or explicitly outside the rig
- [ ] Update existing integration tests that use `.` as base dir to use `./rig/`
- [ ] Update OpenAPI spec: `/config/rig` returns rig path info
- [ ] **Alert:** this changes the rig root from `.` to `./rig/`. Any existing loom directories in the project root will need to be moved or discovered via `POST /looms/discover` pointing to the correct path.

### Phase 1: Tracking Mock EventSource and AppContext Extension

**Failing tests created:** `adapters::inbound::tests::mock_event_source_tracks_watches`, `adapters::inbound::tests::app_context_has_event_source`

- [ ] Failing test: `adapters::inbound::tests::mock_event_source_tracks_watches` — tracking mock `EventSource` records `watch()` and `unwatch()` calls; verify lists are accessible after calls
- [ ] Failing test: `adapters::inbound::tests::app_context_has_event_source` — `AppContext` has an `event_source: Arc<dyn EventSource>` field; `build_test_context()` provides a tracking mock
- [ ] Add `TrackingEventSource` mock to handler tests (records `watch`/`unwatch` paths)
- [ ] Add `event_source: Arc<dyn EventSource>` field to `AppContext`
- [ ] Update `build_test_context()` to provide tracking mock
- [ ] In composition root (`lib.rs` `build_app_context`): create `NotifyEventSource` and store reference in `AppContext`

### Phase 2: RegisterLoom Starts Watchers

**Failing tests created:** `application::usecases::tests::register_loom_starts_watchers`, `adapters::inbound::tests::post_loom_starts_watcher`

- [ ] Failing test: `application::usecases::tests::register_loom_starts_watchers` — `RegisterLoom` with mock `EventSource`: after registration, `watch()` called for source directory (and per-knot source dirs)
- [ ] Failing test: `adapters::inbound::tests::post_loom_starts_watcher` — `POST /looms` with valid body → 201 → mock `EventSource` has recorded a `watch()` call for the source directory
- [ ] Add `event_source: Arc<dyn EventSource>` parameter to `RegisterLoom::new()`
- [ ] In `RegisterLoom::execute()`: after storing loom, call `event_source.watch()` for each knot's effective source directory (knot `source_dir` or loom `source_dir` fallback)
- [ ] Update `AppContext` handler wiring: pass `ctx.event_source` to `RegisterLoom`
- [ ] Update startup (`run_startup` in `lib.rs`): for each discovered loom, start watchers via the same path
- [ ] Update existing tests that construct `RegisterLoom` (add `Arc::new(MockEventSource)`)

### Phase 3: UnregisterLoom Stops Watchers

**Failing tests created:** `application::usecases::tests::unregister_loom_stops_watchers`, `adapters::inbound::tests::delete_loom_stops_watcher`

- [ ] Failing test: `application::usecases::tests::unregister_loom_stops_watchers` — `UnregisterLoom` with mock `EventSource`: after unregistration, `unwatch()` called for each watched source directory
- [ ] Failing test: `adapters::inbound::tests::delete_loom_stops_watcher` — `DELETE /looms/:id` → 204 → mock `EventSource` has recorded an `unwatch()` call
- [ ] Add `event_source: Arc<dyn EventSource>` parameter to `UnregisterLoom::new()`
- [ ] In `UnregisterLoom::execute()`: look up loom in store, call `event_source.unwatch()` for each effective source directory, then remove from store
- [ ] Update handler wiring: pass `ctx.event_source` to `UnregisterLoom`
- [ ] Update existing tests that construct `UnregisterLoom`

### Phase 4: POST /looms/discover Implementation

**Failing tests created:** `adapters::inbound::tests::discover_looms_scans_and_registers`, `adapters::inbound::tests::discover_looms_skips_existing`, `application::usecases::tests::discover_looms_runtime_skips_registered`

- [ ] Failing test: `adapters::inbound::tests::discover_looms_scans_and_registers` — `POST /looms/discover` with a rig containing new loom directories → 200 with list of discovered IDs → mock `EventSource` has `watch()` calls → looms appear in `GET /looms`
- [ ] Failing test: `adapters::inbound::tests::discover_looms_skips_existing` — `POST /looms/discover` when loom already registered → 200 with empty or partial list (no duplicates) → no duplicate `watch()` calls
- [ ] Failing test: `application::usecases::tests::discover_looms_runtime_skips_registered` — `DiscoverLooms` use case given looms where one ID already in store → only new looms are registered (log entries + watchers), existing ones skipped
- [ ] Implement `discover_looms` handler: calls `DiscoverLooms` with base dir from `AppContext` (need to store `base_dir` in context or accept as param)
- [ ] In `DiscoverLooms::execute()`: check `store.get()` before registering each loom; skip if already present
- [ ] After registering each new loom: call `event_source.watch()` for source directories (same logic as `RegisterLoom`)
- [ ] Return discovered loom IDs in response (200 with JSON array)
- [ ] Register route: `POST /looms/discover` already wired, update handler body
- [ ] Update OpenAPI schema to remove 501 response, add 200 response

### Phase 5: Integration Test — Full Lifecycle

**Failing tests created:** `integration::tests::http_register_then_process_strand`, `integration::tests::discover_then_process_strand`, `integration::tests::unregister_stops_processing`

- [ ] Failing test: `integration::tests::http_register_then_process_strand` — `POST /looms` to register → create strand file in source dir → tie-off produced → verify via `GET /looms/:id/knots/:knot_name`
- [ ] Failing test: `integration::tests::discover_then_process_strand` — create loom directory on disk → `POST /looms/discover` → create strand file → tie-off produced
- [ ] Failing test: `integration::tests::unregister_stops_processing` — `DELETE /looms/:id` → create strand file → no new tie-off produced (watcher removed)
- [ ] Tests use mock agent CLI (`echo "processed"`) and `tempfile` for rig/loom/source directories
- [ ] Verify end-to-end: HTTP → use case → `EventSource::watch()` → file creation → debounce → agent → tie-off

## Notes
