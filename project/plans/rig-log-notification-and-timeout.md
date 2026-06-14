# Plan: Rig-Log Notification and Timeout Handling

## Related PRD

This plan contributes to [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md).

It implements Story 6 (Rig-Log Notification) and the timeout/error handling changes that support Story 5 (Event Replay — user touches file to reprocess). The rig-log surfaces serious operational events (timeouts, queue idle) so the user or an external agent can monitor and react. On timeout, the tie-off is preserved unchanged (no error appended).

## Problem

1. **No rig-level notification** — when an agent session times out, the user has no way to discover it without polling HTTP endpoints or inspecting individual loom-logs. There is no single file that an external watcher (human or LLM agent) can monitor for "something needs attention."

2. **Timeout errors pollute the tie-off** — currently `ProcessStrand::execute` writes `Processing failed: ...` into the tie-off file on any error (including timeout). The tie-off is the agent's output and should contain only agent-produced content. Operational errors belong in logs.

3. **No per-profile timeout** — the agent runner timeout is a single global value set at composition root. Different agent profiles may need different timeouts (e.g. a fast model at 60s vs. a slow reasoning model at 600s). The user needs to configure timeout per profile so each knot gets the right deadline for its model.

## Target

1. **Rig-log** (`rig/.rig-log`) — append-only JSONL file recording `TimeoutExceeded` and `QueueIdle` events. Survives server restarts. Multiple consumers can watch it safely.

2. **Timeout handling** — on `PortError::Timeout`, `ProcessStrand` writes to loom-log and rig-log only, **not** to the tie-off file. Previous tie-off content is preserved unchanged.

3. **Queue idle** — after processing completes and no events are pending, a `QueueIdle` entry is written to the rig-log so the user knows the system is quiet.

4. **Per-profile timeout** — `AgentProfile` has an optional `timeout` field (seconds). When a profile specifies `timeout`, the agent runner uses that value for the session deadline. When not set, the runner's global default is used. This lets fast models use short timeouts (60s) and slow models use long ones (600s).

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
- No test for profile timeout field parsing (timeout in YAML frontmatter)
- No test for `ExecutionContext` timeout override (per-profile timeout reaching the runner)
- No test for profile timeout round-trip (save → load → execute with correct timeout)

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
- [x] Add `RigLogPath` to `src/domain/entities.rs`
- [x] Add `RigLogEvent` enum to `src/domain/events.rs`
- [x] Add serialisation round-trip tests for all `RigLogEvent` variants
- [x] Run `cargo test` — all existing tests still pass

### Phase 1: Domain — AgentProfile timeout field

Hex layer: **Domain**

Add an optional `timeout` field to `AgentProfile` so each profile can declare its own session timeout. This is the configuration side of Story 9.

- `AgentProfile` gains an optional field:
  - `timeout: Option<u64>` — session timeout in seconds (e.g. 60, 300, 600). `None` means use the runner's default.

The `AgentProfile::new()` constructor keeps its existing signature (no timeout). A new builder method `with_timeout()` or a `with_timeout` helper on the struct lets callers set it. The `AgentConfig::new()` is unaffected — timeout is a profile concern, not a per-knot config.

A constant or associated function provides the default: `AgentProfile::DEFAULT_TIMEOUT_SECS = 300` (matches current `AppConfig.agent_timeout`).

**Tests:**
- Unit test: `AgentProfile` with `timeout = Some(600)` serialises/deserialises correctly
- Unit test: `AgentProfile` with `timeout = None` serialises/deserialises correctly
- Unit test: `AgentProfile::with_timeout()` builder sets the field correctly
- Unit test: default `AgentProfile` (no timeout) still constructs fine

**Tasks:**
- [ ] Add `timeout: Option<u64>` field to `AgentProfile` struct in `src/domain/value_objects.rs`
- [ ] Add `#[serde(default)]` to the field so profiles without `timeout` parse correctly
- [ ] Add `#[serde(skip_serializing_if = "Option::is_none")]` so the field is omitted from YAML when `None`
- [ ] Add `AgentProfile::with_timeout()` builder method
- [ ] Add `AgentProfile::DEFAULT_TIMEOUT_SECS` constant
- [ ] Add serialisation round-trip tests for profiles with and without timeout
- [ ] Add unit tests for `with_timeout` builder
- [ ] Run `cargo test` — all existing tests still pass

### Phase 2: Domain — Parse timeout from profile frontmatter

Hex layer: **Domain (knot_file parsing)**

Update `parse_agent_profile()` to read the `timeout` field from YAML frontmatter.

- `RawProfileFrontmatter` in `src/domain/knot_file.rs` gains an optional `timeout` field (`Option<u64>`)
- `parse_agent_profile()` passes the parsed timeout to `AgentProfile::with_timeout()`
- Existing profiles without `timeout` in frontmatter parse as `None` (backwards compatible)
- Invalid timeout values (negative numbers) are rejected at parse time — though `u64` from YAML can't be negative, so this is naturally enforced

**Tests:**
- Unit test: profile file with `timeout: 600` parses correctly with `timeout = Some(600)`
- Unit test: profile file without `timeout` key parses with `timeout = None`
- Unit test: profile file with `timeout: null` parses with `timeout = None`
- Round-trip test: profile with timeout serialises to YAML, parses back, timeout preserved

**Tasks:**
- [ ] Add `timeout: Option<u64>` to `RawProfileFrontmatter` in `src/domain/knot_file.rs`
- [ ] Update `parse_agent_profile()` to call `.with_timeout(raw.timeout)` on the built profile
- [ ] Add parsing unit tests for timeout present, absent, and null
- [ ] Add round-trip test (parse → serialise → parse)
- [ ] Run `cargo test` — all existing tests still pass

