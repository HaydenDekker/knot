# Plan: Intent-Based Event Routing

## Related PRD

This plan contributes to [Agent-to-Agent Event Routing](../prds/prd-agent-event-routing.md).

Implements the core mechanism for knots to declare event intents, emit structured events via tie-offs, and have Knot automatically dispatch those events to interested consumer knots — without polluting the project workspace or requiring agents to act as routers.

## Problem
Currently, Knot has no first-class primitive for agent-to-agent events. Strands are user-routed (files placed in a directory), and tie-offs are append-only logs. There is no way for a producing knot to signal a state change that other knots can react to without either:
- Polluting the project workspace with comms files
- Requiring agents to know about and route to other agents
- Relying on fragile filename conventions or regex filters

## Target
Knot will support **intent-based event routing**:
1. Consumer knots declare `listens-for` intents in their frontmatter
2. Producer knots declare `publishes` capabilities in their frontmatter
3. When a producer writes a structured event to its tie-off, Knot parses it, matches against declared intents, and creates a synthetic event strand in the consumer's loom-box
4. The consumer knot fires as if a real strand was placed in its `strand-dir`
5. The producer's prompt is injected with context about which knots are listening, so it can add relevant detail to the event

## Implementation Status: ⬜ Draft

## Existing Tests
| Test Class | What it covers | Status |
|------------|---------------|--------|
| `tie_off_parser` | Parses tie-off sections with header/timestamp/body | ✅ Green — current format only |
| `knot_file` | Parses knot frontmatter (name, profile-ref, strand-dir, git-versioned) | ✅ Green — no listens-for/publishes support |
| `events` | Domain event types (StrandEvent, LoomEvent, ConfigEvent) | ✅ Green — no AgentEvent type yet |
| `usecases` | In-memory use case tests with mock ports | ✅ Green — no event dispatch logic |

## Test Gaps
- No tests for parsing `listens-for` / `publishes` YAML in knot files
- No tests for detecting structured event entries in tie-off content
- No tests for matching event payloads against declared intents
- No tests for synthetic event strand creation in loom-box
- No integration test for the full producer → event → consumer flow

## Phases

### Phase 0: Domain Model — Extend KnotFile and TieOff entities
- [ ] Add `listens_for: Vec<Intent>` and `publishes: Vec<EventCapability>` to `KnotFile`
- [ ] Define `Intent` struct (event type, optional from/to filters, optional required payload fields)
- [ ] Define `EventCapability` struct (event type, description of when emitted)
- [ ] Define `AgentEvent` struct (event type, payload map, emitted-by, emitted-at, source strand)
- [ ] Add `agent_events: Vec<AgentEvent>` to `TieOff` entity
- [ ] Update `KnotFile::parse()` to accept and validate new YAML keys (unknown keys still warn, not error)
- [ ] Update `TieOff` serialization to include agent events
- [ ] **Tests**: Unit tests for parsing new frontmatter fields, serialization round-trips

### Phase 1: Tie-Off Parser — Detect structured events
- [ ] Add `extract_agent_events(content: &str) -> Vec<AgentEvent>` to `tieoff_parser`
- [ ] Define structured event format in tie-off entries:
  ```markdown
  [2026-06-25T10:00:00Z] Plan PLAN-001 promoted to review.
    event: plan-status-change
    plan: PLAN-001
    from: drafted
    to: review
    triggered: plan-reviewer
  ```
- [ ] Parse YAML-like key-value pairs from tie-off body lines (indented under the timestamp)
- [ ] Detect `event:` key as the signal that this entry contains structured event data
- [ ] **Tests**: Unit tests for extracting events from tie-off content, handling malformed entries gracefully

### Phase 2: Intent Matching — Match events to consumer declarations
- [ ] Add `matches_intent(event: &AgentEvent, intent: &Intent) -> bool` function
- [ ] Match on `event_type` (required)
- [ ] Match on `from`/`to` filters when present in intent (optional)
- [ ] Match on required payload fields when present in intent (optional)
- [ ] **Tests**: Unit tests for various match/no-match scenarios

### Phase 3: Event Dispatch — Create synthetic event strands
- [ ] Add `EventDispatcher` port trait (or extend existing ports)
  - `dispatch(event: AgentEvent, consumer: &Knot) -> Result<TieOffPath, PortError>`
- [ ] Implement filesystem adapter: create event file in consumer's `strand-dir` (loom-box)
  - Filename: `{source-knot}-{event-type}-{timestamp}.md`
  - Content: YAML frontmatter with event payload + markdown body with context
- [ ] Implement deduplication: track processed event IDs per consumer (event ID = hash of event content + consumer ID)
- [ ] **Tests**: Unit tests for synthetic file creation, deduplication, filename generation

### Phase 4: Context Injection — Inform producer of listening knots
- [ ] Add `build_listener_context(knot: &Knot, all_knots: &[Knot]) -> String` function
- [ ] When a producer knot runs, prepend to its prompt:
  > Note: The following knots are listening for events you may emit:
  > - `plan-reviewer` — listens for `plan-status-change` (drafted → review/approved)
  > - `adr-planner` — listens for `plan-status-change` (any transition)
- [ ] Only inject context for knots that declare `publishes` matching the producer's capabilities (or inject all if producer has no `publishes` declared — conservative)
- [ ] **Tests**: Unit tests for context generation, filtering, formatting

### Phase 5: Integration — Wire into processing pipeline
- [ ] After a knot produces a tie-off, invoke the event dispatcher
- [ ] Parse the tie-off for agent events
- [ ] For each event, find all consumer knots with matching intents
- [ ] Create synthetic event strands in each consumer's loom-box
- [ ] Log event dispatch to loom-log (new `LoomEvent` variant or reuse existing)
- [ ] **Tests**: Integration test with mock ports covering full flow: producer runs → event detected → consumer strand created → consumer fires

### Phase 6: Observability — Structured tie-off entries for events
- [ ] Ensure synthetic event strands produce tie-off entries that include:
  - `event:` key in the structured metadata
  - `source:` field (which knot emitted the original event)
  - `original_strand:` field (which strand triggered the producer)
- [ ] This enables counting a2a messages from tie-offs alone
- [ ] **Tests**: Verify tie-off entries for event-triggered consumer runs contain structured metadata

## Notes

### Event ID and Deduplication Strategy
Each `AgentEvent` gets a deterministic ID: `{source_knot}:{event_type}:{payload_hash}`. The dispatcher tracks `(event_id, consumer_knot_id)` pairs in a simple on-disk file (`rig/events/dispatched.jsonl`). Before dispatching, the dispatcher checks if the pair already exists. This prevents duplicate firing when a producer re-runs without changes.

### Loom-Box Convention
Consumer knots that listen for events set `strand-dir: "loom-box/"` relative to their loom directory. The loom directory is owned by the loom, so the loom-box is namespaced per-loom. This avoids cross-loom coupling while keeping event files co-located with the consumer.

### Backward Compatibility
- Knots without `listens-for` or `publishes` fields behave exactly as before (no events, no injection)
- Existing tie-off entries without structured event data are parsed normally (no events extracted)
- The tie-off parser gracefully skips malformed event entries

### What This Does NOT Cover
- Cross-rig event routing (events stay within a single rig)
- Event expiration or TTL (events persist until manually cleaned)
- Priority or ordering of event delivery (FIFO by creation time)
- Retry on dispatch failure (failed dispatch is logged, not retried)
