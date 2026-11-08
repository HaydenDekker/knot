# Plan 104: `knot-standards` — the Self-Contained Home for the Two Face-Standards on Rig-Authoring

**Implementation Status:** ⬜ Planned (2026-11-08) — drafted for owner
review. Docs-only: one new skill + four skill hooks + version bumps.
No engine changes, no rig changes, no new tests.

## Problem

The two face-standards for artifact authoring — **vocabulary-scope**
(a document's terms must be resolvable inside its bounded context:
the definer set, first-use declaration, the polysemy rule) and
**content-scope** (single normative home, one editor per shared
document act-based, one-hop pointers) — are needed at authoring time
for the rig's normative documents. They currently exist only in the
**agents repo** (`~/.agents/skills-library/{vocabulary-scope,
content-scope}`, v1.0.0, agents plans 012 + 013 — the 013 creation
eval, n = 1, is the positive result: the skillless control took both
planted traps, 1/3 vs the treatment's 3/3).

But **this project is standalone**: its skills must be self-contained
within the knot project — the same rule the agents repo applies to
its `skills` domain (normative references may not reach into another
project's skill tree; cross-project names appear as **provenance in
plan notes, never as the source of a skill's behaviour**). A hook in
a knot skill that pointed at `~/.agents/skills-library/...` as its
normative home would break the boundary: the knot skill's behaviour
would depend on another project's files, and a standalone knot
checkout (no agents clone) would have a dangling normative pointer.

**Field evidence (the harness cross-repo review, 2026-11-08):** the
one genuine creation-time defect found in the software-factory rig's
007/008/009-era documents — `coding-implementation-loom/
plan-implementer.md:51` prescribing a plan citation
("— harness plan 009") into consumer-facing records; 33
provenance-citation hits in three tiers across ~15 files; the "L1"
polysemy (a plan-born term colliding with arc42's established "L1
whitebox" in every SAD); the K1 wiring-boundary violations (knot
prose > 80 words or ≥ 3 numbered steps prescribed into the knot
body — the knot-lint baseline runs 34 K1 WARN); and the K2
single-editor failures (the L1 register written by three documents —
baseline 3 FAIL + 1 missed K2). Each of these is a bounded-context
or content-placement defect at **creation time** — the surface this
plan closes on the knot side.

## Target

A new self-contained knot skill — **`knot-standards`** — that is the
**normative home, within this project, for the two face-standards as
they apply to the rig's normative documents**, plus a short
compliance hook in each of the four knot skills that author those
documents, pointing at `knot-standards`.

**The home skill — `knot-standards`:**

- A port of the two face-standards' content, **self-contained**: all
  rules, the term classes, the artifact-class table, the advisory
  sweep procedure, and the advisory-report convention are written
  inline — no normative pointer to the agents repo (or anywhere
  outside the knot project). The agents plans 012 + 013 and their
  eval records are cited in the plan notes and the skill's
  provenance note **only** (where the standard came from and how it
  was measured) — never as the source of the skill's behaviour.
- **The bounded context is adapted to the rig:** the definer set is
  the rig's own documents (the knot bodies, the profiles, the agent-
  event documents, the interface documents, the two instruction
  documents) plus the engine's documented vocabulary; a term a rig
  document uses must be resolvable from that set, or first-use-
  declared in the document. The polysemy rule applies against the
  consumer's context (the SAD's arc42 vocabulary, the engine's
  documented terms) — the L1 case is its worked example.
- **`content-scope` rides its existing home as far as possible:**
  the K1 wiring-boundary and K2 single-editor rules already live in
  the knot domain (`knot-design`'s *Behaviour-in-Skill — the Wiring
  Boundary*, the knot-lint K1/K2 checks). `knot-standards` states the
  full content-placement standard (single normative home, one editor
  per shared document **act-based**, one-hop pointers) and names the
  existing K1/K2 home as the rig's mechanical face — inside the knot
  project, so the pointer stays in-bounds.
