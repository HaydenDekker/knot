# Phase 8: Application test migration — auto-discovery and CRUD

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [x] Remove `tests/auto_discovery_and_knot_crud.rs` (9 composition tests)
- [x] Verify all 9 scenarios covered by existing tests:
  - [x] `auto_discover_new_loom` → `config_handler_loom_added` (unit)
  - [x] `auto_discover_new_knot` → `config_handler_knot_added` (unit)
  - [x] `auto_detect_knot_deletion` → `config_handler_knot_deleted` (unit)
  - [x] `auto_detect_loom_deletion` → `config_handler_knot_deleted` × N (unit) + NotifyEventSource adapter test
  - [x] `auto_detect_knot_modification` → `config_handler_knot_modified` (unit)
  - [x] `auto_discover_multiple_looms_sequentially` → `config_handler_loom_added` (unit)
  - [x] `loom_log_records_discovery_events` → NotifyEventSource adapter tests + ConfigEventHandler unit tests
  - [x] `auto_discover_rapid_loom_creations` → NotifyEventSource adapter tests (debounce coalescing)
  - [x] `auto_discovery_is_idempotent` → `config_handler_loom_added_already_registered` (unit)
- [x] Verify: `cargo test` passes, 0 regressions

## Deviations

## Discoveries

## Notes

### Coverage mapping

All 9 composition tests in `auto_discovery_and_knot_crud.rs` were verified against existing unit and adapter tests:

| Removed integration test | Covered by |
|---|---|
| `auto_discover_new_loom` | `config_handler_loom_added` (unit — `src/application/usecases/config_event_handler.rs`) + NotifyEventSource adapter test `watch_starts_notify_watching` (`tests/adapters.rs`) |
| `auto_discover_new_knot` | `config_handler_knot_added` (unit) |
| `auto_detect_knot_deletion` | `config_handler_knot_deleted` (unit) |
| `auto_detect_loom_deletion` | `config_handler_knot_deleted` × N (unit — each knot in the loom is deleted independently) + NotifyEventSource adapter test `file_delete_emits_event` (`tests/adapters.rs`) |
| `auto_detect_knot_modification` | `config_handler_knot_modified` (unit) + `config_handler_knot_modified_same_strand_dir` (unit) |
| `auto_discover_multiple_looms_sequentially` | `config_handler_loom_added` (unit — `LoomStore` is shared across calls, proving sequential registration works) |
| `loom_log_records_discovery_events` | `config_handler_loom_added` (unit — asserts `KnotRegistered` + `LoomStarted` in log) + `loom_log_adapter::append_writes_jsonl_entry` + `read_all_returns_parsed_events` (adapter — `tests/adapters.rs`) |
| `auto_discover_rapid_loom_creations` | NotifyEventSource adapter tests (`tests/adapters.rs`) + debounce unit test `rapid_events_emit_only_last` (`src/application/debounce.rs`) |
| `auto_discovery_is_idempotent` | `config_handler_loom_added_already_registered` (unit — verifies skip when loom already in store) |

The integration tests verified the full file-watcher pipeline (fs → notify → config handler → state). This same pipeline is now covered at each layer:
- **File watcher → event channel**: NotifyEventSource adapter tests (`tests/adapters.rs`)
- **Event handling logic**: ConfigEventHandler unit tests (mock ports, isolated)
- **End-to-end composition**: `tests/smoke.rs` (full Knot start + loom discovery + strand processing)
- **State persistence**: FileSystemStateWriter adapter tests (`tests/adapters.rs`)
