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
1. Consumer knots declare `listens-for` entries in their frontmatter, each specifying:
   - `target-knot` — which knot may emit the event
   - `event-id` — unique identifier for the event
   - `event-description` — when the event should be emitted and what data it should contain
2. Before a target knot runs, Knot collects all `listens-for` entries that target it and injects them at the **beginning** of its prompt with instructions to emit a structured event object in its tie-off if the event occurred
3. The target knot writes structured events to its tie-off using the format Knot told it about
4. Knot parses tie-off events, matches by `event-id` + `target-knot`, and creates an event file in each matching consumer's loom tie-off directory under a `{event-id}/` subdirectory
5. The consumer knot fires because its `strand-dir` watches its loom's tie-off directory (event subdirectories included)

Producers (target knots) have no declarations of their own — they are fully decoupled. The subscriber defines what event it needs, from whom, and under what conditions. Knot bridges the gap by injecting event instructions into the target knot's prompt.

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
| **Deduplication** | None. Re-running a producer that writes the same file re-triggers all consumers. | None — consumer knot must be idempotent for re-runs. |
| **Producer context** | Producer has no knowledge of which knots are watching its directory. | Knot injects event instructions at the start of the target knot's prompt — each event has an ID, description of when to emit it, and the exact format to use in the tie-off. Target knot has no `publishes` declaration. |
| **Workspace cleanliness** | Communication files live in tie-off directories — already the derived-state namespace. Visible in the rig but excluded from sharing packages. | Event files live in consumer's tie-off directory — derived-state namespace. One copy per consumer, enabling selective replay. |
| **Idempotency burden** | On the consumer: must detect whether it already processed the message file. | On the consumer: must detect whether it already processed the event file. |
| **Operational complexity** | Simple. Standard filesystem watches, no new Knot code needed. | Requires Knot runtime changes: intent parsing, event extraction from tie-offs, dispatch logic. |
| **Debugging** | Easy: inspect the typed subdirectory. Directory name is the event type, files are persistent artifacts. | Traceable via tie-off entries with `source:` and `original_strand:` metadata. Dispatch log in `rig/events/dispatched.jsonl`. |
| **When to use** | Immediate need for a2a comms; small number of fixed routes; prototype or simple workflow. | Growing rig with multiple producer-consumer pairs; dynamic consumer discovery; events that need conditional dispatch. |

### Migration Path

The static routing pattern is a valid current solution. It can be used now to unblock workflows that need a2a communication (e.g. the review → refactor-planning flow). Once intent-based routing ships:

1. Remove `strand-dir` paths that point at other knots' tie-off directories.
2. Add `listens-for` declarations to consumer knot frontmatter.
3. Add structured event entries to producer tie-offs.
4. Remove static event subdirectories from tie-off directories (they become redundant).

No breaking changes — intent-based routing is backward compatible with static routing during transition.

## Implementation Status: ✅ Complete (2026-07-09)

## Notes
- Phases 0-6 implemented: domain model (Intent, AgentEvent, EventMetadata), tie-off parser, intent matching, event dispatcher, context injection, processing pipeline integration, and observability
- Full test suite passes (669 tests, 0 failures) in ~25s wall clock
- Version bumped to 0.23.0
- Skills updated: knot-create (listens-for documentation), knot-update (changelog), knot-design (compatibility)

## Existing Tests
| Test Class | What it covers | Status |
|------------|---------------|--------|
| `tie_off_parser` | Parses tie-off sections with header/timestamp/body | ✅ Green — current format only |
| `knot_file` | Parses knot frontmatter (name, profile-ref, strand-dir, git-versioned) | ✅ Green — no listens-for support |
| `events` | Domain event types (StrandEvent, LoomEvent, ConfigEvent) | ✅ Green — no AgentEvent type yet |
| `usecases` | In-memory use case tests with mock ports | ✅ Green — no event dispatch logic |

