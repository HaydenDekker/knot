# PRD: Input Reconciliation — Initialise Queue With Unfinished Work

## Problem

Knot agents process work triggered by events or file changes, but the **trigger is not the input** — it's a notification. The real input is a set of documents (plans, PRDs, config files) whose *domain completion* is what matters. When a loom is offline, the agent times out, or the event stream is missed, the agent has no awareness that it has fallen behind.

A concrete example from a real rig: the validation loom's `completion-validator` receives `TestReady` events emitted by the planning loom when plans complete. These events are notifications. The actual input is `project/plans/` — the agent's job is to validate every completed plan. If the loom was stopped for three days and five plans completed in that time, the events sit unprocessed in a dispatch directory, but the agent has no mechanism to discover and catch up on them. When it restarts, it only processes the one event Knot happens to queue — the rest are silently missed.

This is not unique to validation. Every loom with mutable document inputs has the same problem:
- The documentation loom writes docs per completed plan. If its `ProgressReport` events are missed, docs are never written.
- The review loom reviews code per completed plan. If its events are missed, code is never reviewed.
- The acceptance loom authors specs per PRD. If file-change events are missed, specs are never created.

Knot needs **input reconciliation** so that on startup, every agent can determine what work remains unfinished and the queue is initialised accordingly — regardless of whether trigger events were consumed.

## Need

On startup, Knot must ensure the processing queue is initialised with all unfinished work for each knot. This means:

1. Each knot declares what "done" looks like — its **Definition of Done** — so completeness can be evaluated against the actual project state.
2. An agent reviews the project state against each knot's Definition of Done and identifies any incomplete work.
3. The queue is populated so that incomplete work is processed — either by creating synthetic events or by touching existing event files to trigger re-processing.

This replaces any need for a mechanical ledger (file manifest + timestamp diff) with an agent-driven evaluation that is:
- **Flexible** — the Definition of Done can express semantic conditions, not just "was file X seen?"
- **Self-healing** — if the user manually alters project state (e.g. marks a plan complete, edits a doc), the agent detects the change on next startup
- **Transparent** — the user can directly see and alter input events, rather than reasoning through a ledger's state

## Goals

- [ ] Knots can declare a **Definition of Done** — a statement of what "complete" means for the work this knot is responsible for
- [ ] On startup, an agent evaluates each knot's Definition of Done against the current project state
- [ ] The agent identifies any incomplete work and ensures it is queued for processing
- [ ] The mechanism works for both event-driven knots (where the event is a notification) and file-driven knots (where the strand is the actual document)
- [ ] The reconciliation works per-loom, even when multiple looms share the same input directory
- [ ] The user can manually alter input events or project documents and the agent detects the change on next startup
- [ ] No per-knot state file (ledger, manifest, etc.) needs to be maintained by the user

## Non-Goals

- Replacing or modifying the event system — reconciliation feeds the existing queue
- Cross-loom coordination — each loom reconciles independently
- Schema validation of input documents — the agent handles malformed content
- Continuous reconciliation during runtime — this is startup-only (a future PRD may cover periodic re-checks)
- Tracking non-markdown inputs — reconciliation evaluates `.md` files. Binary or other formats are out of scope.
- Replacing agent judgement with mechanical rules — the Definition of Done is evaluated by an agent, not a parser

## User Stories

### Story 1: Declare Definition of Done

As a rig author, I want to declare what "done" means for each knot so that Knot knows what completeness looks like.

**Scenarios:**

1. Given a validation knot processes completed plans, when I declare the Definition of Done as "every plan with status Complete has a corresponding validation tie-off", then Knot knows to check which completed plans lack validation
2. Given a documentation knot writes docs per completed plan, when I declare the Definition of Done as "every completed plan has a corresponding feature document in project/docs/", then Knot knows which plans need documentation
3. Given a knot's Definition of Done references project files (e.g. "every PRD has a BDD spec"), when Knot evaluates on startup, then it checks the actual project state — not a manifest or ledger

### Story 2: Startup Reconciliation

As a rig author, I want Knot to evaluate the Definition of Done on startup so that any incomplete work is automatically queued.

**Scenarios:**

