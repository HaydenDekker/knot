---
name: knot-design
description: "Design looms and knots for the Knot agent orchestration framework. Covers idempotency, naming conventions, responsibility boundaries, domain direction, loop design, and loop-breaking patterns. USE FOR: design knot, design loom, knot design, loom design, knot architecture, agent loop, feedback loop, knot naming, strand direction, knot responsibility, idempotent knot, loop-breaking, knot workflow design. DO NOT USE FOR: creating looms/knots (use knot-create), initialising a rig (use knot-init), inspecting state (use knot-inspect)."
license: MIT
metadata:
  author: Knot Team
  version: "1.3.0"
  compatibility: "Knot 0.26.0+"
---

# Knot Design Skill

Design looms and knots that are idempotent, correctly scoped, and resilient
to feedback loops.

This skill captures design principles learned from building and debugging
real Knot rigs. Use it when planning a new loom, reviewing knot boundaries,
or diagnosing loop behaviour.

---

## ⚠️ Never Leak Internal Terminology Into Prompts or Event Descriptions

The markdown body of a knot (its instructions) and the `event-description`
frontmatter field are **injected directly into the agent's prompt**. They
are **not** internal documentation — they are what the agent reads and
follows. Therefore:

**Never use knot-specific internal terms in prompts or event descriptions.**
Use generic, domain-agnostic language instead. Workflows should be reusable
across different orchestration systems, not tied to Knot's terminology.

| ❌ Knot-specific (do NOT use in prompts) | ✅ Generic (use instead) |
|------------------------------------------|--------------------------|
| tie-off                                  | final response |
| strand                                   | input file, work item, trigger file |
| knot                                     | task |
| loom                                     | workspace |
| strand-dir                               | input directory, source path |
| tie-off directory                        | output directory |
| tie-off file                             | output document, result file |
| event                                    | message, notification, signal |
| event-description                        | (use a plain `description:` or `summary` field) |

### Template for Generic Instructions

When writing knot instructions, follow this pattern:

```markdown
You are a <role>. <Goal statement>.

1. Read the <input file>.
2. Inspect current state of <target domain>.
3. Determine if the goal is already met.
4. If yes, report "no changes needed" with explanation.
5. If no, apply minimal changes to achieve the goal.
6. Write your <output document> at the expected output path.

## Constraints
- Never overwrite work in <other domain> — only append observations.
- Re-running on the same <input file> must produce no additional changes.
```

### Examples

**❌ Bad — leaks internal terminology:**

```markdown
Read the strand. Process it. Append to the tie-off file.
Emit an event in your tie-off if one occurs.
```

**✅ Good — uses generic terms:**

```markdown
Read the input file. Process it. Write your final response
to the output document. Emit a notification in your output
document if one occurs.
```

---

## Idempotency — The First Rule

A knot can retrigger multiple times from the same strand. The strand
does not advance — Knot re-reads it whenever the strand file changes
or the loom is reprocessed. Therefore **every knot must be idempotent**.

### What Idempotency Means for Knot Design

A knot is not a one-shot script. It is a **goal-seeking agent** that:

1. **Reads its strand** (the trigger file that changed).
2. **Inspects current state** of whatever it is responsible for.
3. **Compares** the strand's requirements against current state.
4. **Applies only the changes needed** to reach the goal.
5. **Reports** what it did (or why nothing was needed).

If the knot runs again on the same strand, steps 2–4 find that the goal
is already achieved, and no further changes are made.

### Designing for Idempotency

Write knot instructions that are **goal-focused, not step-focused**:

**❌ Bad (step-focused — fails on re-run):**

```
1. Create the plan file
2. Add phase 0
3. Add phase 1
```

If the strand re-triggers, this appends duplicate phases.

**✅ Good (goal-focused — idempotent):**

```
Ensure the plan file exists at project/plans/<slug>.md and contains
phases that deliver [goal]. Inspect the file first. If it already
contains aligned phases, make no changes. If phases are missing or
misaligned, update in place.

Write your final response to the output document.
```

### Idempotency Checklist

When designing a knot, verify:

- [ ] Instructions begin with a **goal statement**, not a procedure
- [ ] The knot **reads current state before writing**
- [ ] The knot can explain **why no changes were needed**
- [ ] Re-running on the same strand produces **no additional mutations**
- [ ] Tie-off output is **append-only** (never rewrite previous entries)

---

## Naming and Responsibility

A knot's name and loom placement encode its **responsibility** and
**data flow direction**.

