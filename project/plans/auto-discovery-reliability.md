# Plan: Auto-Discovery Reliability Fixes

## Problem

The auto-discovery feature from Plan #14 (Loom/Knot Auto-Discovery) has several reliability defects that prevent new looms from being discovered at runtime without restart. Four defects were identified in production use:

1. **Relative path mismatch in rig watch** — `run_startup()` registers the rig watch with a non-canonicalised path (e.g. `./rig`). Notify reports absolute paths. `find_watch_types()` uses `path.starts_with(dir)` which fails when the watch path is relative and the event path is absolute. The rig watch is effectively dead.

2. **Full rig re-scan on `ConfigEvent::LoomAdded`** — `ConfigEventHandler::handle_loom_added()` re-scans the entire rig directory to find the new loom. This races with filesystem writes (the loom directory may not be fully written when notify fires), and is wasteful for large rigs.

3. **`ConfigEvent::LoomAdded` carries only ID, not path** — The event loses the filesystem path of the new loom directory, forcing the handler to derive it and re-scan the full rig.

4. **`try_send` silently drops config events** — If the config channel is full (100 capacity), the `LoomAdded` event is dropped with no recovery path. There may be no "next event" to trigger re-processing.

The result: creating a new loom directory while Knot is running can silently fail — the loom won't appear in `GET /looms` and won't process any strands.

## Target

When this plan is done:

- New looms are discovered reliably within ~500ms of directory creation
- Path canonicalisation is consistent between watch registration and notify event reporting
- `handle_loom_added` scans only the new loom directory (not the full rig)
- `ConfigEvent::LoomAdded` carries the loom directory path for targeted scanning
- `POST /config/reload` endpoint provides manual recovery when the watcher misses an event

## Implementation Status: ⬜ Draft

## Hex Layers

- **Domain** — Enrich `ConfigEvent::LoomAdded` with loom directory path
- **Outbound Adapters** — Canonicalise paths in watch registration; propagate loom path in events
- **Application** — Targeted directory scan in `handle_loom_added`; `ReloadConfig` use case for manual reload
- **Inbound Adapters** — `POST /config/reload` handler
- **Composition Root** — Canonicalise `rig_dir` before watch registration

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `domain::events::tests::config_event_types` | ConfigEvent variants serialise round-trip | ✅ Green — `LoomAdded` carries only `LoomId` |
| `application::usecases::tests::config_handler_loom_added` | Handler scans rig, registers loom | ✅ Green — will change (scan specific dir) |
| `application::usecases::tests::config_handler_knot_*` | Knot CRUD via config events | ✅ Green — no change |
| `application::usecases::tests::discover_looms_*` | DiscoverLooms at startup | ✅ Green — no change |
| `outbound::event_source::tests::rig_dir_new_loom_emits_config_event` | Rig watch emits LoomAdded | ✅ Green — will change (includes loom_dir) |
| `outbound::event_source::tests::rig_dir_non_loom_directory_ignored` | Non-loom dirs ignored | ✅ Green — no change |
| `outbound::event_source::tests::register_watch_idempotent_for_same_knot` | Watch dedup | ✅ Green — no change |
| `outbound::event_source::tests::two_knots_same_directory_both_receive_events` | Multi-knot fanout | ✅ Green — no change |
| `integration::tests::auto_discovery_and_knot_crud::runtime_loom_auto_discovery` | End-to-end auto-discovery | ✅ Green — **flaky** (path mismatch) |

## Test Gaps

- No test for canonical path matching between watch registration and notify events
- No test for `handle_loom_added` scanning a specific directory
- No test for `ConfigEvent::LoomAdded` carrying `loom_dir: PathBuf`
- No test for `ReloadConfig` use case
- No test for `POST /config/reload` handler
- No integration test with relative rig path (all integration tests use absolute tempfile paths, masking the path mismatch)

## Phases

### Phase 0: Domain — Enrich `ConfigEvent::LoomAdded` with directory path

