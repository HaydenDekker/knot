# Plan: New Skill `project-planner-scope`

> **Status:** 📝 Draft
> **Created:** 2026-08-10

---

## Problem

The `project-` skill suite has **no skill that explains how to identify the
need for further development and how to scope a plan for implementation**.
The suite splits this work across separate concerns:

- **BDD acceptance specs** — owned by `project-acceptance`
  (`project/acceptance/specs/`, `master-bdd.md`).
- **CI solution design** — owned by `project-configuration-management`
  (`project/cm/config-management.md`) and `project-design-document`.
- **Test coverage / validation results** — owned by
  `project-validation-management` (the VCRM at `project/validation/VCRM.md`).

Nothing **connects these back into scoping a new plan**. The VCRM is defined
only as a validation *record/output* (rows = CI, columns = BDD spec, cells =
PASS/FAIL/NOT RUN). No skill reads the VCRM **backwards** — i.e. "these
CI × BDD cells are ⬜ NOT RUN, so we need a plan to implement them." The gap
matrix from `project-architecture-reconciliation` flags missing/drifted CIs
but does not translate them into plan scope.

`project-planner-create` handles *writing* the plan document (and already has
"Tests Are The Plan's Spine"), but it **assumes the work has already been
identified** — it does not say *where to look* to determine what needs doing
or *how to gather the inputs that scope the plan*. That scoping step is a
separate, currently missing responsibility.

This plan creates a new skill, `project-planner-scope`, that sits **before**
`project-planner-create` in the workflow: it details where to look (BDD specs,
CI solution design, VCRM test coverage, gap matrix) and how to scope out a new
plan, then hands the scoped definition to `project-planner-create` to write.

---

## Target

A new skill `project-planner-scope` in the `project-` suite that:

1. **Identifies the need for further development** by cross-referencing three
   primary inputs:
   - **BDD specs** (`project/acceptance/specs/` + `master-bdd.md`) — what
     behaviours are specified as acceptance criteria.
   - **CI solution design** (`project/cm/config-management.md` +
     `project/design/` + SAD) — what the architecture says each CI should be
     and do.
   - **VCRM** (`project/validation/VCRM.md`) — which CI × BDD cells are
     ⬜ NOT RUN, 🟢 PASS, or 🔴 FAIL, revealing test/coverage gaps.
   - Secondary inputs: the reconciliation gap matrix
     (`project/reconciliation/gap-matrix.md`), state inventory
     (`project/state/`), existing test plans (`project/test-plan/`), and the
     master plan (`project/plans/master-plan.md`) to avoid duplicating
     planned/in-progress work.
2. **Scopes the plan** — produces a scoped definition: single usecase focus,
   the CI tags affected, the BDD specs / acceptance criteria targeted, the
   existing tests and test gaps, and the target state. This is the *input* to
   `project-planner-create`.
3. **Hands off** to `project-planner-create` to write the plan document. It
   does **not** write the plan, does not create phase documents, does not
   validate, and does not reconcile.

### Relationship boundary

| Skill | Does | Does NOT |
|---|---|---|
| **`project-planner-scope`** (new) | Identify development need; gather scoping inputs; produce a scoped definition | Write the plan document; create phases; validate; reconcile |
| `project-planner-create` | Write the plan document from a scoped definition | Identify the need; gather BDD/CI/VCRM inputs |
| `project-validation-management` | Run validation, maintain VCRM | Read VCRM *backwards* to drive new development |
| `project-architecture-reconciliation` | Compare contract vs state (gap matrix) | Translate gaps into plan scope |
| `project-acceptance` | Author/maintain BDD specs | Use BDD specs to identify development need |

### Where the skill looks (reference table)

| Input | Location | What it reveals |
|---|---|---|
| BDD specs / master BDD | `project/acceptance/specs/`, `master-bdd.md` | The acceptance criteria (what behaviour proves a feature works) |
| CI solution design (CM doc) | `project/cm/config-management.md` | CI tags, expected outputs, expected test coverage |
| Design docs / SAD | `project/design/`, `project/sad/` | Subsystem detail and system topology for each CI |
| VCRM | `project/validation/VCRM.md` | Which CI × BDD cells are NOT RUN / FAIL / PASS — the coverage gaps |
| Reconciliation gap matrix | `project/reconciliation/gap-matrix.md` | Missing / Drifted / Orphaned CI tags |
| State inventory | `project/state/` | What actually exists (config values, test counts) |
| Test plans | `project/test-plan/` | How each CI's tests are executed |
| Master plan | `project/plans/master-plan.md` | Existing / planned / in-progress work (avoid duplication) |

### Skill metadata

- `name: project-planner-scope`
- `version: 1.0.0`
- `compatibility: "Any project using the project- skill set"`
- Frontmatter `description` with `USE FOR` / `DO NOT USE FOR` covering:
  identify development need, scope a plan, plan scoping, where to look for
  work, plan input gathering, VCRM-driven planning, gap-to-plan translation.

---

## Existing References

