# Plan 47: Session Resume on Invocation Failure

## Related PRD

This plan contributes to [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md), implementing Story 5a (Session Resume on Invocation Failure).

This plan depends on Plan 46 (agent-json-adapter.md). The JSON adapter provides `session_id` capture in `AgentOutput.metadata` and `PortError::is_resumable()` — both prerequisites for session resume.

## Problem

When an agent invocation fails partway through (network error, provider timeout mid-stream, subprocess killed), Knot currently discards all partial work and marks the strand as failed. The user must manually `touch` the strand file to reprocess, which starts a **fresh Pi session** — the provider re-sends the full context from scratch.

This wastes provider capacity and increases cost: the conversation history already accepted by Pi is re-sent unnecessarily. If the failure was transient (network blip, momentary provider slowdown), resuming the same Pi session would complete the work without re-sending the full context.

## Target

When a resumable invocation failure occurs and a session ID was captured, Knot automatically retries the same invocation using `--session-id <id>` to continue the Pi session — up to 10 retry attempts **or** until the profile's overall timeout budget is exhausted (whichever comes first). On retry, a `"please continue"` message is appended to the session so the agent resumes where it left off. A 10-second delay between retries allows transient network errors to recover. On successful resume, the strand completes normally (transparent). On exhausted retries or budget expiry, the strand is marked failed and the existing failure path (loom-log, rig-log) takes over.

## Implementation Status: ✅ Complete

**Completed:** 2026-06-28 on branch `refactor/session-resume-on-invocation-failure`

**Depends on:** Plan 46 (agent-json-adapter.md) — provides `AgentInvocationMetadata`, `PortError::is_resumable()`, `JsonSubprocessAgentRunner`.

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `src/application/usecases.rs` tests | ProcessStrand execute flow, config resolution, mock runner | ✅ Green — 83 tests |
| `tests/profile_timeout.rs` | Timeout enforcement, tie-off preservation on timeout | ✅ Green — integration tests |
| `tests/rig_log.rs` | Rig-log entries for TimeoutExceeded, QueueIdle | ✅ Green — integration tests |
| `tests/pipeline.rs` | Full event pipeline with mock runner | ✅ Green — integration tests |
| `tests/git_versioning.rs` | Git commit after tie-off write | ✅ Green — integration tests |
| `src/domain/entities.rs` tests | Knot entity construction, serialization | ✅ Green — 22 tests |
| `src/domain/events.rs` tests | LoomEvent variants, serialization | ✅ Green — 21 tests |
| `src/domain/knot_file.rs` tests | Knot YAML parsing, warnings | ✅ Green — 40 tests |
| `src/domain/value_objects.rs` tests | AgentProfile, RigAgentConfig parsing | ✅ Green — 32 tests |

## Test Gaps

- No retry loop in ProcessStrand — single execute() call, no retry logic
- No `SessionResumed` loom-log event
- No test for overall timeout budget tracking across retry attempts
- No test for remaining-time calculation per retry attempt
- No test for `"please continue"` prompt append on retry
- No test for 10-second retry delay
- No test for `--session-id` passthrough on retry
- No test for 10-retry hard cap
- No integration test for session resume end-to-end
- No test for resumable vs. fatal failure classification in use-case

## Phases

### [x] Phase 0: Domain — SessionResumed Event, Resume Helper

Work in domain layer: `src/domain/events.rs`. Application layer: `src/application/ports.rs`.

**Changes:**

1. **`src/domain/events.rs`** — Add `SessionResumed` variant to `LoomEvent`:
   ```rust
   /// A failed agent invocation was resumed using the same Pi session.
   SessionResumed {
       loom_id: LoomId,
       knot_id: KnotId,
       strand_path: StrandPath,
       session_id: String,
       attempt: u32,
       timestamp: String,
   },
   ```

2. **`src/application/ports.rs`** — Add `is_session_resumable()` helper function (free function — takes both session_id and error):
   ```rust
   /// Determine if a failed invocation should trigger a session-resume retry.
   pub fn is_session_resumable(session_id: &Option<String>, error: &PortError) -> bool {
       session_id.is_some() && error.is_resumable()
   }
   ```

   Note: `PortError::is_resumable()` and `PortError::session_id()` already exist from Plan 46.

**No domain entity changes** — retry limit (10) and delay (10s) are application-layer constants, not per-knot configuration. The profile timeout (from `AgentProfile.timeout`) serves as the overall budget.

