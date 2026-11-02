# Plan 101: Rig-declared init tasks — the `rig/init/` interface for `knot-init`

- **Date:** 2026-10-29
- **Status:** 🔄 In progress — owner-reviewed 2026-10-29; shim rule
  decided (external-first, local fallback, shadow flag); skill changes
  implemented (knot-init 4.13.0, knot-abstractions 1.5.0,
  knot-update 1.31.0)
- **Consumer (external):** harness plan 007 — *Rig gate policy* (the
  software-factory rig's `init/commit-filter.md` task is the first
  declared task; the interface is generic and carries no gate knowledge)

## Problem

`knot-init` is rig-agnostic by design: it creates the generic rig
structure, seeds profiles, installs skills. But a **rig** may need
project-side initialisation acts that belong to the rig's workflow, not to
knot core and not to the project's docs — the immediate case: installing
the rig's standard gate commit-filter into the project's
`scripts/gate/commit-filter.sh` so the gate policy is in place **at the
start of development** (harness plan 007).

Today such acts have no home. They are either hand-run once (drift-prone,
skipped on fresh projects) or bolted onto unrelated skills. There is no
standard way for a rig to *declare* "when you initialise me, also do X in
the project".

## Target

A generic, optional interface:

```
<rig>/init/
├── commit-filter.md      # task: install the rig's standard commit filter
├── <task>.md             # any further tasks, one file each
└── ...
```