| Artifact | Relevance | Status |
|---|---|---|
| `project-planner-create` SKILL.md | The skill that writes the plan; `project-planner-scope` feeds it and must not overlap its "Tests Are The Plan's Spine" section | Read-only reference |
| `project-validation-management` SKILL.md (Validation Workflow, Steps 1–2) | Describes reading plans → BDD targets → VCRM topmost incomplete item; closest existing behaviour to "identify work" | Reference / partially reused |
| `project-architecture-reconciliation` SKILL.md | Gap matrix (Missing/Drifted/Orphaned); run "before plan creation" | Reference input |
| `project-acceptance` SKILL.md | BDD spec authoring, master BDD | Reference input |
| `project-configuration-management` SKILL.md | CI tags, expected outputs, expected test coverage | Reference input |
| `project-management` SKILL.md (Quick Decision Guide) | The suite overview; needs a new row/entry for `project-planner-scope` | To be updated |
| `project-planner-structure` SKILL.md | Cross-cutting planning standards (single usecase, TDD, hexagonal) | Reference |
| `project-initialisation` SKILL.md | Creates `project/` skeleton | Reference (no change) |

---

## Verification (no test suite — documentation-only change)

This is a **documentation-only** change: it adds one new skill `.md` file and
updates the suite overview index. No Rust source, no runtime behaviour, no
tests affected. Verification is consistency-based:

- The new `project-planner-scope/SKILL.md` exists with the correct frontmatter
  (`name`, `description`, `version`, `compatibility`).
- The skill's `USE FOR` / `DO NOT USE FOR` boundaries are consistent with the
  relationship table in §Target (does not write the plan, does not validate,
  does not reconcile).
- `project-management` Quick Decision Guide contains a row routing
  "identify development need / scope a plan" → `project-planner-scope`.
- Cross-references resolve (relative `../<skill>/SKILL.md` links point to
  existing skills).
- The skill installs cleanly to `~/.agents/skills/` (diff verification).

---

## Phases

### Phase 0: Design the skill content (analysis)

Draft the full `project-planner-scope/SKILL.md` content:

- **Core philosophy**: scoping is *identification + input-gathering*, distinct
  from plan *writing* (`planner-create`) and from *validation*
  (`validation-management`).
- **Where to look** — the reference table in §Target: BDD specs, CI solution
  design (CM doc + design docs), VCRM test coverage, reconciliation gap
  matrix, state inventory, test plans, master plan.
- **How to scope** — a step-by-step workflow:
  1. Read the master plan to avoid duplicating planned/in-progress work.
  2. Read the CM doc + design docs to understand the CI solution design.
  3. Read the VCRM to find ⬜ NOT RUN / 🔴 FAIL CI × BDD cells (coverage gaps).
  4. Read the reconciliation gap matrix for Missing/Drifted/Orphaned CIs.
  5. For each candidate gap, read the relevant BDD spec(s) for UAT detail;
     if there is no clear BDD test, note that validation cannot proceed and
     the gap is implementation-only.
  6. Determine a **single usecase** focus and the affected CI tags.
  7. Identify existing tests (from state inventory / test plans) and test
     gaps.
  8. Produce the scoped definition (usecase, CIs, BDD targets, existing
     tests, test gaps, target state).
- **Hand-off**: pass the scoped definition to `project-planner-create`.

**Artifact:** `project/plans/067-project-planner-scope/scope-design.md` (a
draft of the skill's structure and key sections).

### Phase 1: Write the skill

Create `.agents/skills/project-planner-scope/SKILL.md` with the content
designed in Phase 0. Follow the standard `project-*` skill format (frontmatter
+ body with Core Philosophy / When to Use / Where to Look / How to Scope /
Hand-off / Related Skills / Cross-References).

Include a "Relationship to Other Skills" table mirroring §Target, and a
"DO NOT USE FOR" that explicitly excludes plan writing, phase creation,
validation, and reconciliation (to keep the boundary sharp).

**Artifact:** `.agents/skills/project-planner-scope/SKILL.md`
**Metadata:** `version: 1.0.0`.

### Phase 2: Register in the suite overview

Update `~/.agents/skills/project-management/SKILL.md`:

- Add `project-planner-scope` to the **Planning and Execution** table.
- Add a row to the **Quick Decision Guide** ("Defining a feature or refactor"
  or a new "Scoping work" section): situation "Identify development need /
  scope a new plan" → `project-planner-scope`.
- Add the `project/` directory entry for any new artifacts (none expected —
  scoping produces no persistent file; it feeds `planner-create`).

**Artifact:** `.agents/skills/project-management/SKILL.md`
**Metadata:** bump `version` from `1.1.0` to `1.2.0`.

### Phase 3: Publish and verify

1. Install the new skill globally (with verification):
   ```bash
   cp -r .agents/skills/project-planner-scope ~/.agents/skills/project-planner-scope
   diff .agents/skills/project-planner-scope/SKILL.md \
        ~/.agents/skills/project-planner-scope/SKILL.md > /dev/null 2>&1 && \
     echo "project-planner-scope: OK" || echo "project-planner-scope: FAILED"
   ```
2. Confirm the skill's frontmatter and boundary:
   ```bash
   grep -n "name:"     ~/.agents/skills/project-planner-scope/SKILL.md
   grep -n "DO NOT USE FOR" ~/.agents/skills/project-planner-scope/SKILL.md
   ```
3. Confirm the suite overview routes to the new skill:
   ```bash
   grep -n "project-planner-scope" ~/.agents/skills/project-management/SKILL.md
   ```

---

## Cross-Reference

- **`project-planner-create`** — the consumer of the scoped definition; the
  boundary this skill must respect.
- **`project-validation-management`** — source of the VCRM (coverage gaps) and
  the "read plans → BDD → VCRM topmost incomplete item" pattern.
- **`project-architecture-reconciliation`** — source of the gap matrix.
- **`project-acceptance`** — source of BDD specs.
- **`project-configuration-management`** — source of the CI solution design.
- **`project-management`** — suite overview updated to register the skill.
- **`project-planner-structure`** — cross-cutting planning standards.
