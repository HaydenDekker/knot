# Plan 47: Session Resume on Invocation Failure

## Related PRD

This plan contributes to [System Reliability — Messaging Control, Replay and Rollback](../prds/prd-system-reliability.md), implementing Story 5a (Session Resume on Invocation Failure).

This plan depends on Plan 46 (agent-json-adapter.md). The JSON adapter provides `session_id` capture in `AgentOutput.metadata` and `PortError::is_resumable()` — both prerequisites for session resume.

## Problem

When an agent invocation fails partway through (network error, provider timeout mid-stream, subprocess killed), Knot currently discards all partial work and marks the strand as failed. The user must manually `touch` the strand file to reprocess, which starts a **fresh Pi session** — the provider re-sends the full context from scratch.

This wastes provider capacity and increases cost: the conversation history already accepted by Pi is re-sent unnecessarily. If the failure was transient (network blip, momentary provider slowdown), resuming the same Pi session would complete the work without re-sending the full context.

## Target

When a resumable invocation failure occurs and a session ID was captured, Knot automatically retries the same invocation using `--session-id <id>` to continue the Pi session — up to `max_retries` (configurable per knot, default 1). On successful resume, the strand completes normally (transparent). On exhausted retries, the strand is marked failed and the existing failure path (loom-log, rig-log) takes over.

## Implementation Status: ⬜ Draft

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
- No `max_retries` field on Knot entity
- No test for `--session-id` passthrough on retry
- No integration test for session resume end-to-end
- No test for resumable vs. fatal failure classification in use-case

## Phases

### Phase 0: Domain — max_retries on Knot, SessionResumed Event

Work in domain layer: `src/domain/entities.rs`, `src/domain/events.rs`, `src/domain/knot_file.rs`.

**Changes:**

1. **`src/domain/entities.rs`** — Add `max_retries: u32` field to `Knot`:
   ```rust
   /// Maximum number of session-resume retry attempts on invocation failure.
   /// Default: 1 (one retry). Set to 0 to disable session resume for this knot.
   #[serde(default = "default_max_retries")]
   pub max_retries: u32,
   ```

2. **`src/domain/events.rs`** — Add `SessionResumed` variant to `LoomEvent`:
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

3. **`src/domain/knot_file.rs`** — Parse `max-retries` from knot YAML frontmatter:
   - Add `max_retries: Option<u32>` to `RawFrontmatter`
   - Extract during `parse()` → set on `KnotFile`
   - Thread through to `Knot` entity construction