### Convention: `<source-domain>-<target-action>`

Name the knot after what it reads and what it does:

| Knot name | Reads (strand-dir) | Writes/Updates | Meaning |
|-----------|-------------------|----------------|---------|
| `prd-planner` | `project/prds/` | `project/plans/` | PRD changes → create/align plans |
| `adr-planner` | `project/adrs/` | `project/plans/` | ADR changes → align plans |
| `plan-architect` | `project/plans/` | `project/adrs/` | Plan changes → inform ADRs |
| `architecture-planner-prds` | `project/prds/` | `project/adrs/` | PRD changes → draft ADRs |

### Placing Knots in Looms

Group knots into looms by **domain concern**, not by file system:

- `planning-loom/` — knots that produce or maintain plans
  - `prd-planner.md` — PRD → plan creation
  - `adr-planner.md` — ADR → plan alignment
- `architecture-loom/` — knots that produce or maintain ADRs
  - `architecture-planner-prds.md` — PRD → ADR drafting
  - `plan-architect.md` — plan → ADR feedback

A loom is a **namespace for a domain of responsibility**, not a
technical grouping. If a knot writes plans, it belongs in the planning
loom regardless of what it reads.

### Anti-Pattern: Duplicate Responsibility

**❌ Bad:** Two knots in different looms that read the same strand
directory and write to the same output directory with overlapping logic.

```
planning-loom/adr-planner.md       ← reads ADRs, writes plans
architecture-loom/adr-planner-plans.md ← reads ADRs, writes plans (duplicate!)
```

Both fire on `project/adrs/` changes and both rectify plans. The second
is a duplicate with slightly more verbose rules — it adds no new
capability and risks conflicting edits.

**✅ Good:** Each data flow direction has exactly one knot.

```
ADR → plan : planning-loom/adr-planner.md
plan → ADR : architecture-loom/plan-architect.md
PRD → ADR  : architecture-loom/architecture-planner-prds.md
PRD → plan : planning-loom/prd-planner.md
```

### The Real Mistake That Created the Duplicate

The knot `adr-planner-plans` was intended to be a knot that reads
**plans** and updates **ADRs** — i.e., "when a plan is drafted, check
if the ADRs capture what the plan is trying to achieve." Its true name
was `plan-architect`. Because it was placed in `architecture-loom/`
with a name suggesting "ADR → plans," it was written as a duplicate of
`adr-planner` instead of as its complementary reverse-flow knot.

**Lesson:** Name the knot after its actual data flow direction
(`<source> → <target>`), not after what you think it should be called.
The name is the specification.

**Concrete example:** The knot `adr-planner-plans` was intended to read
plans and update ADRs. But its name suggested "ADR → plans," so it was
written as a duplicate of `adr-planner` instead of as the complementary
`plan-architect`. Renaming it to `plan-architect` made its true purpose
obvious: it reads plans (source) and architects (acts on) ADRs (target).

---

## Events as Signals — One Reason to Change

Events are the primary mechanism for knots in one loom to inform knots
in another. They are **signals**, not commands. The event announces
that something happened; the target knot owns all investigation and
reaction.

### The Event Contract

An event strand is a markdown file with frontmatter carrying **facts**
and a body carrying **context**. The target knot reads the event and
then investigates whatever state it is responsible for.

**Frontmatter — the signal (immutable facts):**

```yaml
---
event-id: <EventName>
target-knot: <knot-name-or-loom>
timestamp: <ISO-8601>
<fact-key>: <fact-value>
---
```

**Body — the context (read-only narrative):**

```markdown
## Event: <EventName> from <source-knot>

<Narrative explanation of what happened. The target knot can use this
for context but must not treat it as an instruction.>
```

### Events Carry Facts, Not Instructions

**❌ Bad — the event tells the target what to do:**

```markdown
---
event-id: NonConformance
---

## Event

Go look up the plan and create a rectification plan.
You should make the plan at R-005-browse-common-rectify.
Then update the master plan index.
```

The event has become a command. It has absorbed the target knot's
responsibility. If the target knot changes its process, the event
format also needs to change — two things changing for one reason.

**✅ Good — the event states what happened:**

```markdown
---
event-id: NonConformance
plan: 005-browse-common
ci-job: frontend
scenarios: Remove a member, Create a 1-to-1 private Common
result: NOT RUN
---

## Event: NonConformance from completion-validator

TestReady event declared 6 scenarios as delivered. Vitest validation
found only 2 covered by tests. 4 scenarios are NOT RUN.
```