1. Given 5 completed plans exist and 3 have been validated, when Knot starts, then the queue is initialised with the 2 unvalidated plans
2. Given the loom was offline for 3 days and 7 new completed plans were created, when Knot starts, then all 7 plans are queued for validation
3. Given all work is complete (every completed plan has been validated), when Knot starts, then no new events are queued
4. Given the user manually deleted a validation tie-off, when Knot starts, then the agent detects the gap and re-queues the plan for validation

### Story 3: Synthetic Event Creation

As a rig author, I want incomplete work to be represented as events in the queue so that the existing processing pipeline handles it naturally.

**Scenarios:**

1. Given a plan has status Complete but no validation tie-off exists, when Knot reconciles on startup, then a synthetic `TestReady` event is created in the dispatch directory
2. Given a PRD exists but no BDD spec has been authored, when the acceptance loom reconciles, then a synthetic event is created to trigger spec authoring
3. Given synthetic events are created, when the queue processes them, then the knot behaves identically to processing a real event — same tie-off output, same loom-log entries
4. Given the user manually creates an event file, when Knot reconciles, then the agent evaluates the actual project state (not just the presence of an event file) and may still queue work if the outcome is incomplete

### Story 4: User-Controlled State

As a rig author, I want to be able to manually alter input events or project documents and have Knot adapt on the next startup.

**Scenarios:**

1. Given I manually mark a plan as Complete in its frontmatter, when Knot starts, then the agent detects the new completed plan and queues it
2. Given I manually delete a tie-off file to force re-processing, when Knot starts, then the agent detects the missing output and re-queues the work
3. Given I manually create an event file for a specific plan, when Knot processes it and the output is correct, then the Definition of Done is satisfied for that plan
4. Given I manually edit a plan to change its status from Complete to Draft, when Knot starts, then the agent no longer considers it eligible for reconciliation (the condition is no longer met)

### Story 5: Per-Loom Independence

As a rig author, I want each loom to reconcile independently so that multiple looms watching the same input directory each determine their own completeness.

**Scenarios:**

1. Given both the validation loom and documentation loom watch `project/plans/`, when Knot starts, then each loom evaluates its own Definition of Done independently
2. Given the validation loom has processed plan 041 but the documentation loom has not, when both reconcile, then validation sees no gap but documentation queues plan 041
3. Given a loom's Definition of Done is updated, when Knot starts, then the new definition is used — no stale state persists from the old definition

## Success Criteria

- [ ] Each knot can declare a Definition of Done in its knot definition file
- [ ] On startup, Knot evaluates each knot's Definition of Done against the current project state
- [ ] Incomplete work is queued via synthetic events that trigger normal knot processing
- [ ] The mechanism works for both event-driven and file-driven knots
- [ ] Multiple looms sharing the same input directory each reconcile independently
- [ ] Manual changes to project documents or event files are detected on next startup
- [ ] No per-knot state file (ledger, manifest, etc.) is required — completeness is evaluated against the live project
- [ ] Existing knot behaviour is unchanged — reconciliation is additive, no modifications to event wiring or strand processing
- [ ] All existing tests pass — the change is backwards compatible

## Dependencies & Constraints

- **Technical dependency:** Knot must be able to invoke an agent to evaluate project state against the Definition of Done. This requires agent invocation infrastructure on startup.
- **Technical dependency:** The agent must have read access to all project documents and tie-off outputs to evaluate completeness.
- **Design decision:** The Definition of Done is declared per-knot, not per-loom. Each knot may have different completeness criteria even within the same loom.
- **Design decision:** Reconciliation is startup-only. The agent evaluates once when Knot starts. Periodic re-conciliation during runtime is a future consideration.
- **Design decision:** Synthetic events use the same event types as normal operation. This means the knot cannot distinguish between a "real" event and a reconciliation-generated event — it processes both identically.
- **Design decision:** The Definition of Done is expressed in natural language (in the knot's markdown body or frontmatter) and evaluated by an agent, not parsed mechanically. This provides maximum flexibility but requires agent invocation on startup.
- **Design decision:** No persistent reconciliation state is stored. Each startup evaluates from scratch against the live project. This means the user can alter state freely and Knot always reflects reality.

## Implementation Status: 🔵 Open