4. **`src/application/ports.rs`** — Add `is_session_resumable()` helper function (not on `PortError` — it takes both the error AND the session ID as inputs, so it's a free function, not a method):
   ```rust
   /// Determine if a failed invocation should trigger a session-resume retry.
   pub fn is_session_resumable(session_id: &Option<String>, error: &PortError) -> bool {
       session_id.is_some() && error.is_resumable()
   }
   ```

**Tests (domain/application unit):**
- `knot_max_retries_default()` — missing `max-retries` → defaults to 1
- `knot_max_retries_zero()` — `max-retries: 0` → 0
- `knot_max_retries_from_yaml()` — `max-retries: 3` → 3
- `session_resumed_event_serialization()` — `SessionResumed` serialises/deserialises correctly
- `is_session_resumable_with_session_and_timeout()` — session_id + Timeout → `true`
- `is_session_resumable_with_session_and_execution_failed()` — session_id + AgentExecutionFailed → `true`
- `is_session_resumable_without_session()` — session_id None → `false`
- `is_session_resumable_command_not_found()` — CommandNotFound → `false`
- `knot_file_parse_max_retries()` — YAML `max-retries: 2` → KnotFile.max_retries = 2
- `knot_file_parse_missing_max_retries()` — missing → KnotFile.max_retries = 1

**Existing tests to update:**
- All `Knot` construction sites (tests, fixtures) need `max_retries: 1` default — handled by `#[serde(default)]`
- All `KnotFile` construction sites need `max_retries: 1` default
- `LoomEvent` match exhaustiveness — add `SessionResumed` arm

### Phase 1: Application — Retry Loop in ProcessStrand

Work in `src/application/usecases.rs`, `ProcessStrand::execute()`.

**Design:**

The retry loop wraps the `agent_runner.execute()` call. It is NOT in the adapter — retry policy is application-layer concern. The adapter just executes and returns.

```
let mut cli_args = build_base_cli_args();  // existing logic
let mut session_id: Option<String> = None;

for attempt in 0..=knot.max_retries {
    if attempt > 0 {
        // Append --session-id for retry
        cli_args.push("--session-id".into());
        cli_args.push(session_id.clone().unwrap());

        // Log SessionResumed event
        log_port.append(SessionResumed { ..., attempt, session_id: ..., ... })?;
    }

    let ctx = ExecutionContext { cli_args: cli_args.clone(), ... };
    let result = agent_runner.execute(ctx);

    match result {
        Ok(output) => {
            // Capture session_id for potential future retries
            session_id = output.metadata.as_ref().and_then(|m| m.session_id.clone());

            // Success — proceed to tie-off write, git commit, etc.
            // The retry is transparent: single KnotCompleted in loom-log.
            return Ok(());
        }
        Err(ref err) if attempt < knot.max_retries && is_session_resumable(&session_id, err) => {
            // Resumable failure — capture session_id from output if available, retry
            // session_id was captured by JsonSubprocessAgentRunner even on error
            // (it reads the first JSON-L line before the process is killed)
            continue;  // next iteration
        }
        Err(err) => {
            // Fatal failure OR retries exhausted
            // Fall through to existing error handling (KnotFailed, rig-log, etc.)
            return handle_error(err, ...);
        }
    }
}
```

Key details:

1. **Session ID capture on error:** The `JsonSubprocessAgentRunner` captures the session ID from the first JSON-L line (`{"type":"session","id":"..."}`). Even if the process is killed on timeout, the session ID was already parsed. On `PortError::Timeout` or `PortError::AgentExecutionFailed`, the adapter returns an error that includes the captured session ID.

   This means we need `PortError` variants to optionally carry metadata. OR we store the session ID in `ProcessStrand` between attempts. The simpler approach: the JSON adapter captures the session ID and stores it in the error context, or ProcessStrand extracts it from the previous attempt's output.

   **Decision:** On error, the `JsonSubprocessAgentRunner` does NOT return `AgentOutput` (it returns `Err(PortError)`). So the session ID must be extracted differently. Two options:
   - **Option A:** Add `session_id: Option<String>` to `PortError` variants (Timeout, AgentExecutionFailed). The adapter populates it when it captured a session ID before the error occurred.
   - **Option B:** Store session ID in a local variable in `ProcessStrand` between iterations, extracted from the `agent_end` line even on partial output.

   **Selected: Option A** — Add `session_id: Option<String>` to `PortError::Timeout` and `PortError::AgentExecutionFailed`. This keeps the session ID with the error it belongs to. The `is_session_resumable()` function checks the error's embedded session ID.

   Wait — this changes `PortError` which is a port type. Let me reconsider.

   **Better approach:** The retry loop in `ProcessStrand` maintains `session_id` as local state. On each `execute()` call, the result is `Result<AgentOutput, PortError>`. On `Ok(output)`, extract `output.metadata.session_id`. On `Err(err)`, the session ID is lost because `AgentOutput` is not returned.

   **Resolution:** Add `session_id: Option<String>` to `PortError::Timeout` and `PortError::AgentExecutionFailed`. This is a port-level change that stays in `ports.rs`. The `JsonSubprocessAgentRunner` populates it; the `SubprocessAgentRunner` leaves it as `None`. The retry loop reads `err.session_id()`.

   ```rust
   pub enum PortError {
       // ... existing variants ...
       Timeout { message: String, session_id: Option<String> },
       AgentExecutionFailed { message: String, session_id: Option<String> },
       // ...
   }

   impl PortError {
       pub fn session_id(&self) -> Option<&String> {
           match self {
               PortError::Timeout { session_id, .. }
               | PortError::AgentExecutionFailed { session_id, .. }
                   => session_id.as_ref(),
               _ => None,
           }
       }

       pub fn is_resumable(&self) -> bool {
           matches!(
               self,
               PortError::Timeout { .. } | PortError::AgentExecutionFailed { .. }
           )
       }
   }
   ```

   This is a breaking change to `PortError` consumers. All existing code that constructs `PortError::Timeout(msg)` or `PortError::AgentExecutionFailed(msg)` needs updating. Scope: `src/adapters/subprocess.rs`, `src/application/usecases.rs`, test files.

2. **`--session-id` passthrough:** On retry, append `--session-id <captured-id>` to `cli_args`. This is appended AFTER the base args (which already include `--mode json` if configured by Plan 46).

3. **Transparent on success:** If the retry succeeds, the loom-log shows only `KnotCompleted` (single entry). The `SessionResumed` entries are written for EACH retry attempt, but they are informational — they appear in the loom-log for traceability but don't change the strand's processing state.

   Actually, re-reading the PRD scenarios:
   - Scenario 3: "when I check the loom-log, then the strand shows a single successful KnotCompleted entry"
   - Scenario 8: "when I check the loom-log, then I see a SessionResumed entry"

   These seem contradictory. Scenario 3 says the retry is transparent (no noise). Scenario 8 says SessionResumed entries are logged.

   **Resolution:** `SessionResumed` entries are logged for traceability (scenario 8). But the strand's processing result is `KnotCompleted` — not `KnotFailed` followed by `KnotCompleted`. So the loom-log shows: `KnotProcessing` → `SessionResumed` (×N) → `KnotCompleted`. The user sees the retry attempts in the log but the strand is marked as completed, not failed.

4. **Fatal failures bypass the loop:** `CommandNotFound`, `ProfileNotFound` — these are config errors, not invocation errors. They fail immediately without entering the retry loop. The retry loop only wraps the `agent_runner.execute()` call, not the config resolution.

5. **`max_retries: 0` disables resume:** When `knot.max_retries == 0`, the loop runs once (attempt 0 only). Any failure is final.

**Tests (application unit with mock runner):**

Need a `ConfigurableAgentRunner` mock that returns a sequence of results (e.g., `Err, Ok` or `Err, Err`).

- `retry_succeeds_on_first_retry()` — mock returns `[Err(TIMEOUT), Ok]`, max_retries=1 → KnotCompleted, SessionResumed logged
- `retry_exhausted_then_fails()` — mock returns `[Err(TIMEOUT), Err(TIMEOUT)]`, max_retries=1 → KnotFailed, two SessionResumed logged
- `no_retry_when_disabled()` — max_retries=0, mock returns `Err` → KnotFailed immediately, no SessionResumed
- `no_retry_on_fatal_error()` — mock returns `Err(CommandNotFound)` → KnotFailed immediately, no SessionResumed
- `no_retry_when_no_session_id()` — mock returns `Err(TIMEOUT { session_id: None })` → KnotFailed immediately
- `retry_preserves_other_cli_args()` — `--session-id` appended, other args unchanged
- `session_id_captured_from_error()` — PortError::Timeout carries session_id, used in retry
- `successful_retry_transparent()` — loom-log shows KnotProcessing → SessionResumed → KnotCompleted (no KnotFailed)

### Phase 2: Adapter — Session ID on PortError

Work in `src/adapters/json_subprocess.rs` (Plan 46) and `src/adapters/subprocess.rs` (existing).

**Changes:**

1. **`src/application/ports.rs`** — Update `PortError` variants:
   - `Timeout(String)` → `Timeout { message: String, session_id: Option<String> }`
   - `AgentExecutionFailed(String)` → `AgentExecutionFailed { message: String, session_id: Option<String> }`
   - Add `session_id()` and `is_resumable()` methods

2. **`src/adapters/subprocess.rs`** — Update all `PortError::Timeout` and `PortError::AgentExecutionFailed` constructions to include `session_id: None`:
   ```rust
   return Err(PortError::Timeout {
       message: format!("..."),
       session_id: None,
   });
   ```

3. **`src/adapters/json_subprocess.rs`** — On error, include the captured session_id:
   ```rust
   return Err(PortError::Timeout {
       message: format!("..."),
       session_id: captured_session_id.clone(),
   });
   ```

**Tests (adapter unit):**
- `json_runner_timeout_includes_session_id()` — timeout error carries session_id from first line
- `json_runner_execution_failed_includes_session_id()` — non-zero exit error carries session_id
- `stdio_runner_timeout_no_session_id()` — stdio adapter returns session_id: None
- `stdio_runner_execution_failed_no_session_id()` — stdio adapter returns session_id: None

**Existing tests to update:**
- All `PortError::Timeout(msg)` → `PortError::Timeout { message: msg, session_id: None }`
- All `PortError::AgentExecutionFailed(msg)` → `PortError::AgentExecutionFailed { message: msg, session_id: None }`
- All pattern matches on these variants need updating
- `PortError::Display` impl needs updating
- `is_resumable()` test in Phase 0 tests

### Phase 3: Integration Tests and Verification

- [ ] Integration test: `test_session_resume_success()` — real `pi` binary, knot with `max-retries: 1`, simulate first-invocation failure (e.g., very short timeout that Pi doesn't complete within), verify resume attempt with `--session-id`, check loom-log for `SessionResumed` + `KnotCompleted`
- [ ] Integration test: `test_session_resume_exhausted()` — knot with `max-retries: 1`, both attempts timeout → `KnotFailed` in loom-log, `TimeoutExceeded` in rig-log
- [ ] Integration test: `test_session_resume_disabled()` — knot with `max-retries: 0`, first failure → immediate `KnotFailed`, no retry
- [ ] Integration test: `test_session_resume_stdio_no_retry()` — `invocation_mode: stdio` (Plan 46), failure → no retry (session_id never captured)
- [ ] Integration test: `test_session_resume_transparent_on_success()` — first fails, retry succeeds → loom-log has `SessionResumed` + `KnotCompleted`, no `KnotFailed`
- [ ] Regression: all existing tests pass (especially timeout tests in `profile_timeout.rs`, pipeline tests in `pipeline.rs`)
- [ ] `cargo clippy` clean

### Phase 4: Domain Glossary

- [ ] Update `project/domain-glossary.md`:
  - `Session resume` — automatic retry using `--session-id` to continue a Pi session after invocation failure
  - `max-retries` — per-knot configuration controlling session resume limit (default: 1)
  - `SessionResumed` — loom-log event recorded for each resume attempt

## Notes

- **Phase 2 (PortError changes) must be merged with Plan 46 or done atomically.** Changing `PortError` variants affects all consumers. If Plan 46 is already merged, Phase 2 extends its changes. If they're on the same branch, Phase 0 of this plan includes the PortError changes.
- **The retry loop is in the APPLICATION layer**, not the adapter. The adapter (JsonSubprocessAgentRunner) is a dumb execute-and-return. Retry policy, session ID tracking, and loom-log events are all in ProcessStrand.
- **`--session-id` is appended to cli_args**, not injected into the prompt or stdin. Pi reads it as a CLI flag.
- **The session ID is captured from the FIRST JSON-L line**, before any generation happens. This means even if the process is killed immediately, the session ID is available for retry. This is the key insight that makes session resume feasible.
- **On timeout, the JsonSubprocessAgentRunner reads the stdout it captured before killing the process.** The first line (`{"type":"session","id":"..."}`) is already in the buffer. The adapter extracts it and includes it in the error.
