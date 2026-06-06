# Plan: Loom/Knot Auto-Discovery and Knot CRUD API

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan fulfils the PRD success criterion: *"Users can define, update, and remove looms and knots programmatically via Knot's HTTP interface or manually via the file system — without restarting the service."* It delivers the real-time auto-discovery mechanism for looms and knots, and adds HTTP endpoints for direct knot management.

## Problem

Two gaps prevent users from modifying knot configuration at runtime:

1. **No file watching for loom/knot definition changes** — `NotifyEventSource` watches strand directories only. Editing a knot `.md` file, adding a new `.md` file to a loom, or creating a new loom directory on disk is invisible until restart. The `POST /looms/discover` endpoint partially addresses loom creation but is a manual trigger, skips already-registered looms, and does not handle knot-level changes.

2. **No API for individual knot management** — The HTTP interface supports loom-level CRUD (`POST /looms`, `DELETE /looms/{id}`) but has no endpoints to create, update, or delete individual knots within a loom. Knots are treated as file-system-only resources with no API surface.

The PRD promises that both the HTTP interface and the filesystem are equally valid paths for configuration, and that changes are active immediately without restart. The auto-discovery mechanism (watching rig and loom directories) delivers the filesystem path. The knot CRUD API delivers the HTTP path. Both update the same in-memory store and `.md` files on disk.

## Target

When this plan is done:

- `NotifyEventSource` watches the rig directory and loom directories in addition to strand directories
- New `*-loom` directories created on disk are auto-registered with file watchers immediately
- New `.md` files in an existing loom directory are auto-parsed and registered as knots
- Edited `.md` files update the in-memory knot config in real time
- Deleted `.md` files deregister the knot
- HTTP endpoints provide CRUD for individual knots: `POST /looms/{id}/knots`, `PATCH /looms/{id}/knots/{name}`, `DELETE /looms/{id}/knots/{name}`
- `POST /looms/discover` is removed (replaced by auto-discovery)
- The HTTP interface (`GET /looms`, `GET /looms/{id}/knots`) always reflects the current in-memory state
- Full integration tests verify auto-discovery and knot CRUD end-to-end

## Implementation Status: ⬜ Draft

## Hex Layer: Domain → Application → Outbound Adapters → Inbound Adapter → Composition Root

This plan works across all hex layers:

- **Domain** — New `ConfigEvent` type for loom/knot lifecycle events (separate from `StrandEvent`)
- **Application** — `ConfigEventHandler` use case processes config events; `ManageKnot` use case for HTTP-driven knot CRUD
- **Outbound Adapters** — `NotifyEventSource` extended to emit config events for rig/loom directory changes
- **Inbound Adapters** — New knot CRUD handlers; remove discover handler
- **Composition Root** — Wire config event channel, start config event handler task

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `domain::events::tests::strand_event_types` | StrandEvent variants | ✅ Green — no change needed |
| `domain::events::tests::loom_event_types` | LoomEvent variants | ✅ Green — no change needed |
| `application::usecases::tests::discover_looms_*` | DiscoverLooms use case | ✅ Green — will be superseded by ConfigEventHandler |
| `application::usecases::tests::register_loom_starts_watchers` | RegisterLoom with EventSource | ✅ Green — no change needed |
| `application::usecases::tests::unregister_loom_stops_watchers` | UnregisterLoom with EventSource | ✅ Green — no change needed |
| `application::debounce::tests::*` | DebounceEngine | ✅ Green — no change needed |
| `application::store::tests::*` | LoomStore CRUD | ✅ Green — no change needed |
| `outbound::event_source::tests::create_event_emitted` | Strand create event | ✅ Green — no change needed |
| `outbound::event_source::tests::modify_event_emitted` | Strand modify event | ✅ Green — no change needed |
| `outbound::event_source::tests::delete_event_emitted` | Strand delete event | ✅ Green — no change needed |
| `outbound::event_source::tests::directory_events_filtered` | Directory events dropped | ✅ Green — **will change**: directory events now meaningful for rig |
| `outbound::loom_repository::tests::scan_*` | Loom repository scanning | ✅ Green — no change needed |
| `inbound::tests::post_loom_success` | POST /looms handler | ✅ Green — no change needed |
| `inbound::tests::discover_looms_scans_and_registers` | POST /looms/discover handler | ✅ Green — **will be removed** |
| `inbound::tests::discover_looms_skips_existing` | POST /looms/discover skip logic | ✅ Green — **will be removed** |
| `integration::tests::startup_discovers_looms` | Startup discovery | ✅ Green — no change needed |
| `integration::tests::discover_then_process_strand` | Discover endpoint flow | ✅ Green — **will be removed** |
| `integration::tests::api_register_then_discover_after_restart` | Register + restart rediscovery | ✅ Green — **will be removed** |
| `integration::tests::discovery_ignores_non_loom_directories` | -loom filter | ✅ Green — no change needed |
| `swagger_ui::tests::swagger_endpoint_paths` | Swagger path presence | ✅ Green — will update to remove `/looms/discover` |

