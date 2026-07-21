# Phase 4: Integration Test — Full Persistence Cycle

## Status

Completed

## Summary

Created end-to-end integration tests verifying the disk-backed event queue works
correctly with real filesystem state. Also completed Phase 3b (swap server from
`InMemoryEventQueue` to `DiskBackedEventQueue`), which is a prerequisite.

## Changes

### Phase 3b: Swap to DiskBackedEventQueue (prerequisite)

**`src/server.rs`:**
- Replaced `InMemoryEventQueue` with `DiskBackedEventQueue` in
  `start_event_pipeline()`
- Events directory: `rig/events/` created automatically before queue construction
- `load_persisted()` called at startup (before debounce engine starts), logs
  count of recovered events

### Phase 4: Integration Tests

**New file:** `tests/persistent_queue.rs`

8 integration tests covering:

| Test | What it verifies |
|---|---|
| `full_cycle_push_pop_disk_empty` | Push creates `.json` file on disk, pop removes it |
| `restart_survival_events_load_after_recreation` | Events survive queue recreation (simulated crash + restart) |
| `malformed_file_handling_skips_bad_json` | Invalid JSON files are skipped with warning, valid events still process |
| `empty_events_directory_clean_startup` | Clean startup with empty events dir works correctly |
| `multiple_events_five_pushed_processed` | 5 events: all files created, all processed, disk empty after |
| `delete_from_queue_removes_event` | `delete(id)` removes both queue entry and disk file |
| `modify_on_disk_processed_with_updated_content` | On-disk edits are honoured on pop (disk is source of truth) |
| `non_json_files_ignored_in_events_directory` | Non-`.json` files in events dir are silently ignored |

## Test Results

```
running 8 tests
test empty_events_directory_clean_startup      ... ok
test delete_from_queue_removes_event           ... ok
test non_json_files_ignored_in_events_directory ... ok
test full_cycle_push_pop_disk_empty            ... ok
test modify_on_disk_processed_with_updated_content ... ok
test malformed_file_handling_skips_bad_json     ... ok
test multiple_events_five_pushed_processed     ... ok
test restart_survival_events_load_after_recreation ... ok

test result: ok. 8 passed; 0 failed
```

Full suite verification:
- `cargo test --lib`: 725 passed
- `cargo test --test smoke`: 11 passed (composition with disk queue)
- `cargo test --test pipeline`: 21 passed
- `cargo test --test persistent_queue`: 8 passed

## Notes

- Tests use `DiskBackedEventQueue` directly (not through `start_knot()`) because
  the queue is the persistence layer and its disk behaviour is independent of
  the debounce engine timing.
- `load_persisted()` is called at startup in `start_event_pipeline()` before
  the debounce engine begins, ensuring persisted events are re-queued before
  new file-watcher events arrive.
- Event files are verified by checking `rig/events/*.json` file existence and
  content — the disk IS the queue, so file presence = queue membership.
- Malformed files are not auto-cleaned (user responsibility to remove them).
- Non-`.json` files (`.tmp`, `.md`, `.*`) are silently ignored by scan.