- **Deliberately a duplicate in the user's sense, bounded:** the
  agents-repo face-standards and `knot-standards` carry the same two
  rulesets for **different bounded contexts** (the agents repo's
  skill/document ecosystem vs the rig's normative documents). Drift
  between them is expected and legal — each is the single normative
  home for its own context; neither may cite the other. Provenance:
  one line in the skill ("standard first authored in the agents repo,
  plans 012/013, 2026-11-08; ported self-contained for the knot
  project — the agents copies are not the source of this skill's
  behaviour").

**The four hooks** — one canonical block, identical in all four
skills (a fixed instruction block; the rules live in
`knot-standards`, so the identical copies restate nothing). The hook
points at the **in-project** home only:

```markdown
## Compliance — vocabulary and content scope

Authoring or modifying the rig's normative documents — knot bodies,
profiles, agent-event documents, interface documents, and the two
instruction documents — is subject to the two face-standards as
stated for the rig in [knot-standards](../knot-standards/SKILL.md):
the bounded-context vocabulary rules (the definer set, first-use
declaration, the polysemy rule) and the content-placement rules
(single normative home, one editor per shared document — act-based —
and one-hop pointers). Read `knot-standards` and follow it. This
note points; it does not restate. The rules live there.
```

**Placement** (per skill, the authoring-time point of the skill):

| Skill | Version bump | Hook placement |
|-------|--------------|----------------|
| `knot-design` | 1.8.0 → 1.9.0 | beside *Terminology in Knots, Profiles, and the Rig* and *Behaviour-in-Skill — the Wiring Boundary* (the two sections the vocabulary/content rules are the named form of) |
| `knot-create` | 5.12.0 → 5.13.0 | after *Core Philosophy*, before the domain-model/workflow sections |
| `knot-software-factory` | 1.2.0 → 1.3.0 | at *The interface documents* / *Building a factory for a project* (the consumer-facing authoring surface) |
| `knot-template` | 3.1.0 → 3.2.0 | at *Procedure* (the two instruction documents are its authored surface) |

The hook block may be trimmed per skill to name that skill's own
artifacts — **never extended** with rule content from
`knot-standards` (a restatement is the Rule-1 defect the hook exists
to prevent). A hook copy carries **no path outside the knot
project**.

## Out of scope

- **The agents-repo face-standards** — untouched by this plan (they
  serve the agents repo's own context; the companion set for the
  project-domain skills `project-sad`, `project-architecture-
  decision-record`, `architecture-init` + the `software-architecture`
  standard was applied directly in those repos, 2026-11-08).
- **`knot-lint` / `project-lint-files`** — the mechanical verification
  side; the rig-side verification-surface pane (rig plan 22) owns it.
  `knot-standards` names knot-lint's K1/K2 as the rig's mechanical
  face but adds no checks to it.
- **`knot` master routing** — the owner scoped this to the
  sub-skills; `knot-standards` is reached via the four hooks, not a
  routing row. (If a row is wanted later, it is a one-line follow-up.)
- **`knot-abstractions`** — read-side reference; at most a
  cross-reference pointer (deferred).
- **The rig-side rectification** of the 007/008/009 documents
  themselves — rig plan 22's own scope.

## Phases

1. **Draft `knot-standards` v1.0.0.** Port the two rulesets, self-
   contained, with the rig bounded-context adaptation (definer set,
   the L1 polysemy worked example, the advisory sweep + report
   convention inline, the provenance note, the in-bounds pointer to
   knot-lint's K1/K2 as the mechanical face). Owner review of the
   draft — this skill is the new normative home; its content is the
   review surface.
2. **Apply the four hooks** at the placements above; bump the four
   versions; commit per the skill-change convention (one
   `docs(skills)` commit covering the new skill + the four hooks +
   version bumps, per the plan 103 precedent — commit 0898690).
3. **Verify.** (a) `knot-standards` passes both face-standards' own
   sweep over itself (zero unbounded terms, zero restatement, one-
   hop pointers only — the Phase-1 validation shape from the agents
   runs). (b) Each of the four skills cites `knot-standards` and
   restates none of its rules (a content-scope sweep over the four:
   zero restatements, zero write-claims on `knot-standards` as
   editor, one-hop pointers). (c) **Boundary check:** no normative
   reference in any of the five files reaches outside the knot
   project (the agents-repo paths appear, if at all, only in the
   provenance note, labelled as provenance). (d) A standalone-
   checkout check: the five files resolve every normative pointer
   with the agents repo absent.

## Notes

- **Why a duplicate instead of a pointer:** a normative pointer out
  of the knot project makes the knot skills' behaviour depend on
  another project's tree — the standalone boundary forbids it. The
  duplicate is the cost of the boundary; the drift cost is bounded
  because the two contexts are disjoint (an agents-repo rule change
  does not obligate a `knot-standards` change, and vice versa — each
  context's defects drive its own home).
- **Sync is not a goal.** If the agents-repo face-standards and
  `knot-standards` diverge, neither is wrong: they standardise
  different document sets. The shared *shape* of the rules (the two
  faces, the three-leg term test, the act-based editor rule) is the
  stable part; the term classes and definer sets are context-local.
- **Precedent:** the same self-containment logic is stated in the
  agents repo's `skills` master (boundary 3: cross-domain names as
  provenance, never as the source of a skill's behaviour) and in
  agents plan 012/013's plan-notes convention. This plan mirrors it
  for the knot project's boundary.