- **Discovery:** `knot-init` resolves the task directory from
  `tie-offs/<rig>/state.json` and lists `init/*.md`.
  - **Plain rig** (no `external_rig` field): the task directory is
    the local `rig/init/`.
  - **Shim rig** (0.64.0): the **external rig is the durable task
    source**. Note that `state.json`'s `rig_path` is the **local**
    rig directory (ADR-013 Decision 5); the resolved external path
    is the additive `external_rig` field. Resolution therefore
    prefers **`<external_rig>/init/`** when it exists; if the
    external rig declares no `init/`, resolution **falls back** to
    the local `rig/init/`.
  - **Shadow flag:** if a shim rig has **both** an external and a
    local `init/`, the external wins and the local folder is
    **flagged** in the init summary —
    `OVERRIDE NOTICE: rig/init/ not executed — shadowed by <external_rig>/init/` —
    left in place, never merged (the never-merge posture; same shape
    as the shim's warn-and-ignore rule for local looms/profiles).
  - Resolved folder absent, or no `*.md` files → **no-op** (the
    common case for plain rigs).
- **Execution:** tasks are read and executed **in sorted filename order**
  (deterministic; prefix with `00-`, `10-` where order matters). Each
  task file is a **self-contained instruction for the init agent** — the
  same skill-agent model the rest of knot-init already uses; **no knot
  binary change** is required.
- **Idempotency is the task's own contract.** Each task declares its
  posture (the standard set: `WRITTEN` / `VERIFIED UNCHANGED` /
  `SKIPPED` / `CONFLICT — left in place`). knot-init re-runs all tasks
  on **every** init invocation → drift self-heal; a `CONFLICT` is
  reported loudly, never auto-resolved (the never-merge posture).
- **Error posture:** a failed task does **not** abort init; it is
  reported in the init summary as `FAILED: <task> — <reason>` and init
  continues (init must stay usable; the failure is visible).
- **Shim rigs (0.64.0):** a shim rig runs the **external rig's**
  declared tasks (when present) against the **consuming project
  root** — the task contract is identical in both rig models. The
  local `init/` is a fallback only, and is flagged when shadowed.

## Changes

| Artifact | Change |
|---|---|
| `.agents/skills/knot-init/SKILL.md` | NEW step 9a (after the rig structure + profiles exist, before the final report): "Execute the rig's declared init tasks" — discovery (external-first shim resolution, local fallback, shadow flag), ordering, per-task report vocabulary, error posture, trust model; step 10 report gains the init-task results; version bump |
| `.agents/skills/knot-init/SKILL.md` (step 4a + Global Skill Installation) | Pre-existing drift fix folded in: the install/verify skill lists omit `knot-visualise`, `knot-template`, `knot-software-factory`, `knot-lint` (all deployed per AGENTS.md). Add them to all four loops and to the generated AGENTS.md skill list in step 4b |
| `.agents/skills/knot-init/knot-glossary.md` | add the term **rig init task** (definition: a rig-declared, self-contained, idempotent project-side init instruction in `<rig>/init/*.md`, executed by knot-init in sorted order) |
| `.agents/skills/knot-abstractions/SKILL.md` §1 (The Rig) | one line: the rig's `init/` folder is part of the rig's *source* surface (tracked in the rig git, never runtime); version bump |
| `knot-update` changelog note | rigs adopting the convention are documented; no migration needed for rigs without `init/` (absent = no-op) |

## Existing tests

| Test asset | Covers | Status |
|---|---|---|
| knot-init existing flow (skill-level, agent-executed) | rig structure, profiles, skill install | ✅ baseline — unchanged by this plan |
| Shim-rig fixtures (plan 100) | `external_rig` provenance in `state.json` | ✅ green — the `external_rig` field is the shim discovery input |

## Test gaps

- No fixture rig **with** an `init/` folder.
- No scripted check of ordering, report vocabulary, or the
  failed-task-does-not-abort posture.

## Phases

### Phase 0 — Interface spec (pane: the contract is written down)
Define the task-file shape (title, purpose, steps, idempotency report
vocabulary), ordering rule, error posture, and shim resolution rule in
`knot-init/SKILL.md` + glossary. Verification: doc review against the
consumer contract in harness plan 007 Phase 3 (the `commit-filter.md`
task must be expressible in the shape without extension).

### Phase 1 — knot-init step + local fixture (pane: a dummy task executes)
Add the execution step to `knot-init`. Fixture: a scratch rig with one
dummy task (writes a sentinel file in the project). Assert: first init →
`WRITTEN`; second init → `VERIFIED UNCHANGED`; a task that always fails
→ `FAILED` reported, init completes; absent `init/` → no-op, report
silent on the section.

### Phase 2 — Shim resolution (pane: external rig's tasks run against the project)
Fixture: external rig + project shim (`external-rig:` config). Three
cases:
1. **External `init/` present** → tasks resolve from
   `<external_rig>/init/` (per `state.json`'s `external_rig` field,
   **not** `rig_path`, which is the local rig dir per ADR-013) and act
   on the consuming project root; report identical to the local case.
2. **External `init/` absent, local `init/` present** → tasks fall back
   to the local `rig/init/`; report identical to the plain-rig case.
3. **Both present** → external tasks run; the local folder is flagged:
   `OVERRIDE NOTICE: rig/init/ not executed — shadowed by
   <external_rig>/init/`; the local folder is left untouched.

**Verification mechanism (all phases):** knot-init is a skill — every
step is agent-executed and there is no `cargo test` coverage for skill
steps. Phases are verified by **agent-walked scratch-directory fixtures**
(temp dirs outside this repo, mock task files, asserted report lines),
not by automated tests and not by live rig runs in this repository
(per AGENTS.md).

## Notes

- The interface deliberately carries **zero domain knowledge** — the
  gate/filter semantics live entirely in the rig's task file (harness
  plan 007). This keeps the knot plan small and lets any rig declare any
  init act.
- A binary-level discovery (knot itself executing tasks at startup) is
  explicitly **out of scope** — the skill-agent execution model matches
  how knot-init already works (every current step is agent-executed). If
  unattended init is ever needed, that is a separate plan.
- Ordering discipline: tasks that depend on each other must encode order
  in filenames; cross-task dependencies beyond ordering are out of
  contract (merge the tasks).
- **Trust model:** init tasks are rig-declared instruction text the
  user chose to point the rig at — the same trust model as the
  external-rig prompt-text contract (ADR-013). The interface carries
  no domain knowledge; all semantics live in the task files.
- **Re-run posture:** the step runs on **every** `knot-init`
  invocation, including when the rig already exists (the skill's
  "report instead of recreate" flow does not short-circuit it) —
  that is what makes drift self-heal work.
- **Skill-surface audit (owner decision):** the only skills requiring
  change for this feature are `knot-init` (+ its glossary),
  `knot-abstractions` (§1 one-liner), and `knot-update` (a
  convention note). The master router's routing is unchanged; the
  software-factory rig's `init/commit-filter.md` task file itself is
  harness plan 007's deliverable, not this repo's.
