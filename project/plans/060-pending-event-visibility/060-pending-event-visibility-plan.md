# Plan: Pending Event Visibility for Agent Producers

## Implementation Status: ✅ Complete (2026-07-15)

## Notes
- All 5 phases implemented (0–4)
- 668 tests pass (650 pre-existing + 18 new)
- Version bumped to 0.28.0
- Extracted ContextProvider pattern to design document

## Problem

When strand event dependencies are injected into a producer knot's initial prompt
(via `build_listener_context`), the knot has no awareness of whether it has
**previously raised events** of the required types that are still pending
downstream consumer invocation. This causes the producer to emit duplicate events
on re-processing (e.g., after session resume, retry, or re-processing the same
strand).

The producer is told "you may emit `PlanCreated`" but not "you already emitted a
`PlanCreated` with description 'X' that is still in the dispatch directory
waiting to trigger its consumer."

Currently, context injection is handled by a single function
`build_listener_context()` in `domain/events.rs` that only produces static
instructions. There is no mechanism for dynamic, state-aware context providers.

## Target

1. **Context providers become first-class citizens** — a `ContextProvider` trait
   encapsulates the concerns of building dynamic prompt context. The existing
   agent events context is its first implementation.

2. **Producers see pending events** — before a producer knot is invoked, the
   system scans the event dispatch directory
   (`rig/tie-offs/{loom-id}/{event-id}/`) for any event files that have been
   dispatched but are still present. These "pending events" are injected into the
   prompt alongside the event emission instructions, giving the agent visibility
   to decide whether a new event would be duplicative.

3. **Firmed-up event structure** — the agent event frontmatter format is
   documented and enforced with three required fields: `event`, `description`,
   `timestamp`. The body remains freeform narrative. The agent may not edit
   dispatched events, so if adjustment is needed it must emit a new event
   (discouraged unless critical).

## Existing Tests

| Test Class / File | What it covers | Status |
|---|---|---|
| `domain/events.rs` — `build_listener_context` tests | Context generation for single/multiple consumers, event dedup, loom-level subscriptions, `event: None` format, generic fallback | ✅ Green |
| `domain/tieoff_parser.rs` — `extract_agent_events` tests | Parsing ```markdown blocks, frontmatter extraction, multi-event, `event: None` skip, body extraction | ✅ Green |
| `domain/tieoff_parser.rs` — `has_no_events` tests | Event enforcement gate — detects missing event blocks | ✅ Green |
| `adapters/outbound/event_dispatcher.rs` tests | Dispatch creates files with correct path, frontmatter structure, body handling, fan-out | ✅ Green |
| `application/usecases/process_strand.rs` — execution tests | Full pipeline including listener context injection, event dispatch, event enforcement | ✅ Green |

## Test Gaps

- No tests for dynamic context building (context that depends on filesystem state).
- No tests for scanning event dispatch directories for pending events.
- No tests for the `ContextProvider` abstraction.
- No tests verifying the `timestamp` field in event format.
- No integration test verifying that pending events appear in the producer's prompt.

## Phases

### Phase 0: ContextProvider trait and domain structure

Introduce `ContextProvider` as a domain trait that encapsulates the building of
dynamic prompt context segments. This is a clean concern — the domain defines
the interface, the application layer provides the concrete implementation that
has access to the rig directory and filesystem.

**Domain** — `domain/events.rs`:

- Define `trait ContextProvider` with `fn build_context(&self, input: &BuildContext) -> String`.
- Define `BuildContext` struct carrying the data all providers need: the current
  `Knot`, `LoomId`, all `Knot`s, and `rig_dir`.
- This is a pure refactor of the *interface* — no behavioural change yet.

**Tests**: Domain tests verifying the trait compiles, `BuildContext` carries
correct fields, and an empty/no-op provider returns empty string.

### Phase 1: AgentEventsContextProvider implementation

Implement `AgentEventsContextProvider` that combines two concerns:

1. **Event emission instructions** — the existing `build_listener_context` logic
   (what events the knot may emit and the format).
2. **Pending event visibility** — scan the event dispatch directories for
   pending event files and format them as a "Pending Events" context section.

The pending event scan works by:
- For each event ID the knot is instructed to emit, scan
  `rig/tie-offs/{loom-id}/{event-id}/` for `.md` files.
- For each file found, extract the frontmatter fields: `event-id`,
  `description` (from the `description` payload field), `timestamp`, and the
  filename (which is `event-{timestamp}.md`).
- Format as a markdown section prepended to the listener context.

The pending events section format (injected into the prompt):

```markdown
## Pending Events

The following events have been emitted but may not yet have triggered their
consumers. Check these before deciding to emit a new event — if a pending event
covers the same outcome, you may not need to emit a duplicate.

