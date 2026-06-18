# Design Guide

Best practices for designing looms and knots that are idempotent,
correctly scoped, and resilient to feedback loops.

## Idempotency — The First Rule

A knot can retrigger multiple times from the same strand. The strand
does not advance — Knot re-reads it whenever the file changes or the
loom is reprocessed. **Every knot must be idempotent.**

### What Idempotency Means

A knot is not a one-shot script. It is a **goal-seeking agent** that:

1. **Reads its strand** (the trigger file that changed).
2. **Inspects current state** of whatever it is responsible for.
3. **Compares** the strand's requirements against current state.
4. **Applies only the changes needed** to reach the goal.
5. **Reports** what it did (or why nothing was needed).

If the knot runs again on the same strand, steps 2–4 find that the goal
is already achieved, and no further changes are made.

### Goal-Focused, Not Step-Focused

Write knot instructions that describe **what should exist**, not
**what steps to perform**:

**❌ Step-focused — fails on re-run:**

```
1. Create the plan file
2. Add phase 0
3. Add phase 1
```

If the strand re-triggers, this appends duplicate phases.

**✅ Goal-focused — idempotent:**

```
Ensure the plan file exists at project/plans/<slug>.md and contains
phases that deliver the stated goals. Inspect the file first. If it
already contains aligned phases, make no changes. If phases are
missing or misaligned, update in place.
```

### Idempotency Checklist

- [ ] Instructions begin with a **goal statement**, not a procedure
- [ ] The knot **reads current state before writing**
- [ ] The knot can explain **why no changes were needed**
- [ ] Re-running on the same strand produces **no additional mutations**
- [ ] Tie-off output is **append-only** (never rewrite previous entries)

## Naming Conventions

### Knot Names: `<source-domain>-<target-action>`

Name the knot after what it reads and what it produces:

| Name | Reads | Writes | Meaning |
|------|-------|--------|---------|
| `prd-planner` | `project/prds/` | `project/plans/` | PRD changes → create/align plans |
| `adr-planner` | `project/adrs/` | `project/plans/` | ADR changes → align plans |
| `plan-architect` | `project/plans/` | `project/adrs/` | Plan changes → inform ADRs |

The name is the specification. If you can't describe the data flow
direction from the name, rename it.

### Loom Placement: By Output Domain

Group knots into looms by **what they produce**, not what they read:

- `planning-loom/` — knots that produce or maintain plans
- `architecture-loom/` — knots that produce or maintain ADRs
- `docs-loom/` — knots that produce or maintain documentation

### Anti-Pattern: Duplicate Responsibility

**❌ Bad** — two knots in different looms that read the same source and
write to the same target:

```
planning-loom/adr-planner.md              ← reads ADRs, writes plans
architecture-loom/adr-planner-plans.md    ← reads ADRs, writes plans (duplicate!)
```

**✅ Good** — each data flow direction has exactly one knot:

```
ADR → plan : planning-loom/adr-planner.md
plan → ADR : architecture-loom/plan-architect.md
```

## Responsibility Boundaries

Each knot should have a **single, clear goal**. The goal determines
what the knot reads, what it writes, and what it ignores.

**Clear responsibility:**

> Goal: "Plans reflect the ADR's decisions."
>
> The `adr-planner` reads ADRs, inspects plans, and updates plans to
> align with ADR decisions. It does not modify ADRs.

**Unclear responsibility (anti-pattern):**

> "Keep everything in sync."
>
> This knot reads ADRs, plans, and PRDs, and modifies all three. It
> has no clear authority and conflicts with other knots.

### Rules for Responsibility

1. **One goal per knot.** If a knot has two goals, split it into two
   knots.
2. **Read the other's output as input, not as authority.** A knot
   respects what another knot produced but may note gaps.
3. **Append observations, don't overwrite.** If knot A manages plans
   and knot B reads plans, knot B should only append notes to plans —
   never rewrite knot A's sections.

## Loop Design

When knots flow in opposite directions, they form a **feedback loop**:

```
ADR change → adr-planner updates plan
Plan change → plan-architect updates ADR
ADR change → adr-planner updates plan
...
```

This is correct and expected. The loop **converges** when both sides
agree — no more changes are needed.

### How Loops Converge

Each knot is idempotent and goal-focused. The loop terminates when:

1. `adr-planner` runs: plan already aligned → no changes
2. `plan-architect` runs: ADR already captures plan's needs → no changes
3. Both report "no changes needed" → convergence

Tie-off files provide the audit trail showing each iteration.

### Loop Design Checklist

- [ ] Each knot has a **different goal** (information actually flows)
- [ ] Each knot is **idempotent** (re-running converges)
- [ ] One knot is the **authority** for each domain
- [ ] Tie-off files provide an **audit trail** of iterations
- [ ] A **stale-strand check** exists (skip if strand content unchanged)
- [ ] A **human-escalation path** exists (max iterations or oscillation detection)

### Breaking Non-Convergent Loops

Sometimes loops do not converge. Three patterns to prevent infinite
oscillation:

#### Break 1: One-Way Authority

Designate one knot as authoritative for each domain. The authoritative
side does not react to changes from the other side.

Example: ADRs are authoritative for architecture decisions — only the
user or a PRD-driven knot creates them. The `adr-planner` reads ADRs
but never modifies them.

#### Break 2: Status-Gating

A knot only acts when the strand is in a specific status:

| Status | Meaning | Who sets it | Knot acts? |
|--------|---------|-------------|------------|
| 🔴 Draft | Initial draft | Auto-created | No |
| 🟡 Review | Needs approval | Secondary knot | No |
| 🟢 Approved | User approved | User only | Yes |

The loop advances through statuses and only proceeds when the user
explicitly approves.

#### Break 3: Strand Acknowledgement

The knot records the strand content hash in its tie-off. On re-trigger,
it compares the hash and skips if unchanged:

```
Processed ADR-001 (sha256: abc123...) — no changes needed
```

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

### Step 3: Define the Goal (One Sentence)

```
Goal: "<target> reflects <source>'s decisions/constraints."
```

### Step 4: Place It in a Loom

The loom matches the knot's output domain:

- Writes plans → `planning-loom/`
- Writes ADRs → `architecture-loom/`

### Step 5: Write Goal-Focused Instructions

```yaml
prompt-template:
  input-bundling: "full-file"
  instructions: |
    You are a <role>. <Goal statement>.

    1. Read the strand (provided).
    2. Inspect current state of <target domain>.
    3. Determine if the goal is already met.
    4. If yes, report "no changes needed" with explanation.
    5. If no, apply minimal changes to achieve the goal.

    ## Constraints
    - Never overwrite work in <other domain> — only append observations.
    - Re-running on the same strand must produce no additional changes.
```

### Step 6: Check for Loops

Does another knot read the target domain and write back to the source
domain? If yes, document the loop and ensure a convergence mechanism
exists (see loop breaking patterns above).
