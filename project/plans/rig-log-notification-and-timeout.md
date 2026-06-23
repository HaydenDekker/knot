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

## Implementation Status: ✅ Complete

**Completed:** 2026-06-14
**Bugfix (Phase X):** 2026-06-15 — Spurious timeout warning from detached thread not checking child exit status; missing knot context in processing logs. Fixed via `AtomicBool` cancelled flag + strand/knot context in logs. Version bumped to `0.5.1`.

**Bugfix (Phase Y):** 2026-06-14 — Timestamp logs producing wrong year (`-2773` for `QueueIdle` events). Two bugs in separate `format_timestamp()` implementations: `usecases.rs` used a wrong Dershowitz & Reingold formula variant (producing year -2773), `logging.rs` had an off-by-one (`+1`) shifting dates by one day. `tieoff_sink.rs` was already correct. Fixed by correcting `logging.rs::days_to_ymd()` and delegating `usecases.rs::format_timestamp()` to `logging::format_timestamp()`, eliminating the duplicate algorithm. All 293+ tests pass.

**Bugfix (Phase Z):** 2026-06-23 — `timeout` field missing from `RigStateProfile` in `rig/state.json`. The `AgentProfile` domain type had `timeout: Option<u64>` and it was correctly parsed from profile frontmatter and used at runtime, but `RigStateProfile` (the state snapshot type) only exposed `name`, `provider`, and `model`. The `build_state()` mapper in `usecases.rs` did not copy `timeout` into `RigStateProfile`, so the timeout was invisible in the state file. Fixed by adding `timeout: Option<u64>` to `RigStateProfile` with `#[serde(skip_serializing_if = "Option::is_none")]` and wiring it through `build_state()`. Skill docs updated to reflect timeout visibility in state. Version bumped to `0.16.1`.

**Result:** Rig-log (`rig/.rig-log`) records `TimeoutExceeded` and `QueueIdle` events as JSONL. On timeout, tie-off is preserved unchanged (error written to loom-log + rig-log only). Per-profile timeout via `AgentProfile.timeout` field (optional, in seconds). `SubprocessAgentRunner` reads effective timeout from `ExecutionContext.timeout`, falling back to runner default. 362 tests pass (11 new unit + 11 new integration). Domain glossary updated with `Rig-log` term. Clippy clean (no new warnings).

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
- [x] Add `timeout: Option<u64>` field to `AgentProfile` struct in `src/domain/value_objects.rs`
- [x] Add `#[serde(default)]` to the field so profiles without `timeout` parse correctly
- [x] Add `#[serde(skip_serializing_if = "Option::is_none")]` so the field is omitted from YAML when `None`
- [x] Add `AgentProfile::with_timeout()` builder method
- [x] Add `AgentProfile::DEFAULT_TIMEOUT_SECS` constant
- [x] Add serialisation round-trip tests for profiles with and without timeout
- [x] Add unit tests for `with_timeout` builder
- [x] Run `cargo test` — all existing tests still pass

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
- [x] Add `timeout: Option<u64>` to `RawProfileFrontmatter` in `src/domain/knot_file.rs`
- [x] Update `parse_agent_profile()` to call `.with_timeout(raw.timeout)` on the built profile
- [x] Add parsing unit tests for timeout present, absent, and null
- [x] Add round-trip test (parse → serialise → parse)
- [x] Run `cargo test` — all existing tests still pass

### Phase 3: Application — RigLogPort

Hex layer: **Application (ports)**

New port trait:

- `RigLogPort` — `append(event: RigLogEvent)` and `read_all() -> Vec<RigLogEvent>`
- Add `PortError::RigLogWriteFailed` and `PortError::RigLogReadFailed` error variants

**Tests:**
- Unit tests verifying new `PortError` variants implement `Display` and `Error`
- Unit tests for mock `RigLogPort`

**Tasks:**
- [x] Add `RigLogPort` trait to `src/application/ports.rs`
- [x] Add new `PortError` variants and `Display` implementations
- [x] Add mock `RigLogPort` in test module
- [x] Run `cargo test` — all existing tests still pass

### Phase 4: Outbound Adapters — FileSystemRigLog

Hex layer: **Outbound adapters**

Concrete filesystem implementation:

- `FileSystemRigLog` — writes `RigLogEvent` as JSONL to `rig/.rig-log` (append mode, creates parent dirs)

**Tests:**
- `FileSystemRigLog::new`, `append`, `read_all` — creates file, appends JSONL, reads back
- `FileSystemRigLog` concurrent writes — multiple threads append safely