## Test Gaps

- No domain test for `ConfigEvent` type
- No application test for `ConfigEventHandler` use case (new loom, new knot, updated knot, deleted knot)
- No application test for `ManageKnot` use case (create, update, delete via API)
- No outbound adapter test for rig directory watching (new `*-loom` dir → ConfigEvent)
- No outbound adapter test for loom directory watching (new `.md` → ConfigEvent, edit `.md` → ConfigEvent, delete `.md` → ConfigEvent)
- No inbound handler test for `POST /looms/{id}/knots` (create knot)
- No inbound handler test for `PATCH /looms/{id}/knots/{name}` (update knot)
- No inbound handler test for `DELETE /looms/{id}/knots/{name}` (delete knot)
- No integration test for runtime loom auto-discovery (create loom dir → GET /looms shows it → create strand → tie-off produced)
- No integration test for runtime knot auto-discovery (add `.md` to existing loom → GET /looms/{id}/knots shows it)
- No integration test for runtime knot update (edit `.md` → GET /looms/{id} shows new config)
- No integration test for runtime knot deletion (delete `.md` → GET /looms/{id}/knots no longer shows it)

## Phases

### Phase 0: Domain — `ConfigEvent` Type

**Failing tests created:** `domain::events::tests::config_event_types`

**Hex Layer:** Domain

The `StrandEvent` enum describes strand lifecycle events. We need a separate type for configuration changes (loom/knot definition file changes). These are structurally different: a config event carries a file path (not a strand path) and a change type that affects the loom/knot registry, not the processing pipeline.

