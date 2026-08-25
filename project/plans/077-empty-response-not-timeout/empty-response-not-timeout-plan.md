# Plan 077: Empty Response Is Not a Timeout

## Related Plans

Bugfix companion to [078 Final Response Request](../078-final-response-request/final-response-request-plan.md).
077 corrects the *classification* of an abrupt turn-end; 078 builds on the new
error variant to make the situation *recoverable* by re-entering the session.
077 is safe to ship on its own: the failure becomes accurate and visible
(failed tie-off) instead of a spurious timeout with the work silently lost.

## Problem

When a pi session ends its turn abruptly — pi exits with code 0 but the agent
produced **no final response** (e.g. `agent_end` carries only intermediate
`stopReason: "toolUse"` messages, or the provider stopped generating) — Knot
reports a **timeout** even though nothing timed out.

Code path:

1. `PiJsonAgentRunner::execute` (`src/adapters/pi_json.rs`) — exit code 0,
   `parse_stdout` finds no assistant message with `stopReason: "stop" |
   "length"` (see plan 051), so `response_text` is empty. Returns
   `Ok(AgentOutput { stdout: "", metadata: Some({ session_id, .. }) })`.
2. `execute_with_resume_internal` (`src/application/session_resume.rs`) — the
   first-attempt `Ok` + empty-stdout branch logs
   `LoomEvent::KnotEmptyResponse { attempt: 1 }` and then **returns
   immediately** with:
   ```rust
   Err(PortError::Timeout {
       message: "agent returned empty response".to_string(),
       session_id: sid,
   })
   ```
   Two defects here:
   - **Misclassification** — an empty response is wrapped in
     `PortError::Timeout` although no deadline was hit.
   - **Dead "resumable" comment** — the code says "treat as resumable … so
     the caller can retry via session-resume", but the retry loop below is
     never entered on this path (early return), and the caller
     (`ProcessStrand`) has no retry of its own. The session ID is captured
     and then thrown away.
3. `TieOffOutcome::derive` (`src/domain/entities.rs`) — classifies **any**
   `PortError::Timeout` as `TimeoutSkipped`.
4. `ProcessStrand::execute_inner` (`src/application/usecases/process_strand.rs`)
   — for `TimeoutSkipped`:
   - appends `RigLogEvent::TimeoutExceeded` to the rig-log (**the spurious
     timeout event** — `error: "timeout: agent returned empty response"`),
   - skips the tie-off write entirely (`should_write_tie_off() == false`),
   - logs `KnotFailed` + `StrandProcessed { error }` to the loom-log,
   - removes the queued event file (consume-on-failure) — **the work is
     lost**, never retried, and no trace remains in the tie-off file.

Observed symptoms: rig-log `TimeoutExceeded`, loom-log
`KnotFailed: timeout: agent returned empty response`, `state.json` knot
status `Failed`, no tie-off section, event gone.

## Target

- A new port error `PortError::AgentNoResponse { message, session_id }` —
  semantically distinct from `Timeout`: the agent stopped, it did not run
  out of time.
  - `Display`: `no final response: {message}` (the tie-off section will
    read `Processing failed: no final response: agent returned empty
    response`).
  - `session_id()` extracts the session ID; `is_resumable()` returns
    `true` (a session with history can be re-entered — used by 078).
  - `is_session_resumable()` (`src/application/ports.rs`) works unchanged
    (it composes `session_id.is_some() && is_resumable()`).
- Both empty-response constructions in `session_resume.rs` (first-attempt
  branch and in-loop retry branch) produce `AgentNoResponse` instead of
  `Timeout`.
- `TieOffOutcome::derive` maps `AgentNoResponse` →
  `Failed { error }` — an **error tie-off is written** (status `failed`,
  content `Processing failed: …`), and `is_timeout()` is `false`, so **no
  `TimeoutExceeded` rig-log event** is emitted.
- Genuine timeouts are untouched: the adapter's SIGKILL timeout and the
  retry-loop's "overall timeout budget exhausted" remain
  `PortError::Timeout` → `TimeoutSkipped` → rig-log. The rig-log stays the
  record of real deadline breaches only.
