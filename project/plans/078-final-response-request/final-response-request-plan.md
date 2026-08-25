# Plan 078: Final-Response Request — Re-enter the Session When the Agent Stops Without a Final Response

## Related Plans

Builds on [077 Empty Response Is Not a Timeout](../077-empty-response-not-timeout/empty-response-not-timeout-plan.md)
(requires `PortError::AgentNoResponse`). Reuses the session-resume machinery
introduced by `session-resume-on-invocation-failure.md` (retry loop,
`--session-id` re-entry, budget tracking) and mirrors the follow-up pattern
of `inject_event_request` (event enforcement). Pairs with
[051 Pi JSON Final Response Filter](../051-pi-json-final-response-filter/pi-json-final-response-filter-plan.md),
which is what makes "no final response" detectable in the first place.

## Problem

When an agent ends its turn abruptly (pi exits 0, no final assistant
message — plan 077's scenario), Knot currently:

- writes a `failed` tie-off section (after 077),
- logs `KnotFailed`,
- consumes the queued event,
- and **gives up** — the work is lost.

But the pi session is still alive on disk, and Knot already knows how to
re-enter it: the session-resume retry loop passes `--session-id <id>` and a
follow-up prompt, exactly like `inject_event_request` does for missing
events. Today only **mid-stream failures** (adapter timeout, non-zero exit)
enter that loop — and only from the *error* path. The first-attempt
empty-response path early-returns before the loop is reached, so the one
case where the agent is most likely to finish on a nudge (it stopped, it
didn't crash) is the one case that never gets a nudge.

## Target

When the agent ends a turn without producing a final response and a session
ID was captured, Knot re-enters the session and requests the tie-off:

> **Please produce your final response, or continue if you have not
> finished.**

Behavioural contract:

- **Trigger** — first attempt returns `Ok` with empty `stdout`
  (`KnotEmptyResponse { attempt: 1 }` is already logged) **and** the output
  metadata carries a `session_id`. No session ID → no re-entry (stdio
  adapter / unparseable output); the 077 terminal failure stands.
- **Retry loop** — the existing `execute_with_resume` loop, unchanged in
  shape: budget check (bail when `remaining < MIN_REMAINING_SECS = 5s`),
  `RETRY_DELAY` (10s, `KNOT_RETRY_DELAY_MS` override in tests),
  `SessionResumed { attempt }` loom event, execution with
  `--session-id <id>`, per-attempt `KnotEmptyResponse` for further empty
  results, `MAX_RETRIES = 10`.
- **Prompt** — the retry prompt is the original prompt plus the
  final-response request (replacing the current `"\n\nplease continue"`
  suffix, see Phase 2). The session already holds the full conversation;
  re-sending the original prompt with the appended request matches the
  existing mid-stream retry behaviour.
- **Success** — a non-empty follow-up response is the tie-off: normal
  `Produced` path (tie-off section, `KnotCompleted`, `StrandProcessed`,
  event dispatch, late removal, git commit). The strand succeeds
  transparently, same as a resumed mid-stream failure today.
- **Exhaustion** — when retries or the budget run out, the terminal error
  reflects the *cause*:
  - last failure was an empty response → `PortError::AgentNoResponse`
    (failed tie-off, no rig-log timeout), message
    `agent returned empty response after {N} attempts (session resume
    exhausted)` where `N` counts all attempts (initial + retries);
  - last failure was a genuine timeout (adapter kill mid-retry) →
    `PortError::Timeout` (`TimeoutSkipped`, rig-log `TimeoutExceeded`);
  - the retry loop bails because the **profile timeout budget** is
    exhausted → `PortError::Timeout` (`overall timeout budget exhausted
    after N attempt(s)`) — the budget *did* run out; the per-attempt
    `KnotEmptyResponse` events in the loom-log show the root cause.
- **Observability** — no new event types. The loom-log tells the whole
  story with existing events: `KnotProcessing` →
  `KnotEmptyResponse(1)` → (`SessionResumed(n)` → `KnotEmptyResponse(n+1)`)…
  → `KnotCompleted` (nudge worked) or `KnotFailed` (exhausted).

Non-goals:

- No new profile/knot configuration knob (the nudge always happens; it is
  bounded by the existing `MAX_RETRIES` / profile timeout budget).
- No change to event enforcement (`inject_event_request`) or to the
  mid-stream retry *gating* (still `is_resumable() + session_id`).
- No change to pi, adapters, or document formats → no `knot-update`
  migration entry.

## Existing Tests

| Test | File | What it covers | Status after this plan |
|------|------|----------------|------------------------|
| `retry_appends_please_continue` | `src/application/session_resume.rs` | Retry prompt suffix | **Must change** — asserts `please continue`; will assert the final-response request |
| `empty_response_first_attempt_logs_knot_empty_response` | `src/application/session_resume.rs` | First-attempt empty → error + `KnotEmptyResponse` | **Must change** — with a session ID the call now *retries*; the test becomes the no-session-ID case (see Phase 0) |
| `retry_succeeds_on_first_retry`, `retry_exhausted_then_fails`, `retry_stops_on_*`, `no_retry_*`, `session_id_captured_from_error`, `successful_retry_transparent`, `first_attempt_succeeds_no_retry` | `src/application/session_resume.rs` | Mid-stream retry flows | Unaffected (error path untouched) — except prompt-suffix assertions |
| `empty_response_retry_logs_knot_empty_response` | `src/application/session_resume.rs` | In-loop empty → `KnotEmptyResponse(2)` | Unaffected |
| `process_strand_retry_transparent_success` / `_exhausted_fails` / `_no_retry_stdio` | `src/application/usecases/process_strand.rs` | ProcessStrand retry flows with real `Timeout` | Unaffected |
| `test_session_resume_success` / `_transparent_on_success` / `_exhausted` / `_non_resumable_error` | `tests/session_resume.rs` | Integration retry flows | Unaffected |

## Test Gaps

- No test that a first-attempt empty response **re-enters** the session
  (`--session-id` present on attempt 2).
- No test that the follow-up prompt is the final-response request.
- No test of the exhausted-empty-response terminal state (failed tie-off,
  no rig-log timeout, `N`-attempt message).
- No integration test of the full nudge→success pipeline (tie-off
  `Produced` from a resumed session after an empty first turn).

## Phases

### Phase 0: Failing Tests

1. `src/application/session_resume.rs` — unit tests:
   - `empty_response_first_attempt_triggers_retry` — runner sequence
     `[Ok(empty, "sess-abc"), Ok("final response", "sess-abc")]`; expect
     `Ok` with stdout `final response`; runner called twice; context[1]
     `extra_args` contain `--session-id` + `sess-abc` and `prompt` contains
     `Please produce your final response, or continue if you have not
     finished.`; loom-log: `KnotEmptyResponse { attempt: 1 }`,
     `SessionResumed { attempt: 1 }`.
   - `empty_response_without_session_id_no_retry` — runner sequence
     `[Ok(AgentOutput { stdout: "", metadata: None })]`; expect
     `Err(PortError::AgentNoResponse)` with `session_id == None`;
     runner called **once**; loom-log: only `KnotEmptyResponse { attempt: 1
     }` (no `SessionResumed`).
   - `empty_response_exhausted_returns_no_response_error` — runner
     sequence `[Ok(empty, "sess-abc"); ×11]`, profile timeout `None` (no
     budget), zero delay; expect `Err(PortError::AgentNoResponse)` with
     message containing `after 11 attempts` and `exhausted`; loom-log:
     11 × `KnotEmptyResponse` + 10 × `SessionResumed`; runner called 11
     times.
   - `empty_response_retry_timeout_terminal_is_timeout` — sequence
     `[Ok(empty, sid), Err(Timeout { sid }), …]` until exhaustion; expect
     terminal `PortError::Timeout` (last failure was a real timeout).
   - Update `empty_response_first_attempt_logs_knot_empty_response` per the
     "Existing Tests" note (it becomes the no-session-ID reproduction).
2. `src/application/usecases/process_strand.rs` — `execution_tests`:
   - `process_strand_empty_response_resumed_success` — sequence
     `[Ok(empty, "sess-abc"), Ok("final", "sess-abc")]` (zero retry delay);
     expect: tie-off **appended** `Produced` with content `final`; loom-log
     `KnotProcessing`, `KnotEmptyResponse`, `SessionResumed`,
     `KnotCompleted`, `StrandProcessed`; rig-log **empty**; no
     `KnotFailed`.
3. `tests/session_resume.rs` — integration tests (mocked ports, no
   `start_knot`):
   - `test_empty_response_requests_final_response` — same shape as
     `test_session_resume_success` but attempt 1 is `Ok(empty, sid)`;
     assert the tie-off section is `Produced` with the resumed text and
     the second invocation carried `--session-id`.
   - `test_empty_response_exhausted_writes_failed_tieoff` — sequence
     `[Ok(empty, sid); ×12]`, `KNOT_RETRY_DELAY_MS=0`, profile without a
     timeout; assert: tie-off appended `Failed` with content containing
     `no final response` and `after 11 attempts`; loom-log has 10 ×
     `SessionResumed` + 11 × `KnotEmptyResponse` + `KnotFailed`;
     rig-log **empty**; queued event removed (late-removal invariant).

### Phase 1: Route First-Attempt Empty Response into the Retry Loop

`src/application/session_resume.rs`,
`execute_with_resume_internal` — restructure the first-attempt block:

```rust
let result = agent_runner.execute_with_config(/* first attempt, unchanged */);

let mut first_error;
if let Ok(output) = result {
    if output.stdout.trim().is_empty() {
        // Abrupt turn-end: log, then request the final response by
        // re-entering the session (plan 078).
        let sid = output.metadata.as_ref()
            .and_then(|m| m.session_id.clone());
        loom_log.append(LoomEvent::KnotEmptyResponse { /* attempt: 1 */ })?;
        if sid.is_none() {
            return Err(PortError::AgentNoResponse {
                message: "agent returned empty response (no session id — cannot request final response)".to_string(),
                session_id: None,
            });
        }
        *session_id = Some(sid.clone());
        first_error = PortError::AgentNoResponse {
            message: "agent returned empty response".to_string(),
            session_id: Some(sid),
        };
        // fall through to the retry loop
    } else {
        if let Some(ref metadata) = output.metadata { /* capture sid */ }
        return Ok(output);
    }
} else {
    let err = result.unwrap_err();
    // Existing resumability gate, unchanged.
    let sid = err.session_id().cloned();
    if !err.is_resumable() || sid.is_none() {
        if let Some(sid) = sid { *session_id = Some(sid); }
        return Err(err);
    }
    *session_id = Some(sid);
    first_error = err;
}

// --- Retry loop (existing, see Phases 2–3 for changes) ---
for attempt in 1..=MAX_RETRIES { … }
```

Notes:

- The in-loop empty-response branch already sets
  `first_error = PortError::…` and `continue`s (it is `AgentNoResponse`
  after 077) — no change needed there.
- The in-loop line `if let Some(sid) = first_error.session_id() {
  *session_id = … }` keeps the session ID current for both failure kinds.

### Phase 2: The Final-Response Request Prompt

`src/application/session_resume.rs`, retry loop:

1. Replace the suffix:
   ```rust
   // was: prompt.push_str("\n\nplease continue");
   prompt.push_str(
       "\n\nPlease produce your final response, or continue if you have not finished.",
   );
   ```
   One message for **all** session resumes: "continue if you have not
   finished" covers the mid-stream case; "produce your final response"
   covers the abrupt-stop case.
2. Extract to a `const FINAL_RESPONSE_REQUEST: &str` with a doc comment
   (it is user-facing agent text — keep it greppable).
3. Update `retry_appends_please_continue` →
   `retry_appends_final_response_request` (assert the new text).
4. Update the module doc comment ("retries the invocation using
   `--session-id <id>` …") to mention the final-response request as the
   retry prompt.

### Phase 3: Cause-Accurate Exhaustion Error

`src/application/session_resume.rs`, end of `execute_with_resume_internal`:

```rust
// Exhausted all retries — classify by the last failure.
match &first_error {
    PortError::Timeout { .. } => Err(PortError::Timeout {
        message: format!("session resume exhausted {MAX_RETRIES} retries{budget_suffix}", …),
        session_id: session_id.clone(),
    }),
    _ => Err(PortError::AgentNoResponse {
        message: format!(
            "agent returned empty response after {} attempts (session resume exhausted)",
            MAX_RETRIES + 1,
        ),
        session_id: session_id.clone(),
    }),
}
```

- The two **budget-bail** returns inside the loop stay
  `PortError::Timeout` (the profile budget genuinely ran out); their
  messages already say `overall timeout budget exhausted after N
  attempt(s)`.
- 077's `TieOffOutcome::derive` then yields: exhausted-empty → `Failed`
  tie-off; budget-exhausted / last-timeout → `TimeoutSkipped` + rig-log.

### Phase 4: Verify + Regression

- `cargo test` — full suite green; watch in particular:
  - `session_resume.rs` unit tests (prompt-suffix and empty-response
    tests),
  - `process_strand.rs` session-resume tests,
  - `tests/session_resume.rs` integration suite,
  - `tests/pipeline.rs` / `tests/tie_off.rs` (no behaviour change for
    non-empty responses).
- `cargo clippy --all-targets` clean.
- Manual rig check (optional): profile with a stubbed pi that exits 0
  after a `toolUse`-only first turn and answers "Done." on the second;
  drop a strand; verify the tie-off section is `Produced` with "Done."
  and the loom-log shows the nudge chain.

### Phase 5: Docs + Version

1. `docs/concepts.md` — session-resume paragraph: an abrupt turn-end (empty
   final response) now triggers the same session re-entry, with the
   final-response request as the nudge.
2. `docs/troubleshooting.md` — extend the plan-077 row: "Knot re-enters the
   session up to 10 times (or the profile timeout budget) asking for the
   final response; if it still fails, check provider/model health and
   consider a larger profile `timeout` so the nudge attempts fit the
   budget."
3. `docs/release-notes.md` — v0.37.0 entry (feature): final-response
   request on abrupt turn-end, transparent recovery, cause-accurate
   terminal errors.
4. `knot-update` skill: **no migration entry** (no document-format
   change) — but note in the release entry that no rig-document migration
   is required.
5. Bump `Cargo.toml` to `0.37.0`; `cargo install --path .`.

## Notes

- **Why not a new loom event?** `KnotEmptyResponse` + `SessionResumed`
  already name every step of the nudge chain; a dedicated
  `FinalResponseRequested` event would duplicate `SessionResumed` with a
  different label. Reconsider only if rig reviewers cannot distinguish
  nudge retries from mid-stream retries in practice — the loom-log entry
  order (a `SessionResumed` immediately following a `KnotEmptyResponse`)
  is already distinguishable.
- **Why re-send the original prompt on retry?** Consistency with the
  existing mid-stream retry (the loop re-sends `prompt` each attempt).
  `inject_event_request` skips the original text, but that function is a
  one-shot post-success follow-up; the retry loop predates it and its
  behaviour is proven. Keeping one mechanism is simpler than special-casing
  the empty-response prompt shape.
- **Interaction with event enforcement (059):** if the nudged response
  arrives, it flows through the normal success path — including the
  `KnotEventsMissing` follow-up if the knot was expected to emit events
  and the final response still lacks them. The two mechanisms compose
  (nudge for the response, then nudge for the events), each bounded
  independently.
- **Budget math:** with the default 10s `RETRY_DELAY`, 10 retries cost up
  to ~100s of delay plus attempt time. Profiles with a small `timeout`
  (e.g. 60s) will bail early (`MIN_REMAINING_SECS = 5s`) — usually after
  1–2 nudge attempts. That is the intended shape: the budget, not the
  retry cap, is the primary bound for timed profiles; `MAX_RETRIES` only
  binds untimed profiles.
- **Idempotency:** the nudge re-enters the *same* pi session, so any tool
  side effects the agent already performed are not repeated by Knot —
  only the agent's own continuation may act on context. This is the same
  trust model as mid-stream session resume.
