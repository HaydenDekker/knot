# Plan: Loom Discovery and State Files

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan defines the application layer: ports (traits), use cases, debounce logic, and the processing state machine. All tests use mock ports — no IO.

## Problem

Knot has domain types but no application layer. There are no ports defining what adapters must implement, no use cases orchestrating behaviour, and no debounce logic or state machine for the processing pipeline. Without this layer, adapters have no contracts to satisfy and domain types have no orchestration.

## Target

- Ports defined as traits in application layer: `LoomRepository`, `KnotStatePort`, `LoomLogPort`, `EventSource`, `AgentRunner`, `TieOffSink`
- Use cases: `RegisterLoom`, `UnregisterLoom`, `DiscoverLooms`, `ProcessStrand`, `ListLooms`, `GetLoom`, `GetLoomActivity`, `GetKnotStatus`
- Debounce logic in application layer (per-file timer, 100ms window)
- Processing state machine: `Idle → Processing → Completed | Failed`
- Loom store (in-memory registry) backed by ports, not concrete adapters

## Implementation Status: ✅ Complete (2026-06-03)

## Hex Layer: Application

Defines ports (traits). Orchestrates domain entities. Tests use mock implementations of ports. No knowledge of axum, notify, tokio::process, or std::fs.

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |
| `domain::*` tests | Domain entities, value objects, events | Plan 1 |

## Test Gaps

- No port interface tests (trait contract verification).
- No use case tests with mock ports.
- No debounce logic tests.
- No state machine tests.
- No loom store tests.

## Port Interfaces

| Port | Method Signature | Purpose |
|------|------------------|---------|
| `LoomRepository` | `scan(rig: &Path) -> Result<Vec<Loom>>`, `get(id: &LoomId) -> Result<Option<Loom>>`, `list() -> Result<Vec<Loom>>`, `save(loom: Loom) -> Result<()>` | Discover and persist looms |
| `KnotStatePort` | `create(knot_id: &KnotId) -> Result<()>`, `update(state: KnotState) -> Result<()>`, `get(knot_id: &KnotId) -> Result<Option<KnotState>>` | CRUD knot processing state |
| `LoomLogPort` | `open(loom_id: &LoomId) -> Result<()>`, `append(event: LoomEvent) -> Result<()>`, `read_all(loom_id: &LoomId) -> Result<Vec<LoomEvent>>` | Append/query loom activity log |
| `EventSource` | `watch(path: &Path) -> Result<()>`, `unwatch(path: &Path) -> Result<()>` | Register/unregister watched directories (events flow through a channel, not via this port) |
| `AgentRunner` | `execute(ctx: ExecutionContext) -> Result<AgentOutput>` | Run the agent CLI and capture output |
| `TieOffSink` | `write(tie_off: TieOff) -> Result<()>` | Write tie-off content to disk |

## Phases

### Phase 0: Port Traits
**Failing tests created:** `application::ports::tests::loom_repository_contract`, `application::ports::tests::knot_state_port_contract`, `application::ports::tests::loom_log_port_contract`, `application::ports::tests::agent_runner_contract`, `application::ports::tests::tieoff_sink_contract`

- [x] Failing test: `application::ports::tests::loom_repository_contract` — mock `LoomRepository` implements all trait methods; verify trait is object-safe and all methods compile
- [x] Failing test: `application::ports::tests::knot_state_port_contract` — mock `KnotStatePort` implements `create`, `update`, `get`; verify trait compiles
- [x] Failing test: `application::ports::tests::loom_log_port_contract` — mock `LoomLogPort` implements `open`, `append`, `read_all`; verify trait compiles
- [x] Failing test: `application::ports::tests::agent_runner_contract` — mock `AgentRunner` implements `execute`; verify `ExecutionContext` and `AgentOutput` types exist
- [x] Failing test: `application::ports::tests::tieoff_sink_contract` — mock `TieOffSink` implements `write`; verify trait compiles
- [x] Define port traits in `src/application/ports.rs`
- [x] Define supporting types: `KnotState` (event_type, strand_path, tie_off_path, status, error, last_updated), `ExecutionContext` (cli_path, cli_args, prompt, strand_path), `AgentOutput` (stdout, stderr, exit_code)
- [x] Define error types: `PortError` with variants for each operation

### Phase 1: Loom Store
**Failing tests created:** `application::store::tests::register_loom`, `application::store::tests::list_looms`, `application::store::tests::get_loom_by_id`, `application::store::tests::get_nonexistent_returns_none`, `application::store::tests::unregister_loom`

- [x] Failing test: `application::store::tests::register_loom` — register a loom in the store; verify it appears in `list()`
- [x] Failing test: `application::store::tests::list_looms` — list returns all registered looms
- [x] Failing test: `application::store::tests::get_loom_by_id` — get existing loom returns `Some(loom)`
- [x] Failing test: `application::store::tests::get_nonexistent_returns_none` — get unknown ID returns `None`
- [x] Failing test: `application::store::tests::unregister_loom` — unregister removes loom from store; `get()` returns `None`
- [x] Implement `LoomStore` — in-memory registry using `Arc<RwLock<HashMap<LoomId, Loom>>>`
- [x] `LoomStore` depends on ports (traits), not concrete adapters
- [x] Methods: `register(loom: Loom)`, `unregister(id: &LoomId)`, `get(id: &LoomId)`, `list()`