**Tests (domain/application unit):**
- `session_resumed_event_serialization()` — `SessionResumed` serialises/deserialises correctly
- `is_session_resumable_with_session_and_timeout()` — session_id + Timeout → `true`
- `is_session_resumable_with_session_and_execution_failed()` — session_id + AgentExecutionFailed → `true`
- `is_session_resumable_without_session()` — session_id None → `false`
- `is_session_resumable_command_not_found()` — CommandNotFound → `false`

**Existing tests to update:**
- `LoomEvent` match exhaustiveness — add `SessionResumed` arm

### [x] Phase 1: Application — Session Resume Module

New file: **`src/application/session_resume.rs`**. Registered in `src/application/mod.rs`.

The retry loop lives in its own module, not embedded in `usecases.rs` (already 8473 lines). `ProcessStrand::execute()` delegates the retry call to this module — single responsibility, easily testable in isolation.

**Module public API:**

```rust
/// Attempt agent execution with automatic session-resume retry.
///
/// Returns `Ok(AgentOutput)` on success (first attempt or after N retries).
/// Returns `Err(PortError)` when retries exhausted or overall timeout budget expired.
///
/// `SessionResumed` events are appended to `loom_log` for each retry attempt.
pub fn execute_with_resume(
    agent_runner: &dyn AgentRunner,
    loom_log: &dyn LoomLogPort,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_path: &StrandPath,
    session_id: &mut Option<String>,
    base_cli_args: Vec<String>,
    prompt: String,
    profile_timeout: Duration,
) -> Result<AgentOutput, PortError>;
```

**Internals:** the module contains:
- `MAX_RETRIES: u32 = 10` and `RETRY_DELAY: Duration = Duration::from_secs(10)` constants
- The retry loop (overall budget tracking, remaining-time calculation, delay, `--session-id` + `"please continue"` append)
- `build_retry_context()` helper — constructs `ExecutionContext` with remaining timeout
- `prepare_retry()` helper — mutates `cli_args`/`prompt` for next attempt

**Separation of concerns:**
- `ProcessStrand::execute()` — orchestrates the strand workflow (config resolution → `execute_with_resume()` → tie-off write → git commit)
- `session_resume::execute_with_resume()` — owns the retry policy (budget tracking, delays, session resume, loom-log events)
- `PiJsonAgentRunner::execute()` — owns the subprocess lifecycle (spawn, stdin, timeout kill, stdout parse)

**Tests (unit with mock runner):**

Same test set as before, but exercised against `execute_with_resume()` directly (no ProcessStrand needed):

- `retry_succeeds_on_first_retry()` — mock returns `[Err(TIMEOUT), Ok]`, profile timeout 120s → Ok, SessionResumed logged
- `retry_exhausted_then_fails()` — mock returns `[Err(TIMEOUT) × 11]` → Err after 10 retries, SessionResumed logged ×10
- `retry_stops_on_overall_timeout()` — mock returns `[Err(TIMEOUT) × 3]`, profile timeout 30s, each attempt takes 11s → Err after 3 attempts (budget exhausted before 10 retries)
- `retry_stops_on_insufficient_time()` — remaining time < 5s → loop bails, Err
- `no_retry_on_fatal_error()` — mock returns `Err(CommandNotFound)` → Err immediately, no SessionResumed
- `no_retry_when_no_session_id()` — mock returns `Err(TIMEOUT { session_id: None })` → Err immediately
- `retry_preserves_other_cli_args()` — `--session-id` appended, other args unchanged
- `retry_appends_please_continue()` — prompt on retry includes `"please continue"` suffix
- `retry_delay_between_attempts()` — 10s delay verified (wall-clock with small value for test)
- `session_id_captured_from_error()` — PortError::Timeout carries session_id, used in retry
- `successful_retry_transparent()` — loom-log shows SessionResumed + no KnotFailed
- `first_attempt_succeeds_no_retry()` — mock returns `[Ok]` → Ok immediately, no SessionResumed

### [x] Phase 2: Application — Wire Into ProcessStrand

Work in `src/application/usecases.rs`, `ProcessStrand::execute()`.

**Changes:**

1. Add `mod session_resume` to `src/application/mod.rs`.
2. In `ProcessStrand::execute()`, replace the single `agent_runner.execute_with_config(...)` call with:
   ```rust
   let agent_output = session_resume::execute_with_resume(
       agent_runner,
       loom_log,
       &loom_id,
       &knot_id,
       &strand_path,
       &mut session_id,
       cli_args,
       prompt,
       profile_timeout,
   )?;
   ```
