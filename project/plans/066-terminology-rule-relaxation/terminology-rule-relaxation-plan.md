# Plan: Relax the Knot Terminology Rule

> **Status:** 📝 Draft
> **Created:** 2026-07-29
> **Analysis input:** `terminology-rule-impact-report.md` (copied into this plan directory from `borrow-my-stuff/project/reports/`)

---

## Problem

The `knot-design` skill enforces a strict rule — **Never Leak Internal
Terminology Into Prompts or Event Descriptions** — that forbids the use of
Knot-specific terms (`strand`, `tie-off`, `knot`, `loom`, `event`) inside
knot body instructions and `event-description` fields. Authors must
translate these into generic equivalents (`input file`, `final response`,
`task`, `workspace`, `message`).

In practice this rule is **universally violated** across the rig: all 24
knot files use "strand," and all 12 event-consuming knots use "Emitted" in
their `event-description`. The rule creates friction (authors must perform
a mental lookup before writing), harms scope communication ("the
`completion-validator` knot" vs "the validation agent"), and produces
terminological drift (some knots use "strand," others "input file," others
"trigger file").

The rule's stated benefit — portability to other orchestrators — is
theoretical: the rig is deeply coupled to Knot's event injection, dispatch
directory, and tie-off mechanics, none of which would port without
significant rework. Stripping terminology from prompts buys no real
portability.

Full analysis: see [terminology-rule-impact-report.md](./terminology-rule-impact-report.md).

This plan relaxes the rule into a **guideline** that *encourages* knot
terminology **inside the rig** (knot bodies, profiles, `event-description`
fields, tie-offs, loom-logs) — where agents always have the Knot glossary
available via `AGENTS.md` — while still **forbidding** knot terminology in
the project-space documents (`project/`) and in **skills** themselves, which
must remain orchestrator-agnostic per the `knot-abstractions` layering.

## Target

### The relaxed rule (guideline)

Replace the strict prohibition in `knot-design` with a guideline that:

1. **Encourages** knot terminology inside rig-internal documents (knot
   bodies, profiles, `event-description` fields, tie-offs, loom-logs)
   because every agent invocation includes `AGENTS.md` that references the
   Knot glossary — the terms are always defined and understood.
2. **Forbids** knot terminology in:
   - **Skill documents** (`.agents/skills/*`) — skills are orchestrator-
     agnostic per `knot-abstractions`; they describe generic behaviours,
     not Knot-specific wiring.
   - **Project-space documents** (`project/`) — domain/application
     knowledge that must remain portable and domain-pure.
   - **Any shared/reusable artifact** intended for cross-rig or
     cross-orchestrator reuse.
3. Provides a **consistent style reference** (not a prohibition) so authors
   converge on a single vocabulary inside the rig:

   | Term (encouraged inside the rig) | Meaning |
   |---|---|
   | strand | the input file or event that triggers this knot |
   | tie-off | this knot's final output document |
   | knot | this specific task/agent workflow |
   | loom | the domain work area grouping related knots |
   | event | a message a producer knot may emit for consumers |

### What stays unchanged

- Frontmatter field names (`strand-dir`, `event-description`, etc.) —
  parsed by Knot, not delivered to the agent as prompt text.
- The `event-description` field **content** may now use "Emitted"
  naturally (matching the existing 12 event descriptions).
- The `knot-abstractions` four-layer model (rig / profiles / skills /
  application).
- The Knot glossary (`knot-init/knot-glossary.md`) — living reference for
  all knot terms.
- No rig file format changes — existing files are already compliant with
  the relaxed rule (they already use the terminology).
- No Rust source changes.

### Skills to change

| Skill | Change |
|---|---|
| `knot-design` | Replace the strict rule section + template + examples with the relaxed guideline |
| `knot-init` | Ensure the `AGENTS.md` "Rig" section notes that knot terminology is encouraged inside the rig because the glossary is always available |
| `knot-update` | Add a changelog/guidance entry documenting the rule relaxation (no migration — no format change) |

## Existing References

| Artifact | Relevance | Status |
|---|---|---|
| `knot-design` SKILL.md — "⚠️ Never Leak Internal Terminology" section | The rule to relax | To be replaced |
| `knot-design` template + Bad/Good examples | Reinforce the strict rule | To be updated |
| `knot-init` SKILL.md — AGANTS.md step 4b | Generates the Rig section of AGENTS.md; references knot-glossary.md | To be augmented |
| `knot-init/knot-glossary.md` | Living glossary of knot terms | Unchanged |
| `knot-abstractions` SKILL.md | Four-layer model motivating the skill/project boundary | Unchanged |
| `knot-update` SKILL.md | Versioned changelog of document format changes | New guidance entry |
| All 24 knot files + 10 profiles | Already use knot terminology (violating the old rule) | Unchanged — now compliant |

## Verification (no test suite — documentation-only change)

This is a **documentation-only** change to skill `.md` files — no Rust
source, no runtime behaviour, no tests affected. Verification is therefore
consistency-based, not test-based:

- After the change, no skill document in `.agents/skills/` (other than the
  three updated above) contains knot-specific terminology in its prose.
- The updated `knot-design` explicitly scopes the relaxation to rig-internal
  documents and forbids it in skills and project space.
- Updated skills install cleanly to `~/.agents/skills/` (diff verification).

## Phases

### Phase 0: Analysis (reference — already complete)

The [terminology-rule-impact-report.md](./terminology-rule-impact-report.md)
(report copied into this plan directory) documents:
- The current strict rule and its translation table.
- Compliance audit: 24/24 knot files and 12/12 event descriptions violate it.
- Impact analysis: cognitive overhead, scope ambiguity, terminological drift,
  theoretical portability.
- Recommendation: relax to a guideline with scoped exceptions.

**Artifact:** `project/plans/066-terminology-rule-relaxation/terminology-rule-impact-report.md`

### Phase 1: Relax the rule in `knot-design`

Replace the section **⚠️ Never Leak Internal Terminology Into Prompts or
Event Descriptions** (and its "Template for Generic Instructions" +
"Examples" that follow it) with a new section **Terminology in Knots,
Profiles, and the Rig** that:

- States the guideline (encourage inside the rig; forbid in skills and
  project space).
- Provides the consistent style reference table from §Target.3.
- Notes the "Generic Instructions" template is still a valid *option* but no
  longer the only permitted style, and that knot terminology is the
  preferred style inside the rig.
- Updates the Bad/Good examples: the "Bad" example becomes about leaking
  terminology into *skills* or *project docs* (outside the rig), not about
  using it in knot bodies.
- Preserves the cross-reference to `knot-abstractions` and the skill list.

**Artifact:** `.agents/skills/knot-design/SKILL.md`
**Metadata:** bump `version` from `1.3.0` to `1.4.0`.
**Note:** Documentation-only — no Rust code or tests.

### Phase 2: Reinforce rig-internal terminology in `knot-init`

Inspect the `knot-init` AGENTS.md generation (step 4b). **Key finding:**
the current `knot-init` does **not** copy a terminology table into
`AGENTS.md` — it reads `knot-glossary.md` and explicitly avoids writing a
glossary section into `AGENTS.md` (the glossary lives in the skill and is
always available to agents with Knot skills installed). This is already the
right design for the relaxed rule, so there is no table to remove.

Change: augment the generated `AGENTS.md` **Rig** section (the content
appended in step 4b) with one sentence so that every rig's AGENTS.md makes
the relaxation explicit:

> Knot terminology (`strand`, `tie-off`, `knot`, `loom`, `event`) is
> encouraged inside rig files — the Knot glossary is installed as a skill
> and is always available, so these terms are always defined. Keep this
> terminology out of skill documents and project-space documents.

**Artifact:** `.agents/skills/knot-init/SKILL.md` (the AGENTS.md template
content in step 4b).
**Metadata:** bump `version` from `3.3.0` to `3.4.0`.
**Note:** Documentation-only — no Rust code or tests.

### Phase 3: Document the change in `knot-update`

Add a new entry at the top of the `knot-update` Changelog documenting the
terminology rule relaxation. Because this is a **skill-guidance change**
(not a document-format change), the entry is a "guidance note" with no
migration steps and no format table — matching the report's conclusion that
no rig file changes are needed:

> ### Guidance — Terminology Rule Relaxed (skill version 1.4.0)
>
> The `knot-design` skill relaxes its "Never Leak Internal Terminology"
> rule from a strict prohibition to a guideline. Knot-specific terms
> (`strand`, `tie-off`, `knot`, `loom`, `event`) are **encouraged** in
> knot body instructions, profiles, `event-description` fields, and
> tie-offs — because every agent invocation includes `AGENTS.md`
> referencing the Knot glossary, these terms are always defined.
>
> **No migration required** — the rule was universally violated in
> practice (all 24 knot files and 12 event descriptions already used
> knot terminology). Relaxing the rule makes existing practice compliant.
>
> **Scope boundary preserved:** knot terminology remains **forbidden**
> in skill documents (`.agents/skills/*`) and project-space documents
> (`project/`). These remain orchestrator-agnostic per the
> `knot-abstractions` layering.
>
> **Affected documents:** none — no frontmatter fields or body semantics
> change. Only authoring guidance changes.

**Artifact:** `.agents/skills/knot-update/SKILL.md` (Changelog section).
**Metadata:** bump `version` from `1.4.0` to `1.4.1` (guidance-only
version bump).
**Note:** Documentation-only — no Rust code or tests.

### Phase 4: Publish and verify

1. Install the three updated skills globally (idempotent, with verification):
   ```bash
   for skill in knot-design knot-init knot-update; do
     cp -r .agents/skills/$skill ~/.agents/skills/$skill
   done
   for skill in knot-design knot-init knot-update; do
     diff .agents/skills/$skill/SKILL.md \
          ~/.agents/skills/$skill/SKILL.md > /dev/null 2>&1 && \
        echo "$skill: OK" || echo "$skill: FAILED"
   done
   ```
2. Confirm the relaxed guideline is present and scoped:
   ```bash
   grep -n "encouraged inside" .agents/skills/knot-design/SKILL.md
   grep -n "forbidden"        .agents/skills/knot-design/SKILL.md
   grep -n "encouraged inside rig" .agents/skills/knot-init/SKILL.md
   ```
3. Confirm no rig file changes are expected — existing knot bodies and
   profiles already use the terminology the relaxed rule encourages.

## Cross-Reference

- **knot-abstractions skill** — four-layer model (rig / profiles / skills /
  application) that defines the boundary the relaxed rule respects.
- **knot-design skill** — the rule being relaxed (Phase 1).
- **knot-init skill** — AGENTS.md generation + glossary reference (Phase 2).
- **knot-update skill** — changelog of document/skills changes (Phase 3).
- **terminology-rule-impact-report.md** — analysis copied into this plan
  directory (Phase 0 reference).