The event announces facts. The target knot (e.g. `nonconformance-planner`)
reads these facts, reads the plan, reads the BDD spec, and decides
what kind of rectification is needed. The event has **one reason to
change**: the validation result. The target knot has **one reason to
change**: the rectification process.

### Why This Matters — SOLID Applied

The **Single Responsibility Principle** applies to events as well as
to code. An event should have **one reason to change**: the state it
represents. It should not encode the reaction logic of its consumer.

```
Event (source knot)      Target knot
─────────────             ─────────────────
States a fact             Reads the fact
One reason to change:     One reason to change:
the thing that happened    the reaction process
```

If the event contained instructions for the target, then any change
in the target's process would also require changing the event format.
Two things changing for one reason — a SOLID violation.

### The Source Loom Owns Event Emission

The knot that emits an event lives in the loom where the **mutation
that caused the event** occurred. The event is a record of that
mutation. It does not contain the target's reaction.

Crucially, the producer knot does not declare its events. It does not
know it has consumers. The consumer declares the subscription via
`strand-dir`, and Knot injects the emission instructions at runtime.
This is a fundamental asymmetry:

```
Consumer side  (declares):
  strand-dir: "event:completion-validator:NonConformance"
  event-description: "Triggered when validation fails"

Producer side  (runtime — injected by Knot):
  ## Agent Events
  Other knots are listening for events you may emit.
  Events you may emit: NonConformance
  If an event occurred, emit in your tie-off:
    event: NonConformance
```

The producer's `.md` file contains no reference to `NonConformance`.
If you change your mind about who listens, you edit only the consumer.
The producer never needs to change.

For example:
- `completion-validator` emits `NonConformance` → lives in `validation-loom`
  because validation is where the non-conformance was discovered
- The event carries `plan`, `ci-job`, `scenarios`, `result` → facts about
  what validation found
- The event does NOT contain "create plan R-005-rectify" → that is the
  `nonconformance-planner` knot's job

### Designing an Event — Checklist

When designing an event between two knots:

- [ ] The event frontmatter carries **facts only** (identifiers, values)
- [ ] The event body provides **context** (narrative), not instructions
- [ ] The event has **one reason to change** (the state it represents)
- [ ] The target knot can **reproduce the full picture** from its own state
- [ ] The event does **not encode the target's process**
- [ ] If the target's process changes, the event format **does not need to change**
- [ ] The event name describes **what happened**, not what should be done
  (`NonConformance`, not `CreateRectificationPlan`)
- [ ] The target knot is named after what it reads and what it does
  (`nonconformance-planner`, not `plan-creator`)

---

## Event Subscription Design

When designing knots that consume events, choose the right subscription
level to keep your rig maintainable.

### How Event Wiring Works

Events are a **Knot runtime mechanism**, not something knots hardcode
in their instructions. The wiring is entirely declarative:

1. **Consumer declares the subscription** using `strand-dir` in its
   frontmatter:

   ```yaml
   strand-dir: "event:<producer-target>:<EventName>"
   ```

   This means: "listen for `<EventName>` events emitted by knots in
   `<producer-target>`." The `<producer-target>` is either a knot ID
   (knot-level) or a loom ID ending in `-loom` (loom-level).

2. **Knot discovers the subscription** and creates a dispatch directory
   at `rig/tie-offs/{consumer-loom-id}/{EventName}/`.

3. **Knot injects event instructions** into the producer's prompt at
   runtime — grouped by event ID, deduplicated across consumers. The
   producer knot does not need to know it has consumers; Knot tells it
   how and when to emit.

4. **Producer emits in its tie-off** (`event: EventName` or
   `event: None`). Knot parses it and writes event files into the
   consumer's dispatch directory.

5. **Consumer processes the event** as a strand.

```yaml
# Consumer declaration — this is the ONLY wiring needed
strand-dir: "event:completion-validator:DependencyIncomplete"
event-description: >
  Trigger if development skips a CI where the SAD defines an order.
```

Multiple knots can listen to the same event. Each knot in its own
loom processes it independently according to its own responsibility.

| Event | Listeners | Each Does |
|-------|-----------|----------|
| `ProgressReport` | `documentation-loom/progress-journalist`, `review-loom/implementation-review` | One writes docs, one reviews code |
| `TestReady` | `validation-loom/completion-validator` | Validates against BDD |
| `TestReady` | `planning-loom/dep-order-planner` | Checks CI dependency order, creates upstream plans |
| `NonConformance` | `planning-loom/nonconformance-planner` | Creates rectification plan |