**Failing test:** `domain::events::tests::config_event_loom_added_has_path`

**Hex Layer:** Domain

Add `loom_dir: PathBuf` to `ConfigEvent::LoomAdded` so the handler knows exactly which directory to scan.

```rust
LoomAdded {
    loom_id: LoomId,
    /// Absolute path to the loom directory (e.g. `/project/rig/new-loom`).
    /// Used by `ConfigEventHandler` to scan only this directory instead of
    /// re-scanning the full rig.
    loom_dir: PathBuf,
},
```

- [ ] Failing test: `config_event_loom_added_has_path` — `LoomAdded` carries `loom_id` and `loom_dir`, serialises round-trip
- [ ] Implement: update `ConfigEvent::LoomAdded` variant in `src/domain/events.rs`
- [ ] Update existing `config_event_types` test to verify new variant shape
- [ ] All domain tests green

### Phase 1: Outbound Adapters — Canonicalise watch paths + propagate loom path

**Failing tests:** `outbound::event_source::tests::rig_watch_path_canonicalised`, `outbound::event_source::tests::rig_loom_added_event_includes_path`

**Hex Layer:** Outbound Adapters

Two changes in `NotifyEventSource`:

1. **Canonicalise the rig directory path** when `register_watch()` is called with `WatchType::Rig`, so `find_watch_types()` matches against absolute paths (which is what notify reports).

2. **Propagate the loom directory path** in the emitted `ConfigEvent::LoomAdded` event.

```rust
// In map_rig_event():
Some(ConfigEvent::LoomAdded {
    loom_id,
    loom_dir: path.to_path_buf(), // absolute path from notify event
})
```

- [ ] Failing test: `rig_watch_path_canonicalised` — register watch with relative path → stored as absolute in watched_dirs
- [ ] Failing test: `rig_loom_added_event_includes_path` — create `*-loom` dir → emitted event includes `loom_dir`
- [ ] Implement: canonicalise path in `register_watch()` for `WatchType::Rig`
- [ ] Implement: `loom_dir` field in `map_rig_event()` → `ConfigEvent::LoomAdded`
- [ ] Update existing `rig_dir_new_loom_emits_config_event` test
- [ ] All outbound adapter tests green

### Phase 2: Application — `handle_loom_added` scans only the new loom directory

**Failing tests:** `application::usecases::tests::config_handler_loom_added_scans_specific_dir`, `application::usecases::tests::config_handler_loom_added_dir_missing`

**Hex Layer:** Application

Replace the full rig re-scan with a targeted scan of the new loom directory.

```rust
fn handle_loom_added(&self, loom_id: &LoomId, loom_dir: PathBuf) -> Result<(), PortError> {
    // Scan only this loom directory (not the full rig)
    let (knots, warnings) = self.repository.scan_knot_files(&loom_dir)?;
    // Resolve paths and build Loom directly
    let loom = Loom { id: loom_id.clone(), knots };
    self.register_loom(&loom, &warnings)
}
```

`LoomRepository` trait needs a new method:

```rust
pub trait LoomRepository {
    // ...existing methods...
    /// Scan a single directory for .md knot definition files.
    /// Returns parsed knots and any warnings.
    fn scan_knot_files(&self, path: &Path) -> Result<(Vec<Knot>, Vec<String>), PortError>;
}
```

- [ ] Failing test: `config_handler_loom_added_scans_specific_dir` — handler scans only the new loom dir
- [ ] Failing test: `config_handler_loom_added_dir_missing` — loom dir doesn't exist → returns error
- [ ] Implement: `scan_knot_files` on `LoomRepository` trait in `application/ports.rs`
- [ ] Implement: `scan_knot_files` on `FileSystemLoomRepository` (delegate to existing method)
- [ ] Implement: `scan_knot_files` on `MockLoomRepository` (return empty vec)
- [ ] Implement: `handle_loom_added()` uses `scan_knot_files(loom_dir)` + builds `Loom` directly
- [ ] Update existing config handler tests
- [ ] All application tests green