- `PlanCreated` — "Implementation plan for feature X" (file: event-2026-07-14T10-00-00Z.md, dispatched: 2026-07-14T10:00:00Z)
```

When no pending events exist, the section is omitted entirely (no empty noise).

**Application** — new file `application/usecases/context_providers.rs`:

- `AgentEventsContextProvider` struct (stateless — uses `BuildContext` for all
  data).
- `build_context()` method implementing the full logic.
- Helper: `scan_pending_events()` — reads dispatch directories and extracts
  event metadata from frontmatter.

**Tests**:
- `AgentEventsContextProvider` with no listeners returns empty string.
- `AgentEventsContextProvider` with listeners but no pending events returns only
  the emission instructions (existing `build_listener_context` output).
- `AgentEventsContextProvider` with pending events includes both emission
  instructions and pending events section.
- Pending events section correctly extracts `event-id`, `description`,
  `timestamp` from dispatched event file frontmatter.
- Multiple event types, each with their own pending events.
- Missing dispatch directory (no events emitted yet) — graceful empty.
- Event file with no description in frontmatter — shown without description.

### Phase 2: Refactor build_listener_context to use ContextProvider

Replace the raw `build_listener_context(knot, loom_id, all_knots)` call in
`process_strand.rs` with the `AgentEventsContextProvider` via the
`ContextProvider` trait.

This is a **pure refactor** — no behavioural change. The output of the provider
must match the existing `build_listener_context` output (minus the pending events
section, which is additive).

**Application** — `application/usecases/process_strand.rs`:

- Replace `build_listener_context()` call with
  `AgentEventsContextProvider.build_context(&build_context)`.
- `BuildContext` is assembled from the same data already available in
  `ProcessStrand::execute()`.

**Domain** — `domain/events.rs`:

- `build_listener_context` becomes an internal helper used by
  `AgentEventsContextProvider`. Its signature may stay the same (it doesn't need
  `rig_dir`).
- The public API is the `ContextProvider` trait.

**Tests**:
- Existing `build_listener_context` tests remain passing (the function still
  exists and works the same).
- `ProcessStrand` execution tests verify the prompt contains both emission
  instructions and (when applicable) pending events.
- Refactor does not change the event enforcement flow.

### Phase 3: Firm up event format — timestamp field and required fields

Update the prompt template (inside `build_listener_context`, used by the
provider) to:

1. Make `timestamp` an explicit required field in the event format example.
2. Document that `event` and `description` are required fields.
3. Add guidance: "You may not edit dispatched events. If you need to adjust an
   event, emit a new event with additional context — but only if critical, as
   this results in an additional event to be processed later."

**Event format in the prompt** (updated):

```
---
event: <EventId>
description: <short summary of what happened>
timestamp: <ISO 8601 timestamp>
<additional fields as relevant>
---

Freeform narrative context about the event.
```

**Domain** — `domain/events.rs`:

- Update `build_listener_context` prompt template to include `timestamp` and
  the "do not edit" guidance.

**Adapter** — `adapters/outbound/event_dispatcher.rs`:

- `build_event_file_content` already writes `timestamp` in the dispatched file.
  Verify consistency. If the agent also provides a timestamp in its event block,
  prefer the agent's timestamp (it's more semantically meaningful — when the
  event occurred from the agent's perspective). The dispatch system's timestamp
  remains as the system-level record.

**Tests**:
- Prompt template includes `timestamp:` field in format example.
- Prompt template includes "do not edit" guidance text.
- `build_event_file_content` in dispatcher still produces correct frontmatter
  (regression test).

### Phase 4: Integration and end-to-end verification

Wire the complete flow end-to-end and verify with integration-level tests:

1. Producer knot emits `PlanCreated` → event file created in dispatch directory.
2. Producer knot re-enters (same strand, e.g., Modified event) → pending event
   from step 1 is visible in its prompt.
3. Producer knot can see the pending event and decide not to duplicate.

This phase uses the existing `ProcessStrand` test infrastructure (mock runner,
mock tie-off sink, temp directories) to simulate the flow.

**Tests**:
- Integration test: producer emits event, then re-enters, prompt contains
  pending event reference.
- Integration test: producer with multiple event types sees only relevant
  pending events.
- Regression: existing event enforcement flow still works (missing events
  trigger follow-up).

## Notes

- The `ContextProvider` trait is deliberately minimal (single method) — it's an
  abstraction for a currently single-use case. It can expand naturally when
  additional context types are needed (e.g., strand history, previous tie-offs).
- The pending event scan reads files from the dispatch directory. These files
  are NOT deleted after consumer processing — they persist as the event record.
  Therefore, the scan shows ALL dispatched events, not just "unprocessed" ones.
  This is intentional: the agent can use the filename (which contains a
  timestamp) and description to assess recency and relevance.
- The `description` field in the pending event section comes from the
  `description` payload key in the dispatched event file's frontmatter. This is
  set by the agent when it emits the event and preserved by the dispatcher.
- No changes to the `AgentEvent` domain struct are required — the `payload`
  HashMap already carries `description`, and the `event_id` field carries the
  event type. The `timestamp` is added by the dispatch system in the file
  frontmatter.