### Knot-Level vs Loom-Level Subscriptions

**Knot-level** (`event:<knot-name>:<EventId>`) subscribes to events
from a *specific* producer knot. Use when:
- You need events from exactly one known producer.
- Different knots in the same loom emit different event types.
- You want precise control over which producer triggers the consumer.

**Loom-level** (`event:<loom-name>:<EventId>`) subscribes to events
from *any knot* within a loom. Use when:
- Multiple knots in a loom can emit the same event type.
- You expect knots to be added to the loom over time.
- The consumer doesn't care which specific knot produced the event,
  only that the event occurred within that domain.

### When to Prefer Loom-Level

```
# ❌ Knot-level — repetitive when multiple producers exist
strand-dir: "event:prd-planner:PlanCreated"
strand-dir: "event:adr-planner:PlanCreated"   # separate knot needed!
strand-dir: "event:goal-planner:PlanCreated"   # another one!

# ✅ Loom-level — one subscription covers the whole loom
strand-dir: "event:planning-loom:PlanCreated"
```

With loom-level, adding a new knot to `planning-loom` that emits
`PlanCreated` automatically triggers the consumer — no subscription
changes needed.

### Loom-Level Design Checklist

When choosing a loom-level subscription:

- [ ] Multiple knots in the loom *can* emit the same event type
- [ ] The consumer's logic is *producer-agnostic* (doesn't need to know which knot produced it)
- [ ] The loom's knot set might grow over time
- [ ] The target loom name ends in `-loom` (required by convention)

### Event Injection Scope

With a loom-level subscription to `event:planning-loom:PlanCreated`:
- **Every knot** inside `planning-loom` receives event injection
  instructions in its prompt.
- This means every knot knows it should emit an `event:` block in its
  tie-off if the event occurred during its turn.
- Events from knots that *don't* actually trigger the event simply
  emit `event: None` — no harm.

### Resolution Precedence

If a target name matches both a knot name and a loom name:
- Loom-level takes precedence if the target ends in `-loom`.
- This is a naming constraint: knot names should not end in `-loom`.

### Backward Compatibility

All existing `event:<knot-name>:<EventId>` URIs continue to work
unchanged. The loom-level feature is additive — no migration needed
for existing subscriptions.

---

## Loop Design

When you have knots flowing in opposite directions, you create a
**feedback loop**:

```
ADR change → adr-planner updates plan
Plan change → plan-architect updates ADR
ADR change → adr-planner updates plan
Plan change → plan-architect updates ADR
...
```

This is **correct and expected**. The loop converges when both sides
agree — no more changes are needed on either side.

### How Loops Converge

Each knot is idempotent and goal-focused. The loop terminates when:

1. `adr-planner` runs: plan already aligned with ADR → no changes
2. `plan-architect` runs: ADR already captures plan's needs → no changes
3. Both knots report "no changes needed" → convergence reached

The tie-off files provide the audit trail showing each iteration
and the eventual stable state.

### Designing Knots That Loop Well

**Rule 1: Each knot has a single, clear goal.**

The `adr-planner` goal: "plans reflect the ADR's decision."
The `plan-architect` goal: "ADRs capture what the plan needs."

These goals are different but compatible. If both knots had the same
goal, the loop would be vacuous (no information flows).

**Rule 2: Each knot reads the other's output as input, not as authority.**

The `adr-planner` treats the ADR as authoritative for architecture
decisions but may note that the plan reveals a gap.
The `plan-architect` treats the plan as authoritative for what is being
built but may note that the ADR already covers it.

**Rule 3: Knots append observations, they don't overwrite the other's work.**

The `plan-architect` might add a note to an ADR:
"Plan 002 introduces concept X — consider whether this needs a decision."
It does not rewrite the ADR's decision.

### Detecting and Breaking Loops

Sometimes a loop does not converge. Design for this.

#### Detection Patterns

**Pattern 1: Tie-off oscillation.**

If the same knot produces different tie-off conclusions on
consecutive runs (e.g., "changed A → no change → changed A"),
the loop is oscillating rather than converging.

The knot should detect this by reading its own tie-off file before
acting. If the last two entries show opposite actions on the same
strand version, log a warning and defer to human review.

**Pattern 2: Strand version pinning.**

Each strand modification has a file mtime or content hash. If a knot
detects it has already processed the same strand content (comparing its
tie-off record against the current strand hash), it skips processing.

