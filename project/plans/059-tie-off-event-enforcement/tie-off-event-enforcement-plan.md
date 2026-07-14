# Plan 59: Tie-Off Event Enforcement

## Related PRD

This plan contributes to [Tie-Off Event Enforcement](../prds/prd-tie-off-event-enforcement.md), implementing Story 1 (Missing Event Detection and Logging) and Story 2 (Automatic Follow-Up Prompt).

The plan adds a post-processing step after successful knot completion: if the agent was instructed to emit events but produced none, Knot logs the failure and attempts one follow-up re-entry to remind the agent to provide events.

## Problem

When a knot is instructed to emit agent events (via `event:` subscriptions from consumer knots), the agent is told to include structured event blocks in its response. Agents frequently omit these blocks entirely — producing a useful response body but no events. The strand completes as "successful" and downstream consumer knots never fire, silently breaking the agent-to-agent workflow.

There is no visibility into this failure (no loom-log entry) and no automatic recovery mechanism.

## Target

After a knot completes successfully, Knot checks whether the agent was instructed to emit events (listener context was injected). If events were expected but the tie-off contains zero event blocks (not even `event: None`), Knot:

1. Appends a `KnotEventsMissing` entry to the loom-log
2. Re-enters the Pi session using `--session-id` with a follow-up prompt reminding the agent it must emit events
3. Parses the follow-up response for events and dispatches them if found
4. If still no events, logs a second `KnotEventsMissing` and stops (max one retry)

If no listener context was injected (no consumers listening), the check is skipped entirely. If `event: None` was emitted, the check passes — no enforcement needed.

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `src/domain/tieoff_parser.rs` tests | Event extraction from markdown blocks, `event: None` handling, multiple events | ✅ Green — 18 tests |
| `src/domain/events.rs` tests | `build_listener_context()`, `AgentEvent` construction, LoomEvent variants | ✅ Green — 20+ tests |
| `src/application/session_resume.rs` tests | Session retry loop, `--session-id` injection, "please continue" prompt | ✅ Green — 13 tests |
| `src/application/usecases/process_strand.rs` tests | ProcessStrand execute flow, profile resolution, timeout handling, event dispatch | ✅ Green — 20+ tests |
| `src/application/ports.rs` tests | PortError, ExecutionContext, AgentOutput, AgentRunner trait | ✅ Green — 15+ tests |
| `src/adapters/pi_json.rs` tests | JSON-L parsing, session ID capture, token usage, response text | ✅ Green — 12+ tests |
| `tests/pipeline.rs` integration tests | Full event pipeline with mock runner | ✅ Green |
| `tests/session_resume.rs` integration tests | Session resume with real pi binary | ✅ Green |

## Test Gaps

- No test for "events expected but none emitted" detection after completion
- No `KnotEventsMissing` LoomEvent variant
- No follow-up session re-entry after natural completion (only retry-after-failure exists)
- No test for follow-up prompt parsing and event dispatch
- No test for `event: None` explicitly passing enforcement
- No test for no-consumer-knots skipping enforcement entirely
- No integration test for the full enforcement flow

## Phases

### Phase 0: Domain — `KnotEventsMissing` LoomEvent Variant

**Layer:** Domain (`src/domain/events.rs`)

Add a new `LoomEvent` variant to record when an agent was instructed to emit events but failed to do so:

```rust
/// A knot completed successfully but was instructed to emit events
/// and produced none in its response.
KnotEventsMissing {
    loom_id: LoomId,
    knot_id: KnotId,
    strand_path: StrandPath,
    /// Description of what events were expected.
    expected_events: Vec<String>,
    timestamp: String,
},
```

**Tests (domain unit):**
- `knot_events_missing_event_serialisation()` — serialises/deserialises correctly
- `knot_events_missing_event_fields()` — expected_events vec is preserved

**Existing tests to update:**
- `LoomEvent` match exhaustiveness in any existing match expressions (process_strand.rs, query handlers)

### Phase 1: Domain — Event Enforcement Detection Helper

**Layer:** Domain (`src/domain/tieoff_parser.rs`)

Add a helper function that determines whether event enforcement should trigger:

```rust
/// Determine if the tie-off content contains any agent events.
///
/// Returns `true` if zero event blocks were found (not even `event: None`).
/// Returns `false` if at least one event block (including `event: None`) was found.
///
/// This is the gate for event enforcement — if the agent was instructed
/// to emit events but produced zero event blocks, enforcement triggers.
pub fn has_no_events(content: &str) -> bool {
    // Checks if zero ```markdown blocks with --- delimiters exist
    // Does NOT require parsing events — just presence/absence of blocks
}
```

This is a lightweight check — it does not call `extract_agent_events()` (which fully parses events). It only checks for the presence of ```markdown blocks with `---` delimiters. If zero such blocks exist, the agent produced no events at all.

**Tests (domain unit):**
- `has_no_events_empty_input_returns_true()` — empty string has no events
- `has_no_events_normal_body_returns_true()` — body text with no markdown blocks returns true
- `has_no_events_with_event_block_returns_false()` — tie-off containing ```markdown with event returns false
- `has_no_events_with_event_none_returns_false()` — `event: None` block returns false (valid outcome)
- `has_no_events_with_multiple_events_returns_false()` — multiple event blocks returns false
- `has_no_events_yaml_block_ignored()` — ```yaml block is not counted as event block

### Phase 2: Application — Follow-Up Re-Entry in Session Resume Module

**Layer:** Application (`src/application/session_resume.rs`)

Add a new function for event enforcement re-entry. This is distinct from the retry loop — it runs after successful completion, not after failure:

```rust
/// Attempt to re-enter the session to request missing events.
///
/// Called after successful strand processing when the agent was instructed
/// to emit events but produced none. Re-enters the Pi session with a
/// follow-up prompt reminding the agent to provide event blocks.
///
/// Returns the agent's response text, which the caller parses for events.
/// Returns `Err` if the session cannot be re-entered (e.g. no session ID,
/// runner error).
pub fn inject_event_request(
    agent_runner: &dyn AgentRunner,
    loom_log: &dyn LoomLogPort,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_path: &StrandPath,
    session_id: &Option<String>,
    agent_config: AgentConfig,
    expected_events: Vec<String>,
    profile_prompt: String,
    event_type: String,
    knot_name: Option<String>,
    profile_timeout: Option<Duration>,
) -> Result<String, PortError>;
```

The follow-up prompt is constructed from `expected_events` and tells the agent:
- It was instructed to emit events: `<list of event IDs>`
- No event blocks were found in its response
- It must include at least one event block or `event: None`

If `session_id` is `None` (stdio adapter), returns `Err(PortError::EventEnforcementSkipped)` — the caller handles this gracefully (logs only, no re-entry).

**Tests (unit with mock runner):**
- `inject_event_request_success_with_events()` — mock returns event blocks → Ok with response text
- `inject_event_request_no_session_id_returns_err()` — session_id None → Err
- `inject_event_request_runner_error_propagates()` — mock returns PortError → Err propagated
- `inject_event_request_prompt_contains_expected_events()` — follow-up prompt lists the expected event IDs
- `inject_event_request_prompt_includes_event_none_option()` — follow-up prompt mentions `event: None` as an option
- `inject_event_request_uses_session_id_from_first_invocation()` — `--session-id` is in extra_args

### Phase 3: Application — Wire Into ProcessStrand

**Layer:** Application (`src/application/usecases/process_strand.rs`)

Add the enforcement check in the `KnotCompleted` success path of `execute()`, after tie-off write and event dispatch:

```
1. Knot completes → tie-off written → events parsed and dispatched
2. NEW: If listener_context was non-empty AND tie-off has zero event blocks:
   a. Log KnotEventsMissing to loom-log
   b. If session_id available: call inject_event_request()
   c. Parse follow-up response for events
   d. Dispatch any found events
   e. If still no events: log second KnotEventsMissing
3. Continue with normal completion (git commit, StrandProcessed, etc.)
```

Key design points:
- Enforcement runs **only** when `listener_context` was non-empty (the agent was actually instructed to emit events)
- Enforcement is **best-effort** — failures (no session ID, runner error) are logged but do not fail the strand
- Maximum **one** follow-up attempt — if the second response also has no events, stop
- The follow-up response body is **not** appended to the tie-off — only extracted events are dispatched
- If follow-up produces events, they are dispatched via the existing `dispatch_agent_events()` path

**Tests (unit with mock runner):**
- `process_strand_enforcement_no_consumers_skipped()` — no listener context → no enforcement run
- `process_strand_enforcement_events_present_skipped()` — events emitted → no enforcement
- `process_strand_enforcement_event_none_skipped()` — `event: None` emitted → no enforcement
- `process_strand_enforcement_missing_events_logs_and_retries()` — no events → KnotEventsMissing logged, follow-up attempted
- `process_strand_enforcement_followup_produces_events_dispatched()` — follow-up produces events → dispatched to consumers
- `process_strand_enforcement_followup_still_missing_logs_twice()` — second attempt also empty → second KnotEventsMissing logged, no further retry
- `process_strand_enforcement_no_session_id_log_only()` — stdio adapter → KnotEventsMissing logged, no re-entry
- Regression: all existing ProcessStrand tests still pass

### Phase 4: Integration Tests and Verification

**Layer:** Integration tests (`tests/`)

- Integration test: `test_event_enforcement_with_real_pi()` — knot with event consumers, agent produces no events → loom-log shows `KnotEventsMissing` → follow-up re-enters session → events dispatched (mock consumer)
- Integration test: `test_event_enforcement_stdio_no_reentry()` — `pi-stdio` adapter → `KnotEventsMissing` logged, no follow-up (no session ID)
- Integration test: `test_event_enforcement_event_none_passes()` — agent emits `event: None` → no enforcement triggered
- Integration test: `test_event_enforcement_no_consumers()` — knot with no consumers → no enforcement logic runs
- Regression: all existing tests pass
- `cargo clippy` clean

## Implementation Status: ✅ Complete (2026-07-14)

## Notes

- **Separation from session resume:** Event enforcement is a **post-success** operation. Session resume is a **post-failure** retry. They share the same re-entry mechanism (`--session-id`) but have different semantics. Enforcement does not use the retry loop — it is a single call.
- **Follow-up response handling:** The follow-up response body is **not** appended to the tie-off. Only the extracted events are dispatched. This keeps the tie-off clean — the enforcement re-entry is an implementation detail.
- **Performance impact:** Enforcement adds one extra Pi invocation per strand in the failure case (events expected but not emitted). In the success case (events emitted or no consumers), there is zero overhead.
- **`pi-stdio` graceful degradation:** Without session ID capture, enforcement can only log the failure — it cannot re-enter the session. This is documented as a known limitation. The user sees the `KnotEventsMissing` entry and can `touch` the strand to reprocess.
- **The `expected_events` list** comes from `build_listener_context()` output parsing. Since we already build the listener context before execution, we can extract the event IDs from the consumer knots' subscriptions. Pass the event IDs alongside the enforcement check.
- **Domain glossary update:** Add `Event Enforcement` and `KnotEventsMissing` terms.