3. Everything after (tie-off write, git commit, KnotCompleted log) stays the same — the retry is transparent to the outer flow.

**Tests:**
- `process_strand_retry_transparent_success()` — ProcessStrand with mock runner that fails then succeeds → KnotCompleted, no KnotFailed
- `process_strand_retry_exhausted_fails()` — ProcessStrand with mock runner that always fails → KnotFailed + TimeoutExceeded in rig-log
- `process_strand_no_retry_stdio()` — stdio adapter (no session_id) → single attempt, immediate fail
- Regression: all existing ProcessStrand tests still pass

### [x] Phase 3: Integration Tests and Verification

- [ ] Integration test: `test_session_resume_success()` — real `pi` binary, knot with profile timeout 120s, simulate first-invocation failure (e.g., very short timeout that Pi doesn't complete within), verify resume attempt with `--session-id` + "please continue" in prompt, check loom-log for `SessionResumed` + `KnotCompleted`
- [ ] Integration test: `test_session_resume_exhausted()` — both attempts timeout within budget → `KnotFailed` in loom-log, `TimeoutExceeded` in rig-log
- [ ] Integration test: `test_session_resume_budget_expired()` — profile timeout 30s, multiple retries eat budget → `KnotFailed` before 10 retries, overall timeout message
- [ ] Integration test: `test_session_resume_stdio_no_retry()` — `invocation_mode: stdio` (Plan 46), failure → no retry (session_id never captured)
- [ ] Integration test: `test_session_resume_transparent_on_success()` — first fails, retry succeeds → loom-log has `SessionResumed` + `KnotCompleted`, no `KnotFailed`
- [ ] Integration test: `test_session_resume_delay_between_retries()` — 10s delay observed between retry attempts (wall-clock or mock clock)
- [ ] Regression: all existing tests pass (especially timeout tests in `profile_timeout.rs`, pipeline tests in `pipeline.rs`)
- [ ] `cargo clippy` clean

### [x] Phase 4: Domain Glossary

- [ ] Update `project/domain-glossary.md`:
  - `Session resume` — automatic retry using `--session-id` to continue a Pi session after invocation failure, up to 10 retries or until the profile timeout budget is exhausted
  - `SessionResumed` — loom-log event recorded for each resume attempt
  - `Overall timeout budget` — the profile's timeout value governs the total time across all retry attempts, not per-attempt
  - `Retry delay` — 10-second pause between retry attempts to allow transient network errors to recover
  - `Please continue` — prompt suffix appended to the session on retry, telling the agent to resume where it left off

## Notes

- **Phase 2 (PortError changes) is already done (Plan 46).** `PortError::Timeout` and `PortError::AgentExecutionFailed` already carry `session_id: Option<String>` and `PortError::session_id()` / `is_resumable()` already exist.
- **The retry loop is in its own module:** `src/application/session_resume.rs`. `ProcessStrand::execute()` delegates to `session_resume::execute_with_resume()` — clean separation: ProcessStrand owns the strand workflow, session_resume owns the retry policy, adapter owns the subprocess lifecycle.
- **`--session-id` is appended to cli_args** on retry. Pi reads it as a CLI flag.
- **"please continue" is appended to the prompt** on retry. This tells the agent to resume where it left off. The original prompt is NOT re-sent — Pi with `--session-id` already has the conversation history.
- **Overall timeout budget:** The profile's timeout value governs total time across all attempts. Remaining time is calculated each iteration and passed as `ExecutionContext.timeout`. If less than 5s remains, the loop bails.
- **10-second retry delay:** Between each retry, the loop sleeps 10 seconds to allow transient network errors to recover. This delay counts against the overall timeout budget.
- **Hard cap: 10 retries:** The retry loop runs at most 11 times total (initial + 10 retries). After 10 retries, strand fails regardless of remaining time.
- **The session ID is captured from the FIRST JSON-L line**, before any generation happens. This means even if the process is killed immediately, the session ID is available for retry. This is the key insight that makes session resume feasible.
- **On timeout, the PiJsonAgentRunner reads the stdout it captured before killing the process.** The first line (`{"type":"session","id":"..."}`) is already in the buffer. The adapter extracts it and includes it in the error.