**Pattern 3: Maximum iteration count.**

A profile system prompt or knot instruction can enforce a guard:
"If this ADR has been modified more than N times by this knot, stop
and report for human review."

#### Breaking Patterns

**Break 1: One-way authority.**

Designate one knot as the authority for each domain:
- ADRs are authoritative for architecture decisions → only user or
  `architecture-planner-prds` creates them; `adr-planner` reads them;
  `plan-architect` only appends observations.
- Plans are authoritative for implementation scope → only user or
  `prd-planner` creates them; `adr-planner` aligns them; `plan-architect`
  reads them.

When authority is one-way, the loop has a natural stop condition:
the authoritative side does not react to the other's changes.

**Break 2: Status-gating.**

A knot only acts when the strand is in a specific status. This creates
a **lifecycle-driven loop** that advances through states rather than
oscillating on the same state.

The ADR → plan → ADR loop uses a three-state ADR lifecycle:

| Status | Icon | Who sets it | adr-planner acts? |
|--------|------|-------------|------------------|
| `🔴 Draft` | Initial draft | `architecture-planner-prds` | **No** |
| `🟡 Review` | Needs user approval | `plan-architect` only | **No** |
| `🟢 Approved` | User approved | User only | **Yes** |

The `adr-planner` has a hard gate: it checks the ADR status as its
first step. If Draft or Review, it reports "no changes made" and exits.
This means the loop can only advance when the user approves — the human
is the gate between iterations.

```
Plan change → plan-architect reviews ADRs → adds detail → 🟡 Review
     ↑                                              |
     |            User approves → 🟢 Approved        |
     |            adr-planner updates plan ----------┘
```

Without status-gating, the loop would oscillate:
adr-planner changes plan → plan-architect changes ADR → adr-planner
changes plan again. With status-gating, the loop advances through
statuses and only proceeds when the user explicitly approves.

**Break 3: Strand acknowledgement.**

The knot appends an acknowledgement line to its tie-off:

```
Processed ADR-001 (sha256: abc123...) — no changes needed
```

On re-trigger, the knot reads its tie-off, compares the strand hash.
If the hash matches, the knot skips: "Already processed this version."

### Loop Design Checklist

When designing a pair of knots that form a loop:

- [ ] Each knot has a **different goal** (information actually flows)
- [ ] Each knot is **idempotent** (re-running converges)
- [ ] One knot is the **authority** for each domain (breaks infinite loops)
- [ ] Tie-off files provide an **audit trail** of iterations
- [ ] A **stale-strand check** exists (skip if strand content unchanged)
- [ ] A **human-escalation path** exists (max iterations or oscillation detection)

---

## Designing a New Knot — Step by Step

### Step 1: Define the Data Flow

```
Source: <which strand-dir?>
Target: <what files does it create or modify?>
Direction: <source> → <target>
```

### Step 2: Name It

```
<source-domain>-<target-action>.md
```

Examples: `adr-planner`, `plan-architect`, `prd-planner`.

### Step 3: Define the Goal (One Sentence)

```
Goal: "<target> reflects <source>'s decisions/constraints."
```

### Step 4: Place It in a Loom

The loom matches the knot's **output domain**:
- Writes plans → `planning-loom/`
- Writes ADRs → `architecture-loom/`

### Step 5: Write the Instructions (Goal-Focused)

Knot instructions go in the **markdown body** (after the closing `---`),
not in frontmatter:

```markdown
---
name: <knot-name>
agent-profile-ref: <profile>
strand-dir: "<path>"
---

You are a <role>. <Goal statement>.

1. Read the input file (provided).
2. Inspect current state of <target domain>.
3. Determine if the goal is already met.
4. If yes, report "no changes needed" with explanation.
5. If no, apply minimal changes to achieve the goal.
6. Write your final response.

## Constraints
- Never overwrite work in <other domain> — only append observations.
- Re-running on the same input file must produce no additional changes.
```

### Step 6: Check for Loops

Does another knot read the target domain and write back to the source
domain? If yes:

- Document the loop in the knot's markdown body
- Ensure one side has authority (see Breaking Patterns above)
- Add a stale-strand check

---

## Cross-Reference

Related skills:

1. **knot-create skill** — create the `.md` files for looms and knots
2. **knot-inspect skill** — verify loom state and knot processing
3. **knot-init skill** — initialise the rig (prerequisite)

This skill covers the **design** decisions. Use `knot-create` for the
actual file writing and verification.