## Test Gaps
- No tests for parsing `listens-for` YAML list (`target-knot`, `event-id`, `event-description`) in knot files
- No tests for detecting structured event entries in tie-off content
- No tests for matching events by `event-id` + `target-knot` against consumer intents
- No tests for prompt context injection (grouping by target)
- No tests for event file creation in consumer tie-off directory
- No integration test for the full target → event → consumer flow

## Phases

### Phase 0: Domain Model — Extend KnotFile and TieOff entities
- [ ] Add `listens_for: Vec<Intent>` to `KnotFile`
- [ ] Define `Intent` struct:
  - `target_knot: String` — which knot may emit this event
  - `event_id: String` — unique event identifier (e.g. `PlanCreated`)
  - `event_description: String` — when the event fires and what data it contains
- [ ] Define `AgentEvent` struct (event_id, target_knot, payload map, emitted-at, source strand)
- [ ] Add `agent_events: Vec<AgentEvent>` to `TieOff` entity
- [ ] Update `KnotFile::parse()` to accept `listens-for` as a YAML list of `{target-knot, event-id, event-description}` objects (unknown keys still warn, not error)
- [ ] Update `TieOff` serialization to include agent events
- [ ] **Tests**: Unit tests for parsing new frontmatter fields, serialization round-trips

### Phase 1: Tie-Off Parser — Detect structured events
- [ ] Add `extract_agent_events(content: &str) -> Vec<AgentEvent>` to `tieoff_parser`
- [ ] Define structured event format in tie-off entries:
  ```markdown
  [2026-06-25T10:00:00Z] Plan PLAN-001 created.
    event: PlanCreated
    target-knot: plan-creator
    plan: PLAN-001
    description: "Implementation plan for knot event routing"
    scope: "Add intent-based routing for agent-to-agent communication"
  ```
- [ ] Parse YAML-like key-value pairs from tie-off body lines (indented under the timestamp)
- [ ] Detect `event:` key as the signal that this entry contains structured event data
- [ ] **Tests**: Unit tests for extracting events from tie-off content, handling malformed entries gracefully

### Phase 2: Intent Matching — Match events to consumer declarations
- [ ] Add `matches_intent(event: &AgentEvent, intent: &Intent) -> bool` function
- [ ] Match on `event_id` (required) — must match exactly
- [ ] Match on `target_knot` (required) — event must come from the knot specified in the intent
- [ ] **Tests**: Unit tests for various match/no-match scenarios

### Phase 3: Event Dispatch — Create event files in consumer's loom tie-off directory
- [x] Add `EventDispatcherPort` port trait in `application/ports.rs`
  - `dispatch(event: &AgentEvent, consumer_knot: &Knot, consumer_loom_id: &LoomId, rig_dir: &Path) -> Result<PathBuf, PortError>`
- [x] Implement `FileSystemEventDispatcher` in `adapters/outbound/event_dispatcher.rs`
  - Creates event file at `rig/tie-offs/{consumer-loom-id}/{event-id}/event-{timestamp}.md`
  - Content: YAML frontmatter with event payload + markdown body with context
  - If multiple consumers listen for the same event, each loom gets its own copy
  - The consumer knot's `strand-dir` is set to the `{event-id}/` subdirectory (e.g. `../../tie-offs/review-loom/PlanCreated/`), so the filesystem watch fires the consumer when a new event file appears
- [x] Add `MockEventDispatcher` in `test_fixtures.rs` for unit tests
- [x] **Tests**: Event file creation, fan-out (two consumers in different looms), same loom different event-ids, empty payload, filename safety

### Phase 4: Context Injection — Inform target knot of listening consumers
- [x] Add `build_listener_context(knot: &Knot, all_knots: &[Knot]) -> String` function
- [x] Before a knot runs, scan all knots' `listens-for` entries and collect those where `target-knot` matches this knot's name
- [x] Inject at the **beginning** of the target knot's prompt:
  > Before undertaking your task, note that other knots are listening for events you may emit.
  > If an event occurs during your work, include an explicit event object in your final response using the format shown.
  >
  > Events you may emit:
  > - `PlanCreated` — The event is emitted if a plan is created for the first time, with description of the plan and its scope.
  >   Emit in your tie-off:
  >   ```
  >   event: PlanCreated
  >   target-knot: plan-creator
  >   plan: <plan-id>
  >   description: <description>
  >   scope: <scope>
  >   ```