Changes:
- `ConfigEvent` enum with three variants: `LoomAdded(LoomId)`, `KnotAdded(Knot)`, `KnotModified(Knot)`, `KnotDeleted(KnotId)`
- The `ConfigEvent` carries enough data for the handler to act without re-scanning
- `ConfigEvent` derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema`
- Domain tests verify all variants exist and serialise round-trip

```rust
pub enum ConfigEvent {
    /// A new loom directory was detected (ends in `-loom`).
    LoomAdded { loom_id: LoomId },
    /// A new knot `.md` file was created in a loom directory.
    KnotAdded { loom_id: LoomId, knot: Knot },
    /// An existing knot `.md` file was modified in a loom directory.
    KnotModified { loom_id: LoomId, knot: Knot },
    /// A knot `.md` file was deleted from a loom directory.
    KnotDeleted { loom_id: LoomId, knot_id: KnotId },
}
```

- [x] Failing test: `domain::events::tests::config_event_types` — all four variants exist, carry correct data, serialise round-trip
- [x] Implement: `ConfigEvent` enum in `src/domain/events.rs`
- [x] All domain tests green

### Phase 1: Application — `ConfigEventHandler` Use Case

**Failing tests created:** `application::usecases::tests::config_handler_loom_added`, `application::usecases::tests::config_handler_knot_added`, `application::usecases::tests::config_handler_knot_modified`, `application::usecases::tests::config_handler_knot_deleted`

**Hex Layer:** Application

The `ConfigEventHandler` receives `ConfigEvent`s and updates the `LoomStore`, starts/stops watchers, and writes loom-log entries. It depends on `LoomStore`, `LoomRepository` (for scanning), `LoomLogPort`, and `EventSource`.

For `LoomAdded`: scan the loom directory via `LoomRepository::scan()` to get the full loom, then register it (same flow as `RegisterLoom` — log events, store loom, start watchers).

For `KnotAdded`: parse the `.md` file, add the knot to the loom in the store, log `KnotRegistered`, start watcher for its `strand_dir`.

For `KnotModified`: re-parse the `.md` file, update the knot in the store, stop old watcher for `strand_dir`, start new watcher if `strand_dir` changed.

For `KnotDeleted`: remove the knot from the loom in the store, stop watcher for its `strand_dir`, log `KnotDeregistered` (new `LoomEvent` variant).

- [x] Failing test: `config_handler_loom_added` — given `ConfigEvent::LoomAdded`, handler scans loom dir via repository, registers loom in store, logs events, starts watchers
- [x] Failing test: `config_handler_knot_added` — given `ConfigEvent::KnotAdded`, handler adds knot to loom in store, logs `KnotRegistered`, starts watcher
- [x] Failing test: `config_handler_knot_modified` — given `ConfigEvent::KnotModified`, handler updates knot in store, stops old watcher, starts new watcher
- [x] Failing test: `config_handler_knot_deleted` — given `ConfigEvent::KnotDeleted`, handler removes knot from loom in store, stops watcher
- [x] Add `LoomEvent::KnotDeregistered { loom_id, knot_id }` variant
- [x] Implement: `ConfigEventHandler` use case in `src/application/usecases.rs`
- [x] All application tests green

### Phase 2: Application — `ManageKnot` Use Case

**Failing tests created:** `application::usecases::tests::manage_knot_create`, `application::usecases::tests::manage_knot_update`, `application::usecases::tests::manage_knot_delete`

**Hex Layer:** Application

HTTP-driven knot CRUD. The `ManageKnot` use case writes `.md` files to disk and updates the in-memory store. It writes to the loom directory (same location the config handler reads from), so both paths stay in sync.

Changes:
- `ManageKnot` accepts `LoomStore`, `EventSource`, `LoomLogPort`, and a `KnotAction` enum
- `KnotAction::Create(knot)` → write `.md` file to loom directory, add to store, start watcher
- `KnotAction::Update(knot)` → update `.md` file, update store, stop old watcher + start new watcher
- `KnotAction::Delete(knot_name)` → delete `.md` file, remove from store, stop watcher
- The `.md` file path is `<rig>/<loom-id>/<knot-name>.md`
- Path resolution uses the same `FileSystemLoomRepository::resolve_path()` for consistency
- The `LoomRepository` port is needed to resolve paths but the use case writes directly (not through the repository, since it's not a scan operation)

**Alert:** This use case writes `.md` files — it has side effects on the filesystem. However, this is consistent with `POST /looms` which also writes knot files. The hex layering is: use case calls a port for file IO? No — the use case doesn't call a port for writing `.md` files. Instead, it delegates to the inbound adapter or... Actually, in the current codebase, `POST /looms` handler writes files directly (it calls `std::fs::write`). The use cases don't touch the filesystem for knot definitions. So `ManageKnot` should also just update the in-memory store and the HTTP handler writes the files.

**Revised approach:** The HTTP handler writes the `.md` files to disk. The `ConfigEventHandler` picks them up from the file watcher. This keeps the use case pure (in-memory only) and lets the file system be the source of truth.

Changes:
- `ManageKnot` use case: `LoomStore` only. Updates the in-memory loom entry.
- HTTP handler: writes `.md` files to disk (same pattern as `POST /looms`), then calls `ManageKnot` to update the store. The config event handler also sees the file change but the store is already updated (idempotent — `LoomStore::register()` overwrites).

- [x] Failing test: `manage_knot_create` — given a new knot, handler adds it to the loom in the store
- [x] Failing test: `manage_knot_update` — given an updated knot, handler updates it in the store
- [x] Failing test: `manage_knot_delete` — given a knot name, handler removes it from the loom in the store
- [x] Implement: `ManageKnot` use case with `KnotAction` enum
- [x] All application tests green

### Phase 3: Outbound Adapters — `NotifyEventSource` with Rig/Loom Watching

**Failing tests created:** `outbound::event_source::tests::rig_dir_new_loom_emits_config_event`, `outbound::event_source::tests::loom_dir_new_knot_emits_config_event`, `outbound::event_source::tests::loom_dir_edit_knot_emits_config_event`, `outbound::event_source::tests::loom_dir_delete_knot_emits_config_event`

**Hex Layer:** Outbound Adapters

`NotifyEventSource` currently emits `StrandEvent` to a single mpsc channel. We need it to also emit `ConfigEvent`s when:
1. A new `*-loom` directory is created in the rig directory
2. A `.md` file is created/modified/deleted in a loom directory

**Architecture:** The `NotifyEventSource` is created with TWO channels — one for strand events, one for config events. The existing `mpsc::Sender<StrandEvent>` stays. A new `mpsc::Sender<ConfigEvent>` is added.

The `watch()` method is extended to accept an optional `WatchType` enum:
- `WatchType::Strand(loom_id, knot_id)` — current behaviour, maps events to `StrandEvent`
- `WatchType::Rig` — maps directory creation events (names ending in `-loom`) to `ConfigEvent::LoomAdded`
- `WatchType::Loom(loom_id)` — maps `.md` file creation/modify/delete to `ConfigEvent::Knot*`

The `set_loom_ids()` method is replaced by a new `register_watch()` method that takes the watch type.

Changes:
- `NotifyEventSource` constructor accepts two senders: `(StrandEventSender, ConfigEventSender)`
- `watch(path, watch_type)` — registers a watch with a type
- `unwatch(path)` — unchanged
- `InnerState.watched_dirs` maps `PathBuf → WatchType` instead of `(LoomId, KnotId)`
- `map_event()` extended to produce `ConfigEvent` for rig/loom watches
- The callback sends to the appropriate channel based on the watch type

- [x] Failing test: `rig_dir_new_loom_emits_config_event` — watch rig dir with `WatchType::Rig`; create `new-loom` directory → `ConfigEvent::LoomAdded` emitted on config channel
- [x] Failing test: `loom_dir_new_knot_emits_config_event` — watch loom dir with `WatchType::Loom(id)`; create `new-knot.md` → `ConfigEvent::KnotAdded` emitted
- [x] Failing test: `loom_dir_edit_knot_emits_config_event` — watch loom dir; edit `existing-knot.md` → `ConfigEvent::KnotModified` emitted
- [x] Failing test: `loom_dir_delete_knot_emits_config_event` — watch loom dir; delete `knot.md` → `ConfigEvent::KnotDeleted` emitted
- [x] Implement: `ConfigEventSender` channel in `NotifyEventSource`, `WatchType` enum, extended `map_event()`
- [x] Update existing `EventSource` port trait to include `register_watch(path, watch_type)` or similar
- [x] Update existing tests to use new API (strand watching still works)
- [x] All outbound adapter tests green

### Phase 4: Composition Root — Wire Config Event Channel and Handler

**Hex Layer:** Composition Root

Changes:
- `build_app_context()` creates a second mpsc channel for config events
- `NotifyEventSource` is created with both channels
- After `NotifyEventSource` construction, register watches:
  - `watch(rig_dir, WatchType::Rig)` — auto-discover new looms
  - For each startup-discovered loom: `watch(loom_dir, WatchType::Loom(loom_id))`
- `start_event_pipeline()` or a new `start_config_pipeline()` spawns a tokio task that reads from the config channel and calls `ConfigEventHandler`
- `AppContext` gains a `config_sender: mpsc::Sender<ConfigEvent>` field (or the sender is captured by the config pipeline task, not stored in context)
- The config pipeline task runs alongside the strand processing pipeline

- [x] Update `build_app_context()` to create config event channel and pass both to `NotifyEventSource`
- [x] Wire rig directory watch in startup (`run_startup`)
- [x] Wire loom directory watches after each loom registration
- [x] Start config event handler task in `start_server_with_shutdown()`
- [x] Update `graceful_shutdown()` if needed (config handler drains when channel closes)
- [x] Build succeeds, all tests pass

### Phase 5: Inbound Adapters — Knot CRUD Endpoints and Remove Discover

**Failing tests created:** `inbound::tests::post_knot_success`, `inbound::tests::post_knot_missing_fields`, `inbound::tests::patch_knot_success`, `inbound::tests::patch_knot_not_found`, `inbound::tests::delete_knot_success`, `inbound::tests::delete_knot_not_found`

**Hex Layer:** Inbound Adapter

New endpoints:

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/looms/{id}/knots` | Create a new knot in a loom |
| `PATCH` | `/looms/{id}/knots/{name}` | Update an existing knot's config |
| `DELETE` | `/looms/{id}/knots/{name}` | Remove a knot from a loom |

