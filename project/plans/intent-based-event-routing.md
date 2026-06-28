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

## Current Capability: Static Routing via Tie-Off Directories

Knot already supports a static routing pattern using tie-off directories as inter-agent message channels. This is not a first-class feature — it is an architectural property of how strand watches, knot discovery, and tie-off layout interact.

### Why Tie-Offs, Not Looms

The PRDs define a clean separation:
- **Rig directories** (`rig/`) hold workflow definitions — loom directories with knot `.md` files
- **Tie-off directories** (`rig/tie-offs/`) hold derived state — outputs that regenerate on any machine

Placing event files inside a loom directory mixes runtime output with workflow definitions. Tie-off directories are the correct namespace for static events — they are already output space, already organised by loom and knot, and already excluded from sharing packages (derived state).

### How It Works

The tie-off directory for a knot lives at `rig/tie-offs/{loom-id}/{knot-name}/`. It contains the knot's append-only tie-off file (`{knot-name}-tie-off.md`). **Typed subdirectories** within the tie-off directory carry static event files — the subdirectory name declares the event type. A consumer knot in any loom can point its `strand-dir` at a specific event subdirectory:

```
rig/tie-offs/review-loom/implementation-review/
├── implementation-review-tie-off.md    ← normal tie-off (append-only log)
├── reviews/                            ← event type: quality reviews
│   ├── 016-quality-review.md           ← static event
│   └── 017-quality-review.md           ← next event
└── findings/                           ← another event type (if needed)
    └── ...

planning-loom/implementation-planner.md
    strand-dir: "../../tie-offs/review-loom/implementation-review/reviews"
```

When the `implementation-review` knot writes `016-quality-review.md` into the `reviews/` subdirectory, the `implementation-planner` knot fires because it watches that specific subdirectory. The subdirectory name (`reviews`) is the event type — anyone reading the consumer's `strand-dir` can see exactly what events it consumes.

### Safety Guarantees

1. **Tie-off files never fire config events.** Config event mapping (`map_rig_event`, `map_loom_event`) only watches the rig and loom directories — `tie-offs/` is not a `-loom` directory and is never scanned for knot definitions.
2. **Knot discovery skips non-`.md` files and subdirectories.** Even if someone accidentally places a `.md` file in a tie-off directory, it cannot be discovered as a knot because tie-off directories are not inside a `-loom` directory.
3. **Strand watches are target-specific.** Only knots whose `strand-dir` points at the tie-off directory receive events. Other knots are unaffected.
4. **Loom watches are non-recursive.** `WatchType::Loom` uses `RecursiveMode::NonRecursive`, so files created deep inside tie-off directories don't leak into loom config processing.

This means a producer knot can write files into its own tie-off directory, and any consumer knot watching that directory fires — without polluting the project workspace or confusing the knot discovery pipeline.

### Example: Static Review → Planning Flow

```
review-loom/implementation-review.md
    strand-dir: "project/progress"        # reads progress reports
    tie-offs at: rig/tie-offs/review-loom/implementation-review/

planning-loom/implementation-planner.md
    strand-dir: "../../tie-offs/review-loom/implementation-review/reviews"
    # subscribes to 'reviews' event type, creates refactor plans

rig/tie-offs/review-loom/implementation-review/
├── implementation-review-tie-off.md    # append-only log
└── reviews/                            # event type directory
    └── 016-quality-review.md           # static event
```

```
implementer → project/progress/016-*.md
  ├── triggers implementation-review (reads project/progress/)
  │     └── writes tie-offs/.../implementation-review/reviews/016-quality-review.md
  │          └── triggers implementation-planner (reads reviews/ subdirectory)
  │               └── creates project/plans/017-refactor-*.md
  └── triggers progress-planner (reads project/progress/)
        └── updates plan status, chains to next plan
```

### Comparison: Static vs. Intent-Based Routing

| Aspect | Static Routing (current) | Intent-Based Routing (this plan) |
|--------|--------------------------|----------------------------------|
| **Routing mechanism** | Producer writes a file to a typed subdirectory in its tie-off directory; consumer's `strand-dir` points at that subdirectory | Agent emits structured event; Knot matches `listens-for` intents |
| **Who decides an event fires** | The agent writes a file — event fires unconditionally | The agent declares the event happened; Knot decides which consumers match |
| **Routing flexibility** | Fixed at rig configuration time. Adding a new consumer requires changing its `strand-dir` or creating a new directory. | Dynamic at runtime. New consumers declare intent in frontmatter; no directory changes needed. |
| **Fan-out (one producer → many consumers)** | Subdirectories provide natural event-type filtering: `strand-dir: "../../tie-offs/.../reviews"` subscribes only to reviews, not other event types. But the subdirectory must exist before the consumer can strand from it. | Native: multiple consumers declare different intents on the same event type. Knot dispatches only matching consumers. |
| **Event payload** | Entire file content. Consumer must parse the full file to find relevant data. | Structured key-value pairs in tie-off. Consumer receives only the event payload it needs. |
| **Deduplication** | None. Re-running a producer that writes the same file re-triggers all consumers. | Built-in: event ID = hash of `(source_knot, event_type, payload_hash)`. Dispatcher skips already-dispatched pairs. |
| **Producer context** | Producer has no knowledge of which knots are watching its directory. | Producer prompt is injected with listener context — can tailor event detail to what consumers need. |
| **Workspace cleanliness** | Communication files live in tie-off directories — already the derived-state namespace. Visible in the rig but excluded from sharing packages. | Synthetic strands created in consumer's loom-box. No persistent comms files needed if consumers process immediately. |
| **Idempotency burden** | On the consumer: must detect whether it already processed the message file. | Shared: dispatcher deduplicates at delivery; consumer still idempotent for re-runs. |
| **Operational complexity** | Simple. Standard filesystem watches, no new Knot code needed. | Requires Knot runtime changes: intent parsing, event extraction from tie-offs, dispatch logic. |
| **Debugging** | Easy: inspect the typed subdirectory. Directory name is the event type, files are persistent artifacts. | Traceable via tie-off entries with `source:` and `original_strand:` metadata. Dispatch log in `rig/events/dispatched.jsonl`. |
| **When to use** | Immediate need for a2a comms; small number of fixed routes; prototype or simple workflow. | Growing rig with multiple producer-consumer pairs; dynamic consumer discovery; events that need conditional dispatch. |

### Migration Path

The static routing pattern is a valid current solution. It can be used now to unblock workflows that need a2a communication (e.g. the review → refactor-planning flow). Once intent-based routing ships:

1. Remove `strand-dir` paths that point at other knots' tie-off directories.
2. Add `listens-for` declarations to consumer knot frontmatter.
3. Add `publishes` declarations and structured event entries to producer tie-offs.
4. Remove static event subdirectories from tie-off directories (they become redundant).

No breaking changes — intent-based routing is backward compatible with static routing during transition.

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
