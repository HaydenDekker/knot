# Plan: Fix `unwatch()` Removing Watchers for Other Knots

## Problem

`NotifyEventSource::unwatch()` removes **all** entries for a given path from `watched_dirs`:

```rust
inner.watched_dirs.retain(|(p, _)| p != &canonical_path);
```

This is too broad — `watched_dirs` can hold multiple entries for the same path (different knots watching the same strand directory). When `handle_knot_modified` or `handle_knot_deleted` calls `unwatch(&old_strand_dir)`, it wipes the watcher entries for **every knot** watching that directory, not just the one being modified/deleted.

`register_watch()` already handles this correctly — it deduplicates only the exact `(path, WatchType)` pair. `unwatch()` mirrors this pattern incorrectly by only checking the path.

## Target

`unwatch()` only removes the entry matching the specific `(path, WatchType)` pair. The `notify::unwatch()` call (which stops the underlying file system watch) only fires when the **last** watcher entry for a path is removed, so the remaining knot watchers continue receiving events.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test | File | What it covers | Status |
|------|------|---------------|--------|
| `watcher_starts` | `event_source.rs` | `unwatch()` returns `Ok` | ✅ Green — but only tests single-watch scenario |
| `unwatch_loom_removes_all_knot_watchers` | `usecases.rs` (~line 3447) | `UnregisterLoom` calls `unwatch()` for each knot | ✅ Green — tests integration, not multi-knot same-dir |
| `knot_modified_strand_dir_change` | `usecases.rs` (~line 2310) | `KnotModified` strand_dir change calls `unwatch(old)` then `watch(new)` | ✅ Green — single knot, doesn't test shared dir |
| `knot_deleted_stops_watcher` | `usecases.rs` (~line 2632) | `KnotDeleted` calls `unwatch()` | ✅ Green — single knot, doesn't test shared dir |

## Test Gaps

- No test for two knots sharing the same `strand_dir` where one knot is modified (strand_dir change) or deleted — the scenario that triggers the bug
- No test that verifies `unwatch()` only removes the matching entry, not all entries at a path
- No test that verifies the underlying `notify::unwatch()` is only called when the last watcher for a path is removed

## Phases

### Phase 0: Failing Test — Multi-Knot Shared Strand Directory

- [ ] Add unit test to `event_source.rs` `mod tests`: two knots (same loom, different knot IDs) registered with the same `strand_dir`. Call `unwatch_with_type(path, Strand(loom, knot-a))`. Verify: knot-a entry removed from `watched_dirs`, knot-b entry still present, `notify::unwatch()` NOT called (since knot-b still watches the path).
- [ ] Test uses the `NotifyEventSource` directly with `register_watch` + `watch` + the new `unwatch_with_type` method
- [ ] Also add a test: after removing the **last** watcher for a path, `notify::unwatch()` IS called

### Phase 1: Add `unwatch_with_type` to `EventSource` Trait

- [ ] Add `fn unwatch_with_type(&self, path: &Path, watch_type: WatchType) -> Result<(), PortError>` to the `EventSource` trait in `ports.rs`
- [ ] Default implementation: delegates to `unwatch(path)` for backward compatibility with mock implementations in tests
- [ ] Update the mock `EventSource` in `ports.rs` tests (if any)
- [ ] Compile check

### Phase 2: Implement `unwatch_with_type` in `NotifyEventSource`

- [ ] Implement `unwatch_with_type` in `NotifyEventSource`:
  - Canonicalise path
  - Remove only the entry matching `(canonical_path, watch_type)` using `watch_types_equal` (same pattern as `register_watch`)
  - Check if any other entries remain for this path
  - Only call `notify::unwatch()` if no other entries remain
  - Log the unwatch event
- [ ] Verify Phase 0 tests pass
- [ ] Compile check + clippy clean

### Phase 3: Update Callers in `usecases.rs`

- [ ] `handle_knot_modified` (line ~1549): change `self.event_source.unwatch(&old_strand_dir)` to `self.event_source.unwatch_with_type(&old_strand_dir, WatchType::Strand(loom_id.clone(), knot_id.clone()))`
- [ ] `handle_knot_deleted` (line ~1650): same — pass `WatchType::Strand(loom_id, knot_id)` so only the deleted knot's entry is removed
- [ ] `UnregisterLoom` (line ~436): can keep calling `unwatch()` since all knots in the loom are being removed, OR change to `unwatch_with_type` for consistency. Leave as-is since the existing behaviour (removing all entries) is correct when the entire loom is being unregistered.
- [ ] Run all existing tests — verify no regressions (existing tests use mock `EventSource` which delegates to `unwatch`)
- [ ] Compile check + clippy clean

### Phase 4: Integration Test — Multi-Knot Shared Directory

- [ ] Add integration test in `tests/pipeline.rs` (or a new test file if appropriate): two knots sharing a strand directory. Modify one knot's strand_dir. Verify: the other knot still processes strand events from the shared directory.
- [ ] This is the end-to-end verification of the bug fix
- [ ] Run full test suite — all tests pass

## Hexagonal Layers

| Layer | File | Change |
|-------|------|--------|
| Port | `src/application/ports.rs` | New `unwatch_with_type` trait method |
| Use Case | `src/application/usecases.rs` | Call `unwatch_with_type` in `handle_knot_modified`, `handle_knot_deleted` |
| Outbound Adapter | `src/adapters/outbound/event_source.rs` | Implement `unwatch_with_type` — targeted entry removal + conditional notify unwatch |
| Domain | None | No domain changes needed |

## Notes

- The root cause: `unwatch()` uses `retain(|(p, _)| p != &canonical_path)` which matches by path only. `register_watch()` correctly uses `(path, watch_type)` equality. The two methods are asymmetrical.
- `notify::watch()` can be called multiple times on the same path without error. `notify::unwatch()` removes the entire watch for a path — it cannot target a specific "instance". This is why the fix must only call `notify::unwatch()` when the last entry for a path is removed.
