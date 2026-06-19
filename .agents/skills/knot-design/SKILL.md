---
name: knot-design
description: "Design looms and knots for the Knot agent orchestration framework. Covers idempotency, naming conventions, responsibility boundaries, domain direction, loop design, and loop-breaking patterns. USE FOR: design knot, design loom, knot design, loom design, knot architecture, agent loop, feedback loop, knot naming, strand direction, knot responsibility, idempotent knot, loop-breaking, knot workflow design. DO NOT USE FOR: creating looms/knots (use knot-create), initialising a rig (use knot-init), inspecting state (use knot-inspect)."
---

# Knot Design Skill

Design looms and knots that are idempotent, correctly scoped, and resilient
to feedback loops.

This skill captures design principles learned from building and debugging
real Knot rigs. Use it when planning a new loom, reviewing knot boundaries,
or diagnosing loop behaviour.

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

```yaml
prompt-template:
  instructions: |
    You are a <role>. <Goal statement>.

    1. Read the strand (provided).
    2. Inspect current state of <target domain>.
    3. Determine if the goal is already met.
    4. If yes, report "no changes needed" with explanation.
    5. If no, apply minimal changes to achieve the goal.

    ## Constraints
    - Never overwrite work in <other domain> — only append observations.
    - Re-running this on the same strand must produce no additional changes.
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