- [x] One block per `event-id`; if multiple consumers listen for the same event from the same knot, they are merged (one event block, not duplicated)
- [x] **Tests**: Unit tests for context generation, formatting (8 tests)
- [x] Add `listens_for: Vec<Intent>` to `Knot` entity (propagated from `KnotFile`)

### Phase 5: Integration — Wire into processing pipeline
- [ ] After a knot produces a tie-off, invoke the event dispatcher
- [ ] Parse the tie-off for agent events
- [ ] For each event, find all consumer knots with matching intents
- [ ] Create event files in each consumer's loom tie-off directory under `{event-id}/` subdirectory
- [ ] Log event dispatch to loom-log (new `LoomEvent` variant or reuse existing)
- [ ] **Tests**: Integration test with mock ports covering full flow: target runs → event detected → consumer event file created → consumer fires

### Phase 6: Observability — Structured tie-off entries for events
- [ ] Ensure synthetic event strands produce tie-off entries that include:
  - `event:` key in the structured metadata
  - `source:` field (which knot emitted the original event)
  - `original_strand:` field (which strand triggered the producer)
- [ ] This enables counting a2a messages from tie-offs alone
- [ ] **Tests**: Verify tie-off entries for event-triggered consumer runs contain structured metadata

## Notes

### Event Directory Layout
Event files are created at `rig/tie-offs/{consumer-loom-id}/{event-id}/event-{timestamp}.md` — scoped per loom, not per knot. The consumer's `strand-dir` is set to the `{event-id}/` subdirectory, so the filesystem watch fires the consumer when a new event file appears. The watcher must exclude tie-off log files (`*-tie-off.md`) from being processed as strands.

This enables selective replay: touching or modifying a file in one loom's event subdirectory re-triggers only that loom's consumers, not other looms' consumers of the same event.

### Benefit: Intent Awareness Enables Tie-Off Monitoring

Because consumers declare their `listens-for` intents with explicit `event-id` and `target-knot`, Knot knows exactly which events should appear in which tie-offs. This enables a monitoring loop:

1. After a target knot produces a tie-off, Knot checks whether the structured event metadata was populated for any `event-id` that matching consumers are listening for.
2. If the agent was instructed to emit an event but omitted the `event:` block in the tie-off, Knot can detect this gap from the intent declarations alone.
3. Knot can then inject an additional session-scoped message into the agent's next session, requesting it populate the missing event information.

This is a form of lightweight agent supervision — the consumer's `listens-for` declaration serves not just as a routing contract but also as a completeness check. It reduces the burden on the target knot to self-audit and ensures inter-agent events are reliably structured.

### Backward Compatibility
- Knots without `listens-for` fields behave exactly as before (no event subscription)
- Existing tie-off entries without structured event data are parsed normally (no events extracted)
- The tie-off parser gracefully skips malformed event entries

### What This Does NOT Cover
- Cross-rig event routing (events stay within a single rig)
- Event expiration or TTL (events persist until manually cleaned)
- Priority or ordering of event delivery (FIFO by creation time)
- Retry on dispatch failure (failed dispatch is logged, not retried)

### Bugfix: Event dispatch directories not watched (2026-07-10)

When a knot has `listens_for` configured, event files dispatched to `rig/tie-offs/{loom-id}/{event-id}/` were never seen by the notify file watcher, so consumer knots were never triggered by dispatched events. Only `strand_dir` was watched — the event dispatch directories were created by `FileSystemEventDispatcher` but no watcher was ever registered on them.

Fixed by adding `ensure_event_watches()` which creates and watches each event dispatch directory during knot registration (`DiscoverLooms`, `RegisterLoom`, and all `ConfigEventHandler` paths). Watchers are (re-)started on `KnotModified` so config updates are picked up dynamically. Deduplicates event-IDs so shared dispatch directories are only registered once.