Request body for `POST` and `PATCH` (`KnotRequest`):

```rust
pub struct KnotRequest {
    pub name: String,
    pub agent_config: AgentConfig,
    pub prompt_template: PromptTemplate,
    pub strand_dir: String,
    pub tie_off_dir: String,
}
```

Each handler:
1. Validates the loom exists in the store
2. Writes/updates/deletes the `.md` file in the loom directory
3. Calls `ManageKnot` to update the in-memory store
4. Returns appropriate status (201/200/204)

The handler also calls the config event channel directly (or the file watcher picks it up — prefer file watcher for consistency, but add a brief delay for the watcher to fire, OR call the config handler directly from the use case). Actually, the cleaner approach: the handler writes the file, and the `ConfigEventHandler` picks it up. But for HTTP responses we need immediate confirmation. So the handler calls `ManageKnot` for the in-memory update, writes the file for persistence, and the config event handler is idempotent (it will see the file change but the store already matches).

Also:
- Remove `POST /looms/discover` route and handler
- Remove `DiscoverLooms` use case? No — keep it, it's still used at startup via `run_startup()`. Just remove the HTTP endpoint.

- [x] Failing test: `post_knot_success` — `POST /looms/{id}/knots` with valid body → 201 → knot appears in `GET /looms/{id}/knots` → `.md` file exists on disk
- [x] Failing test: `post_knot_missing_fields` — `POST /looms/{id}/knots` with missing `strand_dir` → 400
- [x] Failing test: `patch_knot_success` — `PATCH /looms/{id}/knots/{name}` with updated config → 200 → `GET /looms/{id}` shows new config → `.md` file updated on disk
- [x] Failing test: `patch_knot_not_found` — `PATCH /looms/{id}/knots/{unknown}` → 404
- [x] Failing test: `delete_knot_success` — `DELETE /looms/{id}/knots/{name}` → 204 → knot no longer in `GET /looms/{id}/knots` → `.md` file deleted on disk
- [x] Failing test: `delete_knot_not_found` — `DELETE /looms/{id}/knots/{unknown}` → 404
- [x] Implement: `create_knot`, `update_knot`, `delete_knot` handlers
- [x] Wire routes: `.route("/looms/{id}/knots", post(create_knot))`, `.route("/looms/{id}/knots/{name}", patch(update_knot))`, `.route("/looms/{id}/knots/{name}", delete(delete_knot))`
- [x] Remove: `POST /looms/discover` route and handler
- [x] Update OpenAPI schema annotations (remove `/looms/discover`, add new endpoints)
- [x] Update `utoipa::path` annotations
- [x] All inbound tests green

