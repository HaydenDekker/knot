# Plan 47: Session Resume on Invocation Failure

## Related PRD

This plan contributes to [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md), implementing Story 5a (Session Resume on Invocation Failure).

This plan depends on Plan 46 (agent-json-adapter.md). The JSON adapter provides `session_id` capture in `AgentOutput.metadata` and `PortError::is_resumable()` — both prerequisites for session resume.

## Problem

When an agent invocation fails partway through (network error, provider timeout mid-stream, subprocess killed), Knot currently discards all partial work and marks the strand as failed. The user must manually `touch` the strand file to reprocess, which starts a **fresh Pi session** — the provider re-sends the full context from scratch.

This wastes provider capacity and increases cost: the conversation history already accepted by Pi is re-sent unnecessarily. If the failure was transient (network blip, momentary provider slowdown), resuming the same Pi session would complete the work without re-sending the full context.

## Target

When a resumable invocation failure occurs and a session ID was captured, Knot automatically retries the same invocation using `--session-id <id>` to continue the Pi session — up to 10 retry attempts **or** until the profile's overall timeout budget is exhausted (whichever comes first). On retry, a `"please continue"` message is appended to the session so the agent resumes where it left off. A 10-second delay between retries allows transient network errors to recover. On successful resume, the strand completes normally (transparent). On exhausted retries or budget expiry, the strand is marked failed and the existing failure path (loom-log, rig-log) takes over.

## Implementation Status: 🟡 In Progress

**Core session resume:** 2026-06-28, merged to main, version 0.20.0
**Empty response handling:** not yet implemented (Phases 5–6 below)

**Completed:** 2026-06-28 on branch `refactor/session-resume-on-invocation-failure`
**Merged to main:** 2026-06-28
**Version bumped:** 0.19.0 → 0.20.0 (MINOR — new feature, backwards compatible)

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

### [ ] Phase 4: Domain Glossary

- [ ] Update `project/domain-glossary.md`:
  - `Session resume` — automatic retry using `--session-id` to continue a Pi session after invocation failure, up to 10 retries or until the profile timeout budget is exhausted
  - `SessionResumed` — loom-log event recorded for each resume attempt
  - `Overall timeout budget` — the profile's timeout value governs the total time across all retry attempts, not per-attempt
  - `Retry delay` — 10-second pause between retry attempts to allow transient network errors to recover
  - `Please continue` — prompt suffix appended to the session on retry, telling the agent to resume where it left off
  - `KnotEmptyResponse` — loom-log event recorded when an agent exits successfully but produces no response text; treated as a resumable failure and retried with a stronger guidance prompt
  - `Empty response retry` — retry path that injects "you must provide a final response if finished or continue with the task" into the prompt, distinct from the generic "please continue" used for timeout/crash retries

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

### [ ] Phase 5: Empty Response Handling

An agent can exit cleanly (exit-code 0) but produce **empty output** — the provider returned no response text (e.g. immediate finish, empty `message_end`). This is treated as a resumable failure: Knot logs a `KnotEmptyResponse` event to the loom-log and retries with a stronger guidance prompt.

**What is already done** (from Phase 1):
- `KnotEmptyResponse` event variant exists in `src/domain/events.rs` with `attempt` counter
- Empty response detection in `session_resume.rs` (`output.stdout.trim().is_empty()`) — logs event and returns `PortError::Timeout` (resumable)
- Retry loop picks this up and retries with `prepare_retry()` (which appends `"please continue"`)
- `usecases.rs` maps `KnotEmptyResponse` to `ProcessingStatus::Failed` with no `last_tie_off_path`
- No tie-off is written — previous tie-off preserved, loom-log records the event

**What needs to change:**

1. **`src/application/session_resume.rs`** — `prepare_retry()` needs an overload for empty-response retries that injects a stronger guidance message instead of the generic `"please continue"`:
   ```rust
   /// Prepare for retry after an empty response.
   /// Injects guidance telling the agent it must produce output.
   fn prepare_empty_response_retry(
       mut cli_args: Vec<String>,
       mut prompt: String,
       session_id: &Option<String>,
   ) -> (Vec<String>, String) {
       if let Some(sid) = session_id {
           cli_args.push("--session-id".to_string());
           cli_args.push(sid.clone());
       }
       prompt.push_str("\n\nyou must provide a final response if finished or continue with the task");
       (cli_args, prompt)
   }
   ```

2. **Retry loop** — distinguish empty-response failures from other failures:
   - When `output.stdout.trim().is_empty()` on the first attempt: flag that the retry should use empty-response guidance
   - When `output.stdout.trim().is_empty()` on a retry attempt: same — continue retrying with empty-response guidance
   - When any other resumable error (timeout, crash): use existing `prepare_retry()` with `"please continue"`
   - Implementation: add a `was_empty_response: bool` flag that controls which prepare function is called

