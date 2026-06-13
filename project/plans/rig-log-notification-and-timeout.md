# Plan: Rig-Log Notification and Timeout Handling

## Related PRD

This plan contributes to [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md).

It implements Story 6 (Rig-Log Notification) and the timeout/error handling changes that support Story 5 (Event Replay — user touches file to reprocess). The rig-log surfaces serious operational events (timeouts, queue idle) so the user or an external agent can monitor and react. On timeout, the tie-off is preserved unchanged (no error appended).

## Problem

1. **No rig-level notification** — when an agent session times out, the user has no way to discover it without polling HTTP endpoints or inspecting individual loom-logs. There is no single file that an external watcher (human or LLM agent) can monitor for "something needs attention."

2. **Timeout errors pollute the tie-off** — currently `ProcessStrand::execute` writes `Processing failed: ...` into the tie-off file on any error (including timeout). The tie-off is the agent's output and should contain only agent-produced content. Operational errors belong in logs.

## Target

1. **Rig-log** (`rig/.rig-log`) — append-only JSONL file recording `TimeoutExceeded` and `QueueIdle` events. Survives server restarts. Multiple consumers can watch it safely.

2. **Timeout handling** — on `PortError::Timeout`, `ProcessStrand` writes to loom-log and rig-log only, **not** to the tie-off file. Previous tie-off content is preserved unchanged.

3. **Queue idle** — after processing completes and no events are pending, a `QueueIdle` entry is written to the rig-log so the user knows the system is quiet.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test File | What it covers | Status |
|-----------|---------------|--------|
| `tests/tie_off.rs` | Full tie-off lifecycle, append-mode sections, markdown structure | ✅ Green — 2 integration tests |
| `tests/agent_integration.rs` | Mock agent, pi stub, error paths, non-zero exit codes | ✅ Green |
| `tests/pipeline.rs` | Event pipeline, debounce, concurrent requests | ✅ Green |
| `tests/task_management.rs` | ProcessStrand use case, loom-log lifecycle | ✅ Green |
| `src/adapters/outbound/loom_log.rs` | LoomLogPort file-system impl, concurrent writes | ✅ Green |
| `src/adapters/outbound/tieoff_sink.rs` | Tie-off write, overwrite, append-mode, history | ✅ Green |
| `src/application/usecases.rs` | ProcessStrand unit tests with mock ports | ✅ Green |
| `src/domain/events.rs` | LoomEvent serialisation round-trips | ✅ Green |

## Test Gaps

- No test for timeout error path in `ProcessStrand` (the mock agent never times out)
- No test for rig-log creation and append
- No test for queue idle detection
- No integration test for the full timeout → rig-log → user observes flow
- No test for tie-off preservation on timeout (tie-off should NOT receive error content)

## Phases

### Phase 0: Domain — RigLogEvent and RigLogPath types

Hex layer: **Domain**

New domain types for the rig-log feature:

- `RigLogPath` — value object wrapping `PathBuf` (location: `rig/.rig-log`)
- `RigLogEvent` — enum with two variants:
  - `TimeoutExceeded { loom_id, knot_id, strand_path, error, timestamp }`
  - `QueueIdle { timestamp }`

Both variants are `serde::Serialize + Deserialize + Clone + Debug + PartialEq + Eq`.

**Tests:**
- Unit tests in `src/domain/events.rs` for `RigLogEvent` serialisation round-trips (all variants)
- Unit test for `RigLogPath` construction

**Tasks:**
- [ ] Add `RigLogPath` to `src/domain/entities.rs`
- [ ] Add `RigLogEvent` enum to `src/domain/events.rs`
- [ ] Add serialisation round-trip tests for all `RigLogEvent` variants
- [ ] Run `cargo test` — all existing tests still pass

### Phase 1: Application — RigLogPort

Hex layer: **Application (ports)**

New port trait:

- `RigLogPort` — `append(event: RigLogEvent)` and `read_all() -> Vec<RigLogEvent>`
- Add `PortError::RigLogWriteFailed` and `PortError::RigLogReadFailed` error variants

**Tests:**
- Unit tests verifying new `PortError` variants implement `Display` and `Error`
- Unit tests for mock `RigLogPort`

**Tasks:**
- [ ] Add `RigLogPort` trait to `src/application/ports.rs`
- [ ] Add new `PortError` variants and `Display` implementations
- [ ] Add mock `RigLogPort` in test module
- [ ] Run `cargo test` — all existing tests still pass

### Phase 2: Outbound Adapters — FileSystemRigLog

Hex layer: **Outbound adapters**

Concrete filesystem implementation:

- `FileSystemRigLog` — writes `RigLogEvent` as JSONL to `rig/.rig-log` (append mode, creates parent dirs)