**Tasks:**
- [x] Create `src/adapters/outbound/rig_log.rs` with `FileSystemRigLog`
- [x] Implement `RigLogPort` for `FileSystemRigLog`
- [x] Add unit tests for `FileSystemRigLog` (create, append, read_all, concurrent writes)
- [x] Export in `src/adapters/outbound/mod.rs`
- [x] Run `cargo test` — all tests pass

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
- [x] Add `timeout: Option<Duration>` field to `ExecutionContext` in `src/application/ports.rs`
- [x] Update `AgentRunner` trait doc comment to document the timeout behaviour
- [x] In `SubprocessAgentRunner::execute()`: `let effective_timeout = ctx.timeout.unwrap_or(self.timeout);`
- [x] Update mock `ExecutionContext` in port tests to include `timeout: None`
- [x] Add unit tests for per-context timeout (override, fallback, large value)
- [x] Run `cargo test` — all existing tests still pass

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
- [x] Add `rig_log: Arc<dyn RigLogPort>` field to `ProcessStrand` struct
- [x] Update `ProcessStrand::new()` constructor to accept `RigLogPort`
- [x] In `execute()` error branch, match on `PortError::Timeout`:
  - On timeout: skip `tie_off_sink.append()`, write `RigLogEvent::TimeoutExceeded`
  - On other errors: existing behaviour (write error to tie-off)
- [x] Update `server.rs` composition root to create `FileSystemRigLog` and wire into `ProcessStrand`
- [x] Add mock `AgentRunner` that returns `PortError::Timeout` in unit tests
- [x] Add unit tests for timeout path (loom-log, rig-log, tie-off unchanged)
- [x] Add unit test for non-timeout error path (tie-off still receives error — regression guard)
- [x] Run `cargo test` — all tests pass

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
- [x] Update `resolve_agent_config()` to also return the profile's timeout as `Option<Duration>`:
  - `pub fn resolve_agent_config(&self, knot: &Knot) -> Result<(AgentConfig, String, Option<std::time::Duration>), PortError>`
  - Convert `profile.timeout` (Option<u64>) to `Option<Duration>` via `.map(Duration::from_secs)`
- [x] In `execute()`, extract timeout from resolved tuple and pass to `ExecutionContext`
- [x] Update mock `AgentProfile` instances in existing tests to include `timeout: None`
- [x] Add unit tests for timeout resolution from profile
- [x] Run `cargo test` — all existing tests still pass

### Phase 8: Queue Idle detection + rig-log QueueIdle entry

Hex layer: **Application (use cases)**

After `ProcessStrand::execute()` completes, detect when the event pipeline has no pending events and write `RigLogEvent::QueueIdle` to the rig-log.

Strategy: in the `ProcessStrand` event loop (`while let Some(event) = debounce_rx.recv().await`), after processing each event, use `tokio::time::timeout` with a short poll window (e.g. 500ms) to check if another event arrives. If no event arrives within the window, write `QueueIdle`.

This is a "drain check" — it doesn't block processing, just checks if the channel is momentarily empty. If a new event arrives during the poll window, it's processed normally and the check resets after that event completes.

**Tests:**
- Integration test: after processing a strand, verify `QueueIdle` appears in rig-log within the poll window
- Integration test: rapid burst of events → only one `QueueIdle` written (after all complete, not between each)