### Phase 3: Application — `ReloadConfig` Use Case

**Failing tests:** `application::usecases::tests::reload_config_discovers_new_looms`, `application::usecases::tests::reload_config_skips_registered`

**Hex Layer:** Application

Use case that re-runs loom discovery on demand. Provides the business logic for the manual reload endpoint.

```rust
pub struct ReloadConfig {
    repository: Arc<dyn LoomRepository>,
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
    rig_dir: PathBuf,
}

impl ReloadConfig {
    /// Re-scan the rig and register any looms not already in the store.
    /// Returns the list of newly registered loom IDs.
    pub fn execute(&self) -> Result<Vec<LoomId>, PortError> {
        // Reuse DiscoverLooms logic
    }
}
```

This is essentially `DiscoverLooms::execute()` called on demand. We'll create a thin `ReloadConfig` that delegates to `DiscoverLooms` to avoid duplication.

- [ ] Failing test: `reload_config_discovers_new_looms` — new looms on disk are registered
- [ ] Failing test: `reload_config_skips_registered` — existing looms are not re-registered
- [ ] Implement: `ReloadConfig` use case in `src/application/usecases.rs`
- [ ] All application tests green

### Phase 4: Inbound Adapters — `POST /config/reload` Endpoint

**Failing tests:** `inbound::tests::post_config_reload_success`, `inbound::tests::post_config_reload_no_new_looms`

**Hex Layer:** Inbound Adapter

Manual trigger for re-running loom discovery. Provides recovery when the file watcher misses an event.

| Method | Path | Response |
|--------|------|----------|
| `POST` | `/config/reload` | `200` — JSON array of `LoomSummary` for newly discovered looms |

- [ ] Failing test: `post_config_reload_success` — `POST /config/reload` → 200 with new looms
- [ ] Failing test: `post_config_reload_no_new_looms` — no new looms → 200 with empty array
- [ ] Implement: `reload_config` handler in `src/adapters/inbound/system.rs`
- [ ] Wire route: `.route("/config/reload", post(reload_config))` in `router.rs`
- [ ] Add OpenAPI annotation
- [ ] All inbound tests green

### Phase 5: Integration Tests — Runtime Discovery and Manual Reload

**Failing tests:** `integration::tests::auto_discovery_with_absolute_rig_path`, `integration::tests::manual_config_reload_discovers_new_looms`

**Hex Layer:** Integration

- [ ] Failing test: `auto_discovery_with_absolute_rig_path` — start server with absolute rig path → create loom dir → `GET /looms` shows new loom → create strand → tie-off produced
- [ ] Failing test: `manual_config_reload_discovers_new_looms` — create loom dir → `POST /config/reload` → new loom in `GET /looms`
- [ ] Full integration test suite green

### Phase 6: Composition Root — Canonicalise `rig_dir` before watch registration

**Hex Layer:** Composition Root

`run_startup()` in `server.rs` calls `register_watch(rig_dir, WatchType::Rig)`. Canonicalise `rig_dir` before registration so the stored watch path matches notify's reported paths.

- [ ] Implement: `fs::canonicalize(rig_dir)` in `run_startup()` before `register_watch()`
- [ ] Build succeeds, all tests pass

## Notes

- Plan #14 (Loom/Knot Auto-Discovery) delivered the auto-discovery feature. This plan fixes reliability defects found in production use.
- The canonical path mismatch (Issue 1) is the most impactful bug — it means the rig watch is effectively dead when Knot is started with a relative path (which is the default: `./rig`).
- The `POST /config/reload` endpoint is a safety net, not the primary discovery mechanism. The file watcher should handle most cases once the path canonicalisation is fixed.
- `DiscoverLooms` use case stays in the codebase — it's used at startup by `run_startup()` and reused by `ReloadConfig`.