- Terminal state after this plan (no retry yet — that is 078):
  - tie-off file gains a `failed` section recording the failure,
  - loom-log: `KnotProcessing`, `KnotEmptyResponse`, `KnotFailed`,
    `StrandProcessed`,
  - rig-log: **empty**,
  - queued event consumed (as today — consume-on-failure is unchanged).
- `KnotStatus` derivation needs no change: the terminal loom event becomes
  `KnotFailed` (last_error carries the "no final response" message); the
  existing `KnotEmptyResponse` mapping stays valid for in-flight views.

Non-goals:

- No retry / session re-entry (078).
- No new loom-log or rig-log event variants.
- No change to the `PiJsonAgentRunner` parsing (plan 051's filtering stays).
- No document-format change → no `knot-update` migration entry.

## Existing Tests

| Test | File | What it covers | Status after this plan |
|------|------|----------------|------------------------|
| `empty_response_first_attempt_logs_knot_empty_response` | `src/application/session_resume.rs` | First-attempt empty → `KnotEmptyResponse` + error | **Must change** — asserts `PortError::Timeout`; will assert `PortError::AgentNoResponse` |
| `empty_response_retry_logs_knot_empty_response` | `src/application/session_resume.rs` | In-loop empty on retry → `KnotEmptyResponse(attempt 2)` | Unaffected (sequence ends in success) — the in-loop error type changes but is not asserted |
| `process_strand_timeout_skip_tieoff_write_rig_log` | `src/application/usecases/process_strand.rs` | Real `Timeout` → tie-off skipped, rig-log `TimeoutExceeded` | Unaffected (genuine timeout) |
| `process_strand_non_timeout_error_writes_tieoff` | `src/application/usecases/process_strand.rs` | Non-timeout error → tie-off written | Unaffected |
| `process_strand_retry_transparent_success` / `_exhausted_fails` / `_no_retry_stdio` | `src/application/usecases/process_strand.rs` | Session-resume flows with real `Timeout` | Unaffected |
| `TieOffOutcome` derive tests | `src/domain/entities.rs` | `Ok` → `Produced`, `Timeout` → `TimeoutSkipped`, other → `Failed` | **Extend** — add `AgentNoResponse` → `Failed` |
| `PortError` unit tests (`session_id`, `is_resumable`, Display) | `src/application/ports.rs` | Error helpers | **Extend** — add `AgentNoResponse` cases |
| `test_session_resume_*` | `tests/session_resume.rs` | Integration retry flows with real `Timeout` | Unaffected |

## Test Gaps

- No test for the *end-to-end* empty-response outcome at the `ProcessStrand`
  level (what tie-off is written, what hits the rig-log).
- No unit test that `AgentNoResponse` is resumable / carries a session ID /
  displays distinctly from `Timeout`.
- No test pinning `TieOffOutcome::derive(Err(AgentNoResponse)) == Failed`.

## Phases

### Phase 0: Failing Tests (characterisation)

1. `src/domain/entities.rs` — `TieOffOutcome` tests:
   - `derive_agent_no_response_is_failed` — `derive(Err(PortError::AgentNoResponse { message: "agent returned empty response", session_id: Some(…) )})` → `Failed { error }` where `error` contains `no final response`; `should_write_tie_off() == true`; `tie_off_status() == Some(Failed)`; `is_timeout() == false`; `error_message()` is `Some`.
2. `src/application/ports.rs` — `PortError` tests:
   - `agent_no_response_is_resumable` — `is_resumable() == true`; `session_id()` returns the captured ID; `Display` starts with `no final response:`.
   - `agent_no_response_without_session` — `session_id() == None`; `is_session_resumable(&None, &err) == false`.
3. `src/application/session_resume.rs` — update
   `empty_response_first_attempt_logs_knot_empty_response` to assert
   `PortError::AgentNoResponse` (message `agent returned empty response`,
   session ID captured from output metadata) instead of
   `PortError::Timeout`. This test fails before the fix — it is the bug
   reproduction.
4. `src/application/usecases/process_strand.rs` — new
   `execution_tests` test
   `process_strand_empty_response_writes_failed_tieoff_no_rig_log`:
   - `MockAgentRunner` returns `Ok(AgentOutput { stdout: "", metadata: Some({ session_id: Some("sess-abc") }) })`.
   - Assert: tie-off **appended** with `TieOffStatus::Failed` and content
     containing `no final response`; loom-log order
     `KnotProcessing`, `KnotEmptyResponse`, `KnotFailed`,
     `StrandProcessed` (error contains `no final response`, **not**
     `timeout`); rig-log **empty**; `execute()` returns `Ok`.

### Phase 1: `PortError::AgentNoResponse`

`src/application/ports.rs`:

1. Add variant (after `Timeout`):
   ```rust
   /// The agent session ended without producing a final response.
   ///
   /// Distinct from `Timeout`: no deadline was exceeded — the session
   /// simply stopped (e.g. provider returned immediately, turn ended with
   /// only intermediate tool-use messages). Resumable when a session ID
   /// was captured (plan 078 re-enters the session to request the final
   /// response).
   AgentNoResponse {
       message: String,
       session_id: Option<String>,
   },
   ```
2. `Display`: `write!(f, "no final response: {message}")`.
3. `session_id()`: add `PortError::AgentNoResponse { session_id, .. }` to
   the matching arm (alongside `Timeout` / `AgentExecutionFailed`).
4. `is_resumable()`: add `PortError::AgentNoResponse { .. }`.

### Phase 2: Reclassify in `session_resume.rs`

`src/application/session_resume.rs`:

1. First-attempt branch (`if let Ok(output) = &result`, empty stdout):
   replace `PortError::Timeout` with `PortError::AgentNoResponse` (same
   message `agent returned empty response`, same session ID capture).
   Update the surrounding comment — it is *not* "treated as resumable" by
   the caller in this plan; 078 wires the retry.
2. In-loop branch (retry returned `Ok` with empty stdout): replace
   `PortError::Timeout` with `PortError::AgentNoResponse`.

### Phase 3: `TieOffOutcome::derive`

`src/domain/entities.rs`:

1. `derive`: add an arm matching
   `PortError::AgentNoResponse { .. }` → `Self::Failed { error: err.to_string() }`
   (before the catch-all; the existing `Timeout` arm stays
   `TimeoutSkipped`).
2. Update the enum-level doc ("Derived from `Result<AgentOutput,
   PortError>` by classifying the error type: …") to state the three-way
   split: timeout → skip tie-off + rig-log; `AgentNoResponse` → failed
   tie-off; other → failed tie-off.

### Phase 4: Verify + Regression

- `cargo test` — full suite green; in particular the plan-051 adapter
  tests, the rig-log tests (`src/adapters/outbound/rig_log.rs`), and
  `tests/session_resume.rs` are unaffected.
- `cargo clippy --all-targets` clean.
- Manual rig check (optional): with a profile pointed at a stub that emits
  `session` + `agent_end` with only a `toolUse` message and exits 0, drop a
  strand; verify tie-off gets a `failed` section, rig-log stays clean.

### Phase 5: Docs + Version

1. `docs/troubleshooting.md` — event table: keep the `TimeoutExceeded` row
   (now strictly "agent session exceeded the profile timeout"); add a row
   for the knot-status `Failed` with `no final response: …` — "agent ended
   its turn without a final response; a failed tie-off section was written
   (check provider/model health; see plan 078 for automatic
   final-response requests)".
2. `docs/concepts.md` — where timeouts are described: note that an abrupt
   turn-end (empty response) is a *failure*, not a timeout.
3. `docs/release-notes.md` — v0.36.1 entry (bugfix): empty response no
   longer misreported as `TimeoutExceeded`; failed tie-off written.
4. Bump `Cargo.toml` to `0.36.1`; `cargo install --path .`.

## Notes

- The `KnotEmptyResponse` loom event keeps its name and shape — it is a
  per-attempt observability record and remains accurate ("the knot got no
  response on attempt N").
- The `get_knot_status.rs` hard-coded string
  `"agent returned empty response"` for `KnotEmptyResponse` stays; the
  terminal `KnotFailed` event will now carry the fuller
  `no final response: …` message.
- `PortError` is matched exhaustively in exactly three places
  (`Display`, `session_id`, `is_resumable` — all in `ports.rs`); the new
  variant breaks nothing elsewhere (verified by `rg`).
- 078 consumes `AgentNoResponse`'s `session_id` to re-enter the session;
  keeping the field on this variant (rather than reusing
  `AgentExecutionFailed`) is deliberate — it keeps the retry gate
  (`is_session_resumable`) honest: only errors that genuinely leave a live
  session behind are resumable.