**Tasks:**
- [x] In `ProcessStrand` event loop, after `execute()`:
  - Use `tokio::time::timeout(500ms, debounce_rx.recv())` to poll for next event
  - If timeout fires (no event): write `QueueIdle` to rig-log, then loop back to `recv().await` for the real next event
  - If event arrives: process it normally (don't write `QueueIdle`)
- [x] Integration test in `tests/rig_log.rs`: single event → `QueueIdle` written
- [x] Integration test in `tests/rig_log.rs`: burst of 3 events → only one `QueueIdle` after all complete
- [x] Run `cargo test` — all tests pass

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
- [x] Create `tests/rig_log.rs` with integration tests
- [x] Create `tests/profile_timeout.rs` with profile timeout integration tests
- [x] Run `cargo test` — all tests pass
- [x] Update `project/domain-glossary.md` with new term: `Rig-log`
- [x] Run `cargo clippy` — no warnings

### Phase X: Bugfix — Spurious timeout warning + missing knot context in logs

Hex layer: **Outbound adapters** + **Application (use cases)**

**Discovered:** 2026-06-15 in production use.

**Bug 1: Spurious `WARNING: killed 'pi' after timeout of 300s` on normal completion.**

Phase 5 spawned a detached background thread (`std::thread::Builder::new().spawn()`) that sleeps for `effective_timeout` then unconditionally sends `SIGKILL` and prints a warning. When the child process exits *before* the deadline (the common case — agents completing in ~13s against a 300s default), the detached thread keeps sleeping in the background. When its sleep finally expires, it prints the warning even though the process already finished cleanly.

**Root cause:** The timeout thread has no way to know the child exited normally. It always assumes the child is still alive when its sleep completes. The plan's tests only verified "does the agent get killed?" and "does it complete?" — never "does the thread suppress its action when the child exits first?" The spurious warning is invisible in CI because tests use timeouts of 50ms–100ms where the race is negligible.

**Fix:** `AtomicBool` cancelled flag shared between the main thread and the timeout thread. After `wait_with_output()` returns, the main thread sets the flag. The timeout thread checks it after its sleep — if cancelled, returns silently without `SIGKILL` or warning.

**Bug 2: Processing logs lack knot/strand context.**

The `log_strand_event` calls in `ProcessStrand::execute()` emit messages like `Modified processing start — /path/to/file.md` but omit *which knot* is processing. With multiple knots per loom, the user cannot tell which knot triggered the log line. The `SubprocessAgentRunner` timeout warning similarly omits strand context.

**Fix:**
- `ProcessStrand` logs now include `knot=<knot_id>` in processing start, completed, and failed messages
- `SubprocessAgentRunner` timeout warning and `PortError::Timeout` now include `(strand: <path>)`

**Files changed:**
- `src/adapters/subprocess.rs` — `AtomicBool` cancelled flag, strand context in warning/error
- `src/application/usecases.rs` — knot context in all `log_strand_event` calls

**Why the plan missed it:**
- Phase 5's test matrix covered the *correctness* dimension (right timeout value used) but not the *lifecycle* dimension (what happens to the detached thread when the child exits early). A test case for "agent completes in 10ms, timeout is 5s, no spurious output" would have caught this.
- The knot context gap was a missing observability concern — the plan focused on the rig-log as the notification mechanism and didn't consider that the existing `eprintln`-based logs also needed context.

**Tasks:**
- [x] Add `AtomicBool` cancelled flag to `SubprocessAgentRunner::execute()`
- [x] Timeout thread checks cancelled flag before SIGKILL + warning
- [x] Main thread sets cancelled flag after `wait_with_output()` returns
- [x] Add strand path to timeout warning and `PortError::Timeout` message
- [x] Add knot ID to `log_strand_event` messages (start, completed, failed)
- [x] Run `cargo test` — all 362 tests pass
- [x] Run `cargo clippy` — no new warnings

### Phase Y: Bugfix — Wrong year in timestamps

Hex layer: **Application (use cases)** + **Outbound adapters (logging)**

**Discovered:** 2026-06-14 in production use.

**Bug: Timestamps producing year `-2773`.**

Three separate `format_timestamp()` implementations existed in the codebase, each converting Unix epoch seconds to ISO 8601 date:

1. `application/usecases.rs::format_timestamp()` — used a wrong Dershowitz & Reingold formula variant (`let a = z + 305` / `let b = (4*a+3)/146097` / `let year = 100*b + d - 4800 + m/10`). This produced year **-2773** for any modern date. This is the function called by `server.rs` when writing `QueueIdle` timestamps, so this is what produced `"timestamp":"-2773-04-15T11:36:04Z"`.

2. `adapters/logging.rs::format_timestamp()` — used the correct Dershowitz & Reingold algorithm but with an erroneous `let z = z + 1;` that shifted every date forward by one day.

3. `adapters/outbound/tieoff_sink.rs::format_timestamp()` — already correct.

**Fix:**
- `logging.rs::days_to_ymd()` — removed the erroneous `+1` so dates are accurate
- `usecases.rs::format_timestamp()` — replaced the broken algorithm entirely, delegating to `logging::format_timestamp()` instead. This eliminates the duplicate and ensures all timestamp generation goes through a single verified implementation

**Why the plan missed it:**
- The plan introduced `QueueIdle` events with real timestamps in Phase 8, but all timestamps were generated at runtime — CI tests only verified timestamps *parse correctly* and are *present*, not that the *values* are correct. A test that asserts the year falls within a plausible range (e.g. 2024–2100) would have caught this. The `-2773` year is silently valid ISO 8601 so no parser complained.
- The logging off-by-one was similarly invisible in tests because tests compare against `format_timestamp()` itself (identity comparisons), never against an external clock.

**Tasks:**
- [x] Fix `days_to_ymd()` in `src/adapters/logging.rs` — remove `let z = z + 1;`
- [x] Replace `format_timestamp()` in `src/application/usecases.rs` with delegation to `logging::format_timestamp()`
- [x] Run `cargo test` — all 293+ tests pass
- [x] Verify timestamps now produce correct year (2026)

## Notes

- Phase 1 sub-agent proactively completed Phase 2 tasks (parsing timeout from profile frontmatter) since they were tightly coupled domain-layer changes.
- Phase 6 required updating `AppContext` and all test files that manually construct it (`swagger_ui.rs`, `skill_integration.rs`, `loom.rs`, `usecases.rs`) — significant ripple from adding `rig_log_port` field.
- Phase 8 queue idle detection was implemented in `server.rs` event loop (not inside `ProcessStrand`) since the event loop owns the debounce channel receiver.
- Pre-existing `ConfigurableAgentRunner::set_result` unused method warning existed before this plan — not introduced by our changes.
- Phase X bugfix: spurious timeout warning from detached thread not checking child exit status, plus missing knot context in processing logs. Detached thread pattern needs lifecycle awareness — any future timeout implementation should cancel the watcher when the watched handle completes.
- Phase Y bugfix: timestamp year `-2773` from wrong algorithm in `usecases.rs::format_timestamp()` and off-by-one in `logging.rs::days_to_ymd()`. Consolidated to single implementation in `logging.rs`. Any future timestamp code should reuse `logging::format_timestamp()` — never duplicate the epoch-to-date algorithm.