3. **`KnotEmptyResponse` event** — already emitted per-attempt with incrementing `attempt` counter. No change needed.

**Behaviour summary:**
- Agent finishes with empty output → `KnotEmptyResponse` logged → retry with stronger guidance prompt → agent provides real output → `KnotCompleted`, tie-off written. User sees `KnotEmptyResponse` in loom-log but the strand succeeds.
- Agent finishes with empty output → retry with guidance → agent again returns empty → `KnotEmptyResponse` logged again → retry continues until budget exhausted → `KnotFailed` + `TimeoutExceeded` in rig-log.
- The tie-off file records only complete final responses. The loom-log records intermittent errors. The rig-log receives the final timeout failure.

**Tests (unit with mock runner):**
- `empty_response_retries_with_guidance_prompt()` — mock returns `[Ok(empty), Ok(real)]` → retry uses empty-response guidance, second attempt succeeds
- `empty_response_multiple_retries_logs_attempts()` — mock returns `[Ok(empty), Ok(empty), Ok(real)]` → two `KnotEmptyResponse` events with attempt 1 and 2, then success
- `empty_response_exhausted_budget()` — mock returns `[Ok(empty) × N]` → budget exhausted, `KnotFailed`
- `empty_response_then_timeout_mixed()` — mock returns `[Ok(empty), Err(timeout)]` → first retry uses empty guidance, subsequent retry uses generic guidance

**Tests (integration):**
- Integration test in `tests/session_resume.rs`: mock pi that returns empty JSON-L (`session` + `agent_end` with empty content) on first call, real content on second

**Existing tests to update:**
- `empty_response_first_attempt_no_retry_budget()` — existing test already verifies empty response with no budget → verify it still passes with new guidance prompt path

### [ ] Phase 6: Post-Merge Reliability Fixes (Commit)

Uncommitted changes (2026-06-29) — targeted fixes to adapter subprocess lifecycle and test stability, discovered during session-resume integration testing.

**Process group cleanup** (`src/adapters/pi_json.rs`, `src/adapters/pi_stdio.rs`):
- Child processes are now spawned in their own process group via `setpgid(0, 0)` in `pre_exec`
- Timeout kills use `kill(-pid, SIGKILL)` (negative PID) to kill the **entire process group**, including any subprocesses the child spawned — prevents orphaned processes
- `pi_stdio.rs` additionally wraps `wait_with_output()` in a background thread with a 2× timeout deadline, so the main thread doesn't block forever if pipes are held open by orphaned subprocesses

**Test stability** (`tests/adapter_integration.rs`, `tests/agent_integration.rs`, `tests/session_resume.rs`, `tests/helpers.rs`):
- Global `TEST_MUTEX` serialises tests that modify process-global state (`PATH` / env vars), preventing race conditions from parallel test execution; poisoned locks from panics are recovered gracefully
- New helper `wait_for_loom_log_event_with_deadline()` uses caller-provided deadlines instead of fixed timeouts — accounts for the time long operations (e.g. timeouts) take before the event appears
- `handle.abort()` moved **before assertions** in session_resume tests so loom-log is fully flushed before reading
- Reduced test timeout profiles (120s → 60s, 15s → 1s) and retry delays (100ms → 50ms) for faster CI without changing behaviour

> **Action required:** commit these changes before proceeding with plan-completion.

### Completion Notes

- **432 unit tests pass**, clippy clean
- **No per-attempt timeout cap** — first attempt uses full profile timeout budget. Retries only happen when invocation fails *before* budget exhaustion (transient network error, provider crash mid-stream). If the first attempt actually hits the full timeout, no retry is warranted — budget is genuinely exhausted.
- **23 new tests** across 5 files: domain events (5), session_resume module (13), ProcessStrand integration (3), integration test file `tests/session_resume.rs` (7)

### Bugfix: Empty response on first attempt skips retry (2026-07-15)

When an agent returned empty output on the first invocation, the session ID from `AgentOutput.metadata` was never captured before returning the resumable error — so the error carried `session_id: None` and no retry was attempted. This meant the "please continue" retry loop only worked for crashes/timeouts, not for empty responses on the first attempt.

- **What was wrong:** `execute_with_resume_internal()` checked `output.stdout.trim().is_empty()` and returned `PortError::Timeout { session_id: session_id.clone() }` — but `*session_id` was still `None` at that point because the session ID capture from `output.metadata` lived *after* the early return.
- **How it was fixed:** Extract `session_id` from `output.metadata` before returning the empty-response error, so the caller sees a resumable error with a valid session ID and the retry loop fires correctly. Added an assertion to the existing test `empty_response_first_attempt_logs_knot_empty_response` to verify the session ID is now present.