**Tests:**
- `FileSystemRigLog::new`, `append`, `read_all` — creates file, appends JSONL, reads back
- `FileSystemRigLog` concurrent writes — multiple threads append safely

**Tasks:**
- [ ] Create `src/adapters/outbound/rig_log.rs` with `FileSystemRigLog`
- [ ] Implement `RigLogPort` for `FileSystemRigLog`
- [ ] Add unit tests for `FileSystemRigLog` (create, append, read_all, concurrent writes)
- [ ] Export in `src/adapters/outbound/mod.rs`
- [ ] Run `cargo test` — all tests pass

### Phase 3: ProcessStrand — Timeout handling change + rig-log writes

Hex layer: **Application (use cases)** + **Composition root**

Change `ProcessStrand` so that on `PortError::Timeout`:
1. Write `KnotFailed` to loom-log (already done)
2. Write `StrandProcessed` with error to loom-log (already done)
3. **Skip** the `tie_off_sink.append()` call — do NOT write error to tie-off file
4. **New:** Write `RigLogEvent::TimeoutExceeded` to rig-log

For non-timeout errors, current behavior is preserved (error still written to tie-off).

Add `RigLogPort` to `ProcessStrand` struct and composition root.

**Tests:**
- Modify existing mock `AgentRunner` to support returning `PortError::Timeout`
- Unit test: `ProcessStrand::execute` with timeout → verify loom-log has `KnotFailed` + `StrandProcessed`, rig-log has `TimeoutExceeded`, tie-off is **unchanged** (no new section appended)
- Unit test: `ProcessStrand::execute` with non-timeout error → verify error IS written to tie-off (existing behaviour preserved)

**Tasks:**
- [ ] Add `rig_log: Arc<dyn RigLogPort>` field to `ProcessStrand` struct
- [ ] Update `ProcessStrand::new()` constructor to accept `RigLogPort`
- [ ] In `execute()` error branch, match on `PortError::Timeout`:
  - On timeout: skip `tie_off_sink.append()`, write `RigLogEvent::TimeoutExceeded`
  - On other errors: existing behaviour (write error to tie-off)
- [ ] Update `server.rs` composition root to create `FileSystemRigLog` and wire into `ProcessStrand`
- [ ] Add mock `AgentRunner` that returns `PortError::Timeout` in unit tests
- [ ] Add unit tests for timeout path (loom-log, rig-log, tie-off unchanged)
- [ ] Add unit test for non-timeout error path (tie-off still receives error — regression guard)
- [ ] Run `cargo test` — all tests pass

### Phase 4: Queue Idle detection + rig-log QueueIdle entry

Hex layer: **Application (use cases)**

After `ProcessStrand::execute()` completes, detect when the event pipeline has no pending events and write `RigLogEvent::QueueIdle` to the rig-log.

Strategy: in the `ProcessStrand` event loop (`while let Some(event) = debounce_rx.recv().await`), after processing each event, use `tokio::time::timeout` with a short poll window (e.g. 500ms) to check if another event arrives. If no event arrives within the window, write `QueueIdle`.

This is a "drain check" — it doesn't block processing, just checks if the channel is momentarily empty. If a new event arrives during the poll window, it's processed normally and the check resets after that event completes.

**Tests:**
- Integration test: after processing a strand, verify `QueueIdle` appears in rig-log within the poll window
- Integration test: rapid burst of events → only one `QueueIdle` written (after all complete, not between each)

**Tasks:**
- [ ] In `ProcessStrand` event loop, after `execute()`:
  - Use `tokio::time::timeout(500ms, debounce_rx.recv())` to poll for next event
  - If timeout fires (no event): write `QueueIdle` to rig-log, then loop back to `recv().await` for the real next event
  - If event arrives: process it normally (don't write `QueueIdle`)
- [ ] Integration test in `tests/rig_log.rs`: single event → `QueueIdle` written
- [ ] Integration test in `tests/rig_log.rs`: burst of 3 events → only one `QueueIdle` after all complete
- [ ] Run `cargo test` — all tests pass

### Phase 5: Integration tests and cleanup

Full integration test coverage for the rig-log and timeout flows:

- `tests/rig_log.rs` — integration tests:
  - Timeout → rig-log `TimeoutExceeded` entry
  - Successful processing → no rig-log entry
  - Queue idle → rig-log `QueueIdle` entry
  - Burst of events → single `QueueIdle` after all complete
  - Tie-off preserved on timeout (no error content appended)
  - Tie-off receives error on non-timeout failure (regression guard)

**Tasks:**
- [ ] Create `tests/rig_log.rs` with integration tests
- [ ] Run `cargo test` — all tests pass
- [ ] Update `project/domain-glossary.md` with new term: `Rig-log`
- [ ] Run `cargo clippy` — no warnings

## Notes

[Any observations, deviations, or lessons learned during implementation]