### Phase 3: Application — RigLogPort

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

### Phase 4: Outbound Adapters — FileSystemRigLog

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

### Phase 5: Ports — ExecutionContext timeout + AgentRunner interface change

Hex layer: **Application (ports)** + **Outbound adapters**

Currently `SubprocessAgentRunner` has a single `timeout` value set at construction time (composition root). To support per-profile timeouts, the runner needs to read the timeout from the execution context instead.

Changes:

1. `ExecutionContext` gains an optional `timeout: Option<Duration>` field.
2. `AgentRunner::execute()` signature stays the same — it already receives the full `ExecutionContext`.
3. `SubprocessAgentRunner::execute()` reads `ctx.timeout`, falling back to its own `self.timeout` (the global default from composition root) if `None`. This preserves backward compatibility — any code constructing an `ExecutionContext` without setting timeout still gets the runner's default.
4. The `MockAgentRunner` in the ports test module ignores the timeout field (no-op).

This is a **non-breaking change** to the trait — the method signature doesn't change, only the context struct gains a field.

**Tests:**
- Unit test: `SubprocessAgentRunner` with 120s default timeout, context has `timeout = Some(5s)` → kills after 5s
- Unit test: `SubprocessAgentRunner` with 120s default timeout, context has `timeout = None` → uses 120s default
- Unit test: `SubprocessAgentRunner` with 120s default timeout, context has `timeout = Some(300s)` → uses 300s
- Unit test: existing timeout test (no context override) still passes — regression guard

**Tasks:**
- [ ] Add `timeout: Option<Duration>` field to `ExecutionContext` in `src/application/ports.rs`
- [ ] Update `AgentRunner` trait doc comment to document the timeout behaviour
- [ ] In `SubprocessAgentRunner::execute()`: `let effective_timeout = ctx.timeout.unwrap_or(self.timeout);`
- [ ] Update mock `ExecutionContext` in port tests to include `timeout: None`
- [ ] Add unit tests for per-context timeout (override, fallback, large value)
- [ ] Run `cargo test` — all existing tests still pass

### Phase 6: ProcessStrand — Timeout handling change + rig-log writes

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

### Phase 7: ProcessStrand — Resolve profile timeout and pass to ExecutionContext

Hex layer: **Application (use cases)** + **Composition root**

Update `ProcessStrand` to resolve the timeout from the agent profile and pass it into the execution context.

Changes:

1. `resolve_agent_config()` returns a tuple of `(AgentConfig, String, Option<Duration>)` — the third element is the profile's timeout converted to a `Duration` (or `None` if not set).
2. In `execute()`, the resolved timeout is passed into `ExecutionContext::timeout`.
3. The composition root (`server.rs`) keeps `AppConfig.agent_timeout` as the **fallback** — it's still passed to `SubprocessAgentRunner::with_timeout()` as the global default. Profiles that set `timeout` override this; profiles that don't fall back to it.

The `resolve_agent_config()` return type change is a single-use case internal API — only `ProcessStrand::execute()` calls it, so this is a local refactor.

**Tests:**
- Unit test: `ProcessStrand::execute` with a mock profile that has `timeout = Some(60)` → `ExecutionContext.timeout` is `Some(Duration::from_secs(60))`
- Unit test: `ProcessStrand::execute` with a mock profile that has `timeout = None` → `ExecutionContext.timeout` is `None` (falls back to runner default)
- Unit test: `resolve_agent_config()` returns correct timeout from profile

**Tasks:**
- [ ] Update `resolve_agent_config()` to also return the profile's timeout as `Option<Duration>`:
  - `pub fn resolve_agent_config(&self, knot: &Knot) -> Result<(AgentConfig, String, Option<std::time::Duration>), PortError>`
  - Convert `profile.timeout` (Option<u64>) to `Option<Duration>` via `.map(Duration::from_secs)`
- [ ] In `execute()`, extract timeout from resolved tuple and pass to `ExecutionContext`
- [ ] Update mock `AgentProfile` instances in existing tests to include `timeout: None`
- [ ] Add unit tests for timeout resolution from profile
- [ ] Run `cargo test` — all existing tests still pass

### Phase 8: Queue Idle detection + rig-log QueueIdle entry

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

### Phase 9: Integration tests and cleanup

Full integration test coverage for the rig-log and timeout flows:

- `tests/rig_log.rs` — integration tests:
  - Timeout → rig-log `TimeoutExceeded` entry
  - Successful processing → no rig-log entry
  - Queue idle → rig-log `QueueIdle` entry
  - Burst of events → single `QueueIdle` after all complete
  - Tie-off preserved on timeout (no error content appended)
  - Tie-off receives error on non-timeout failure (regression guard)

- `tests/profile_timeout.rs` — integration tests for profile timeout:
  - Profile with `timeout: 2` → agent session killed after 2 seconds, `TimeoutExceeded` in rig-log
  - Profile with no timeout field → uses runner default (e.g. 120s), long-running agent doesn't timeout at 2s
  - Profile with `timeout: 600` → overrides runner default of 120s, agent allowed to run for 600s
  - Profile file with `timeout: 30` serialises/deserialises correctly (round-trip via FileSystemAgentProfileRepository)

**Tasks:**
- [ ] Create `tests/rig_log.rs` with integration tests
- [ ] Create `tests/profile_timeout.rs` with profile timeout integration tests
- [ ] Run `cargo test` — all tests pass
- [ ] Update `project/domain-glossary.md` with new term: `Rig-log`
- [ ] Run `cargo clippy` — no warnings

## Notes

[Any observations, deviations, or lessons learned during implementation]