### Phase 6: Integration Tests — Full Auto-Discovery and CRUD Verification

**Failing tests created:** `integration::tests::runtime_loom_auto_discovery`, `integration::tests::runtime_knot_auto_discovery`, `integration::tests::runtime_knot_edit_picks_up_change`, `integration::tests::runtime_knot_deletion`, `integration::tests::http_create_knot`, `integration::tests::http_update_knot`, `integration::tests::http_delete_knot`, `integration::tests::discover_endpoint_removed`

**Hex Layer:** Integration

- [ ] Failing test: `runtime_loom_auto_discovery` — start server with empty rig → create `test-loom/` dir with `.md` file → `GET /looms` shows new loom → create strand → tie-off produced
- [ ] Failing test: `runtime_knot_auto_discovery` — start server with existing loom → drop new `.md` file in loom dir → `GET /looms/{id}/knots` shows new knot
- [ ] Failing test: `runtime_knot_edit_picks_up_change` — edit `.md` file (change model) → `GET /looms/{id}` shows updated config
- [ ] Failing test: `runtime_knot_deletion` — delete `.md` file → `GET /looms/{id}/knots` no longer shows the knot
- [ ] Failing test: `http_create_knot` — `POST /looms/{id}/knots` with valid body → 201 → knot in `GET /looms/{id}/knots` → `.md` file on disk → create strand → tie-off produced
- [ ] Failing test: `http_update_knot` — `PATCH /looms/{id}/knots/{name}` with new model → 200 → `GET /looms/{id}` shows new model → `.md` file updated on disk
- [ ] Failing test: `http_delete_knot` — `DELETE /looms/{id}/knots/{name}` → 204 → knot no longer in `GET /looms/{id}/knots` → `.md` file deleted on disk
- [ ] Failing test: `discover_endpoint_removed` — `POST /looms/discover` → 404 (not found)
- [ ] Remove/update existing integration tests that use `POST /looms/discover`
- [ ] Update `swagger_ui.rs` to remove `/looms/discover` assertion
- [ ] Full integration test suite green

### Phase 7: Skills Update

**Hex Layer:** Skills + Documentation

- [ ] Update `knots-and-looms/SKILL.md`:
  - Add knot CRUD endpoints to the API reference table
  - Update "Modify a Loom" section with new PATCH/DELETE knot endpoints
  - Remove any references to restart for knot changes (already done in PRD update)
- [ ] Update `knot-inspect/SKILL.md` if it references discover endpoint
- [ ] Verify OpenAPI spec served by Knot matches the updated API

## Notes

- `DiscoverLooms` use case stays in the codebase — it's still called at startup by `run_startup()`. Only the HTTP endpoint is removed.
- The `ConfigEvent` channel is separate from the `StrandEvent` channel. They never mix. The debounce engine only sees strand events.
- Config event handling is synchronous (no debounce needed) — file changes are immediate and the handler processes them one at a time.
- The `LoomStore` is the single source of truth for in-memory state. Both the config handler and the HTTP handlers update it directly.
- Idempotency: if the HTTP handler writes a file AND the config handler also sees the file change, the second update is a no-op (the store already matches). This is safe because `LoomStore::register()` overwrites.