### Phase 2: Discover Looms Use Case
**Failing tests created:** `application::usecases::tests::discover_looms_success`, `application::usecases::tests::discover_looms_empty_workspace`, `application::usecases::tests::discover_looms_repository_error`

- [x] Failing test: `application::usecases::tests::discover_looms_success` — given a mock `LoomRepository` returning 2 looms, `DiscoverLooms` registers them in `LoomStore` and returns 2 looms
- [x] Failing test: `application::usecases::tests::discover_looms_empty_rig` — repository returns empty vec; store remains empty; use case returns empty vec
- [x] Failing test: `application::usecases::tests::discover_looms_repository_error` — repository returns error; use case propagates error without modifying store
- [x] Implement `DiscoverLooms` use case: calls `LoomRepository::scan()`, iterates results, calls `KnotStatePort::create()` for each knot, calls `LoomLogPort::append(KnotRegistered)` for each knot, registers looms in `LoomStore`

### Phase 3: Register and Unregister Loom Use Cases
**Failing tests created:** `application::usecases::tests::register_loom_creates_state_files`, `application::usecases::tests::register_loom_duplicate_id_error`, `application::usecases::tests::unregister_loom_logs_stopped_event`

- [x] Failing test: `application::usecases::tests::register_loom_creates_state_files` — register loom calls `LoomLogPort::open()`, `KnotStatePort::create()` for each knot, `LoomLogPort::append(LoomStarted)`, then stores loom
- [x] Failing test: `application::usecases::tests::register_loom_duplicate_id_error` — register loom with existing ID returns error without side effects
- [x] Failing test: `application::usecases::tests::unregister_loom_logs_stopped_event` — unregister calls `LoomLogPort::append(LoomStopped)`, removes from store
- [x] Implement `RegisterLoom` and `UnregisterLoom` use cases

### Phase 4: Query Use Cases (ListLooms, GetLoom, GetLoomActivity, GetKnotStatus)
**Failing tests created:** `application::usecases::tests::list_looms_returns_summaries`, `application::usecases::tests::get_loom_by_id`, `application::usecases::tests::get_loom_activity_from_log`, `application::usecases::tests::get_knot_status_from_state`

- [x] Failing test: `application::usecases::tests::list_looms_returns_summaries` — `ListLooms` reads from `LoomStore::list()`; returns loom summaries
- [x] Failing test: `application::usecases::tests::get_loom_by_id` — `GetLoom` reads from store by ID; returns full loom or error if missing
- [x] Failing test: `application::usecases::tests::get_loom_activity_from_log` — `GetLoomActivity` calls `LoomLogPort::read_all()`; returns log entries
- [x] Failing test: `application::usecases::tests::get_knot_status_from_state` — `GetKnotStatus` calls `KnotStatePort::get()`; returns state or error
- [x] Implement query use cases — all read through ports or store

### Phase 5: Debounce Logic
**Failing tests created:** `application::debounce::tests::single_event_emits_after_window`, `application::debounce::tests::rapid_events_emit_only_last`, `application::debounce::tests::different_files_emit_independently`, `application::debounce::tests::delete_after_modify_emits_delete`

- [x] Failing test: `application::debounce::tests::single_event_emits_after_window` — feed one event; after 100ms it is emitted on the output channel
- [x] Failing test: `application::debounce::tests::rapid_events_emit_only_last` — feed 5 events for same file within 50ms; only the 5th is emitted after debounce window
- [x] Failing test: `application::debounce::tests::different_files_emit_independently` — feed events for file A and file B; both emit independently (not blocked on each other)
- [x] Failing test: `application::debounce::tests::delete_after_modify_emits_delete` — feed Modify then Delete for same file within window; only Delete is emitted
- [x] Implement `DebounceEngine` — per-file `tokio::time::Instant` tracker with 100ms window
- [x] Takes raw `StrandEvent`s on input channel, emits debounced events on output channel
- [x] Runs as a `tokio::task`; provides `start() -> (Sender, Receiver, JoinHandle)`
- [x] **Design note:** debounce is an application concern (orchestration), not part of the `EventSource` adapter. The adapter emits raw events; the engine filters them.

### Phase 6: ProcessStrand Use Case and State Machine
**Failing tests created:** `application::usecases::tests::process_strand_success`, `application::usecases::tests::process_strand_agent_error`, `application::usecases::tests::process_strand_state_transitions`, `application::usecases::tests::process_strand_deleted_event`

- [x] Failing test: `application::usecases::tests::process_strand_success` — given mock `AgentRunner` returning success, mock `TieOffSink`, mock `KnotStatePort`: verify state transitions `idle → processing → completed`, tie-off written, loom-log appended
- [x] Failing test: `application::usecases::tests::process_strand_agent_error` — mock `AgentRunner` returns error: verify state transitions `idle → processing → failed`, error tie-off written, knot-state has error details
- [x] Failing test: `application::usecases::tests::process_strand_state_transitions` — verify exact state sequence: initial `idle`, then `processing` before agent call, then `completed` or `failed` after
- [x] Failing test: `application::usecases::tests::process_strand_deleted_event` — for `StrandEvent::Deleted`, tie-off still written (reports what was undone), previous tie-off never deleted
- [x] Implement `ProcessStrand` use case:
  1. Receive `StrandEvent`
  2. Update knot-state to `processing`
  3. Build execution context from `RigAgentConfig` + `Knot`
  4. Call `AgentRunner::execute()`
  5. Call `TieOffSink::write()` with result
  6. Update knot-state to `completed` or `failed`
  7. Append to loom-log

## Notes
