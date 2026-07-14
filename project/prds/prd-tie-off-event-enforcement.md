# PRD: Tie-Off Event Enforcement

## Problem

When a knot is configured to emit agent events (via `event:` subscriptions from downstream consumer knots), the agent is instructed to include structured event blocks in its final response. However, agents frequently fail to include these event blocks — they produce a useful response body but omit the `event:` section entirely. This is a **silent failure**: the downstream consumer knots never receive the event files they depend on, breaking the agent-to-agent workflow, and the user has no visibility into what went wrong.

The user cannot tell whether the agent intentionally emitted `event: None` (correct — no events occurred) or simply forgot to emit any event block (incorrect — the agent was instructed to emit events but failed to do so). In both cases the tie-off looks normal, but the downstream workflow is stalled.

There is no automatic recovery mechanism either — if an agent fails to emit required events, the strand completes as "successful" and no retry is attempted. The user must manually inspect the loom-log or tie-off to discover the missing events.

## Goals

- [ ] When an agent is instructed to emit events (listener context was injected), but produces no event blocks in its response, Knot logs a `KnotEventsMissing` entry to the loom-log so the user can see the failure
- [ ] When events are missing, Knot automatically re-enters the Pi session (using `--session-id`) with a follow-up prompt instructing the agent to emit at least one event block or explicitly emit `event: None`
- [ ] If the follow-up produces events, they are dispatched to consumer knots normally and the strand completes successfully
- [ ] If the follow-up still produces no events, a second `KnotEventsMissing` is logged and processing ends (no infinite loop — max one retry)
- [ ] When an agent is **not** instructed to emit events (no listener context was injected), no event enforcement runs — the feature is invisible to normal knots
- [ ] When an agent emits `event: None` (explicit "no events" declaration), no enforcement runs — this is a valid, intentional outcome

## Non-Goals

- Changing the event format or parser — the existing markdown code block format is used as-is
- Adding user-configurable thresholds or limits — max one retry is the default and only mode
- Surface-level HTTP notifications — loom-log entries are the observability interface
- Mid-session injection — re-entry happens after the first invocation completes (uses the same `--session-id` mechanism as session resume)
- Token usage tracking for the enforcement re-entry — it is transparent to the user
- Support for knots that use the `pi-stdio` adapter without session ID capture — enforcement requires `pi-json` mode (session ID must be available). If no session ID is captured, enforcement is skipped and only the loom-log entry is written

## User Stories

### Story 1: Missing Event Detection and Logging

As a user, when an agent that was instructed to emit events fails to include any event block in its response, I want to see a `KnotEventsMissing` entry in the loom-log so I know the downstream workflow is stalled.

**Scenarios:**

1. Given a knot has consumer knots listening for its events (listener context was injected), when the agent completes but produces no event blocks in its response, then a `KnotEventsMissing` entry is appended to the loom-log with the knot ID, strand path, and a message describing the failure
2. Given a knot has no consumer knots listening for its events, when the agent completes with no events, then no `KnotEventsMissing` entry is logged — the feature does not activate
3. Given a knot has consumer knots listening, when the agent emits `event: None` (explicit no-events declaration), then no `KnotEventsMissing` entry is logged — `event: None` is a valid outcome

### Story 2: Automatic Follow-Up Prompt

As a user, when an agent fails to emit required events, I want Knot to automatically re-enter the session and remind the agent to provide events, so that the downstream workflow can continue without manual intervention.

**Scenarios:**

1. Given an agent was instructed to emit events but produced none, when Knot detects the missing events, then it re-enters the Pi session using `--session-id` with a prompt reminding the agent it must emit at least one event block or `event: None`
2. Given the follow-up prompt succeeds and the agent produces event blocks, when Knot parses the new response, then the events are dispatched to consumer knots normally and the strand completes
3. Given the follow-up prompt fails again (still no events), when Knot processes the second response, then a second `KnotEventsMissing` entry is logged and processing ends — no further retries are attempted
4. Given the knot was invoked with `pi-stdio` (no session ID captured), when events are missing, then the loom-log entry is written but no follow-up prompt is attempted — the feature gracefully degrades

## Success Criteria

- [ ] A knot that emits events for a consumer always dispatches events — no silent failures
- [ ] When events are missing, the loom-log contains a `KnotEventsMissing` entry that clearly identifies the knot, strand, and nature of the failure
- [ ] The follow-up prompt resolves the missing events in the common case (agent simply forgot the event block format)
- [ ] No infinite retry loop — maximum one follow-up attempt
- [ ] Knots without event listeners are unaffected — no extra processing, no log entries
- [ ] `event: None` is always accepted as a valid outcome — never triggers enforcement

## Dependencies & Constraints

- **Technical dependency:** Session ID must be captured for follow-up re-entry. This requires `pi-json` adapter mode (`--mode json`). The `pi-stdio` adapter does not capture session IDs — enforcement degrades to log-only in that case.
- **Technical dependency:** The `AgentInvocationMetadata.session_id` from the first invocation must be available to the enforcement logic.
- **Technical constraint:** The follow-up re-entry uses the same mechanism as session resume (`--session-id`). Pi must accept `--session-id` on a session that already completed naturally. This must be verified empirically.
- **Design decision:** Maximum one follow-up attempt. If the agent fails twice, the strand is not retried — the user must intervene. This prevents infinite loops while giving the agent one chance to correct.
- **Design decision:** `event: None` (explicit no-events) is never treated as a failure. The enforcement only triggers when **zero** event blocks are present in the tie-off — not even `event: None`. If the agent writes `event: None`, it has fulfilled its obligation.

## Implementation Status: 🔵 Open
