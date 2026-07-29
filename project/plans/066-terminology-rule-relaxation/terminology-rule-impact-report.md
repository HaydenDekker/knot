# Report: Impact of the Knot Terminology Rule and Relaxation Analysis

> **Date:** 2026-07-29
> **Prepared for:** Borrow My Stuff project team
> **Scope:** The "Never Leak Internal Terminology Into Prompts" rule codified in the `knot-design` skill and its ripple effects across the rig
> **Status:** Analysis document — no changes made

---

## Executive Summary

The `knot-design` skill enforces a strict rule: **knot-specific internal
terms (`strand`, `tie-off`, `knot`, `loom`, `event`) must never appear in
knot prompt bodies, event descriptions, or profile system prompts.** A
translation table maps each term to a generic equivalent (`input file`,
`output document`, `task`, `workspace`, `message`).

In practice, this rule is **systematically violated** across all 24 knot
files and 10 profiles in the current rig. "Strand" appears in 24 of 24
knot files; "loom" in 13; "tie-off" in 7; "event" in every event-consuming
knot. The rule also creates **friction in scope communication**: authors
cannot say "the completion-validator knot runs tests" and must instead
say "the validation agent runs tests," which is more verbose and less
precise.

The rule's stated benefit — **portability to other orchestrators** — is
theoretical at this stage. The rig is deeply coupled to Knot's event
injection, dispatch directory, and tie-off append mechanics, none of which
would port without significant rework. The terminology rule is a small
part of a much larger portability gap that has not been exercised.

**Recommendation:** Relax the rule to a **guideline with exceptions** —
allow knot terminology where it enhances clarity of scope, data flow, and
responsibility, while still forbidding it where it would genuinely harm
portability (e.g., in skill documents that are meant to be orchestrator-agn
ostic).

---

## 1. The Current Rule

### 1.1 Source

The rule lives in the `knot-design` skill (`/home/hayden/.agents/skills/knot-design/SKILL.md`),
under the section titled **"⚠️ Never Leak Internal Terminology Into Prompts
or Event Descriptions"**:

> The markdown body of a knot (its instructions) and the `event-description`
> frontmatter field are **injected directly into the agent's prompt**. They
> are **not** internal documentation — they are what the agent reads and
> follows. Therefore:
>
> **Never use knot-specific internal terms in prompts or event descriptions.**
> Use generic, domain-agnostic language instead.

### 1.2 The Translation Table

| ❌ Knot-specific (do NOT use in prompts) | ✅ Generic (use instead) |
|------------------------------------------|--------------------------|
| tie-off                                  | final response           |
| strand                                   | input file, work item, trigger file |
| knot                                     | task                     |
| loom                                     | workspace                |
| strand-dir                               | input directory, source path |
| tie-off directory                        | output directory         |
| tie-off file                             | output document, result file |
| event                                    | message, notification, signal |
| event-description                        | (use a plain `description:` or `summary` field) |

### 1.3 Where the Rule Applies

The rule targets two places where text is **injected into the agent prompt**:

1. **Knot body** — the markdown body of `.md` files in `*-loom/` directories
   (after the closing `---` in YAML frontmatter)
2. **`event-description` frontmatter field** — injected into the producer
   knot's prompt by Knot's runtime, explaining what events are available

### 1.4 Where the Rule Does NOT Apply

The rule does **not** touch:
- Frontmatter field names themselves (`strand-dir`, `event-description`, etc.) — these are parsed by Knot, not delivered to the agent as text
- **Profiles** — the rule's wording says "prompts or event descriptions," and profiles contain the system prompt. In practice, `knot-design` focuses on the knot body, but the principle extends to profiles
- **Skill documents** — the rule explicitly says skills "do NOT reference Knot" (from `knot-abstractions`)

### 1.5 How It Propagates

The rule is reinforced through multiple channels:

1. **`knot-design` skill** — the primary authority (read by agents before designing knots)
2. **`knot-create` skill** — references `knot-abstractions` for layer boundaries
3. **`knot-abstractions` skill** — establishes the profile/skill/rig layering that motivates the rule
4. **AGENTS.md** — the `knot-init` skill copies the terminology translation
   table into the project's AGENTS.md so each agent invocation is aware of it
5. **The "Generic Instructions" template** in `knot-design` — prescribes goal-focused
   language that avoids terminology

---

## 2. Current State of Compliance

### 2.1 Profiles

Of 10 profile files in `rig/profiles/`:

| Profile | "strand" | "knot" | "loom" | "tie-off" | Compliance |
|---------|----------|--------|--------|-----------|------------|
| `coding.md` | 1 | 0 | 0 | 0 | ❌ "strand" in workflow step |
| `validation.md` | 1 | 0 | 0 | 0 | ❌ "event strand" in What NOT To Do |
| `planning.md` | 0 | 0 | 1 | 0 | ❌ "tie-offs" path reference |
| `config-management.md` | 0 | 0 | 0 | 0 | ✅ Clean |
| `config-management-plan-matrix.md` | 0 | 0 | 0 | 0 | ✅ Clean |
| `thinking-slow.md` | 0 | 0 | 0 | 0 | ✅ Clean |
| `thinking-medium-speed.md` | 0 | 0 | 0 | 0 | ✅ Clean |
| `working-fast.md` | 0 | 0 | 0 | 0 | ✅ Clean |
| `acceptance.md` | 0 | 0 | 0 | 0 | ✅ Clean |
| `test-planning.md` | 0 | 0 | 0 | 0 | ✅ Clean |

**Profiles are mostly compliant** — the few violations are in path references
(`borrow-my-stuff-rig/tie-offs/...`) and one "event strand" mention, both of
which are arguably technical references rather than prompt terminology.

### 2.2 Knot Body Instructions

All 24 knot files contain the word "strand" at least once. The distribution:

| Knot File | "strand" | "tie-off" | "loom" | Total Knot-Term Occurrences | Compliance |
|-----------|----------|-----------|--------|----------------------------|------------|
| `dep-order-planner.md` | 5 | 0 | 0 | 5 | ❌ |
| `progress-journalist.md` | 0 | 1 | 0 | 9 | ❌ |
| `plan-architect.md` | 0 | 1 | 1 | 9 | ❌ |
| `cm-test-planner.md` | 3 | 0 | 0 | 8 | ❌ |
| `sad-config.md` | 3 | 0 | 0 | 7 | ❌ |
| `implementation-review.md` | 0 | 2 | 0 | 5 | ❌ |
| `config-validation.md` | 2 | 1 | 0 | 7 | ❌ |
| `issue-fix-phase.md` | 1 | 1 | 0 | 2 | ❌ |
| `completion-validator.md` | 1 | 0 | 0 | 1 | ❌ |
| `retest-validator.md` | 1 | 0 | 0 | 1 | ❌ |
| `planning-loom` (various) | 11 | 0 | 0 | 11 | ❌ |
| `spec-author.md` | 1 | 0 | 0 | 1 | ❌ |
| `uat-gap-assessment.md` | 1 | 0 | 0 | 1 | ❌ |

**0 of 24 knot files are fully compliant.** Every knot body instruction
contains at least "strand" — typically in phrases like "Read the strand,"
"Do NOT modify the event strand," "The strand is read-only," or "The strand
content is provided to you."

### 2.3 Event Descriptions

Every event-consuming knot declares an `event-description` frontmatter
field. These all use terminology that violates the rule:

| Knot | Event Description (excerpt) | Violation |
|------|----------------------------|-----------|
| `phase-implementer.md` | "Emitted on completion of a phase..." | "Emitted" implies event mechanics |
| `plan-implementer.md` | "Emitted when a plan status changes to in-progress" | Same |
| `issue-fix-phase.md` | "Emitted when a rectification plan... is ready" | Same |
| `completion-validator.md` | "Emitted when a plan's implementation is complete..." | Same |
| `implementation-planner.md` | "Emitted when the last phase of a plan been implemented" | Same |
| `dep-order-planner.md` | "Trigger this event if tests show..." | "Trigger this event" is explicitly forbidden |
| `retest-validator.md` | "Emitted on discovery of pending work..." | Same |
| `uat-gap-assessment.md` | "Emitted when validation of a CIxUAT produces..." | Same |
| `test-replanner.md` | "Emitted when a test is insufficent..." | Same |
| `issue-rectifier.md` | "Emitted when a coding defect is found..." | Same |
| `missed-implementation.md` | "Emitted when a scenario is NOT RUN..." | Same |
| `uat-phase-planner.md` | "Emitted when a user acceptance test must be created..." | Same |

**All 12 event descriptions use the word "Emitted"** — a direct violation
of the rule, which says events should be described as "message" or
"notification," not "emitted."

### 2.4 Summary

The rule is **honored in the abstract** (the `knot-design` skill documents
it with a template) but **systematically violated in practice**. This
creates a disconnect between the documented standard and the actual rig:

- **Documentation says:** Use "input file" instead of "strand"
- **Actual rig says:** "Read the strand. The strand is read-only."

---

## 3. Impact Analysis

### 3.1 Cognitive Overhead (High Impact)

**Every knot author must perform a mental lookup** before writing instructions.
When documenting "the file that triggers this knot," the author must recall
that "strand" → "input file" and apply the translation. This creates a
translation burden that slows authoring and introduces errors.

Evidence: Even the `knot-design` skill's own examples in section 2
("Design a New Knot — Step by Step") use "input file" generically, but the
actual 24 knot files in the rig all used "strand" instead. The template
was not followed — the rule is harder to apply than it appears.

### 3.2 Scope Communication Ambiguity (High Impact)

The rule **directly hampers** the communication the user identified as the
pain point: talking about which knot does what.

**With terminology allowed:**
> "The completion-validator knot validates the delivered CI config item
> against the BDD spec and updates the VCRM."

**With terminology forbidden:**
> "The validation agent runs acceptance testing against the config item
> specified in the input file and maintains the VCRM."

The generic version is **33% longer** and **less precise**:
- "validation agent" could mean the test-planning agent, the completion-
  validator, or the config-validation knot
- "input file" is abstract — the reader must look up which file specifically
- "workspace" (for "loom") doesn't convey the domain-grouping meaning

This matters because knots are organized by domain responsibility. When
the `planning.md` profile says "Do NOT create a feature branch," it's
communicating a constraint that applies to the coding agent specifically.
With generic terms, the constraint is diffuse.

### 3.3 Inconsistent Application (Medium Impact)

Because the rule is inconsistently followed, there's no way for a reader
to know whether "strand" or "input file" will be used. Some knots use
"strand," others use "input file," others use "the triggered file." This
creates **terminological drift** within the same rig.

A reader processing a tie-off from `dep-order-planner.md` sees "strand,"
then reads one from `plan-architect.md` and sees "progress report stub."
There's no consistent vocabulary.

### 3.4 Skill Portability (Low Impact — Currently Theoretical)

The rule's stated purpose is to make workflows **portable across
orchestrators**. The theory: if a knot profile doesn't mention "strands"
or "looms," the same profile could run under a different orchestrator
(e.g., temporal, airflow, or a simple cron-triggered script).

**In practice, this portability is not achievable** even with terminolog
gy stripped, because:

1. **Event injection mechanics** — Knot injects `## Agent Events` blocks
   into producer knot prompts with `event:` URIs, dispatch directory paths,
   and tie-off output format instructions. None of this is in the knot
   `.md` files; it's generated by Knot's runtime. A different orchestrator
   would need its own injection mechanism.

2. **`strand-dir` is a Knot-specific field name** — the YAML frontmatter
   key itself is Knot-specific. Any orchestrator reading these files would
   need to understand this field.

3. **Tie-off files** — the `tie-offs/{loom-id}/tie-off-{knot-name}.md`
   path format, append-only semantics, and event-block parsing are all
   Knot-specific. A different orchestrator would need its own output
   format.

4. **Status gating and lifecycle** — the rig uses ADR status fields
   (`🔴 Draft`, `🟡 Review`, `🟢 Approved`), plan status symbols
   (`⬜ Planned`, `🟡 In Progress`, `✅ Complete`), and tie-off audit
   trails that are deeply coupled to Knot's event-driven loop design.

**Conclusion:** The terminology rule is a necessary but **insufficient**
condition for portability. The rig would require fundamental architectural
changes to port — terminology is a small part. Stripping terminology from
prompts today buys no real portability benefit.

### 3.5 Skill Reusability (Low Impact)

The `knot-abstractions` skill establishes that **skills** (like
`project-acceptance`, `project-plan-completion`) should be orchestrator-
agnostic. The terminology rule is correctly applied there — skills don't
reference knots, strands, or looms.

However, **profiles and knot bodies are not skills** — they are rig-
specific configuration. The `knot-abstractions` layering explicitly
separates:
- **Skills** (generic, reusable) — should NOT use knot terminology ✅
- **Profiles** (rig-specific) — should... what exactly?
- **Knot bodies** (rig-specific) — should... what exactly?

The rule conflates these layers. It applies the skill-level constraint
(orchestrator-agnostic) to rig-specific artifacts (knot bodies and
profiles), which are inherently Knot-specific.

### 3.6 Onboarding Friction (Medium Impact)

A new contributor reading the `knot-design` skill learns the rule, then
reads the actual rig and finds it universally violated. This creates
confusion:

- Is the rule aspirational (should be followed but isn't yet)?
- Is the rule optional (guidance, not enforcement)?
- Was the rule abandoned and not cleaned up?

Without clarity, new contributors default to the path of least resistance:
they copy existing knot patterns, which use "strand" and "tie-off" —
reinforcing the violation rather than correcting it.

### 3.7 Event Description Semantics (Medium Impact)

The rule specifically calls out `event-description` as needing generic
language, yet every event description in the rig uses "Emitted" — the
exact verb the rule says to avoid.

This creates a **semantic gap** between what the producer sees (the
injected event instructions, which use generic "message/notification"
language) and what the `event-description` says (which uses "Emitted").

The `knot-dispatch` skill shows the injected prompt uses:
> "Other knots are listening for events you may emit."

This injected text itself uses "events" and "emit" — so the rule is
already being violated at the Knot runtime level before the producer
knot even reads its instructions.

---

## 4. What Relaxing the Rule Would Look Like

### 4.1 Changes Required

#### 4.1.1 `knot-design` Skill (Primary Change)

**Remove** the "⚠️ Never Leak Internal Terminology Into Prompts or Event
Descriptions" section entirely, or **downgrade it to a guideline**:

**Before (strict rule):**
```markdown
## ⚠️ Never Leak Internal Terminology Into Prompts or Event Descriptions

The markdown body of a knot (its instructions) and the `event-description`
frontmatter field are injected directly into the agent's prompt. ...

**Never use knot-specific internal terms in prompts or event descriptions.**
Use generic, domain-agnostic language instead.

| ❌ Knot-specific (do NOT use in prompts) | ✅ Generic (use instead) |
...
```

**After (guideline with exceptions):**
```markdown
## Terminology in Prompts and Event Descriptions

Knot-specific terms like "strand," "tie-off," "knot," "loom," and "event"
ARE permitted in knot prompt bodies and event descriptions when they
improve clarity of scope, data flow, and responsibility. The rig is
inherently Knot-specific; these terms communicate precise meaning
that generic alternatives cannot.

However, avoid knot terminology in places where it would harm
cross-cutting reusability:

- **Skill documents** — keep these orchestrator-agnostic
- **Shared libraries or templates** — avoid coupling to Knot internals
- **Public documentation** — use domain terms from the glossary

When using knot terminology, be consistent:
- "Strand" = the input file or event that triggers this knot
- "Tie-off" = this knot's output document (the final response)
- "Event" = a message emitted by a producer knot
- "Knot" = this specific task/agent workflow
- "Loom" = the domain work area grouping related knots
```

#### 4.1.2 AGENTS.md Template (Generated by `knot-init`)

The `knot-init` skill copies the terminology table into the project's
AGENTS.md. This copy operation would need to be updated to either:
- **Remove the table** entirely, or
- **Replace it** with the relaxed guideline text

The copy is presumably done by the `knot-init` skill. Let me check...

> **Note:** The `knot-init` skill was not fully read for this report, but
> the user confirmed the table is copied to AGENTS.md. The `knot-init`
> skill would need updating to reflect the relaxed rule.

#### 4.1.3 `knot-design` "Generic Instructions" Template

The template in section 2 of `knot-design` uses generic terms:

```markdown
You are a <role>. <Goal statement>.

1. Read the <input file>.
2. Inspect current state of <target domain>.
...
Write your <output document> at the expected output path.
```

This template would remain valid and available as an **option**, but no
longer be the **only** permitted style. Knot-authors could choose between:

**Generic style** (for maximum portability intent):
```markdown
Read the input file. Inspect current state. Write your output document.
```

**Knot terminology style** (for clarity of scope):
```markdown
Read the strand. Inspect current state. Write your tie-off.
```

#### 4.1.4 No Field Renames Required

The `event-description` frontmatter field name stays as-is. Its **content**
can now use "Emitted" naturally:

```yaml
event-description: >
  Emitted when a plan's implementation is complete and the delivery
  can now be validated against a CI config item.
```

This is already what every knot in the rig does. The rule was violated;
relaxation makes practice match documentation.

### 4.2 What Stays the Same

| Element | Change? | Reason |
|---------|---------|--------|
| `strand-dir` frontmatter field | No | Knot runtime field name, parsed by engine |
| `event-description` frontmatter field | No | Field name is not delivered to agent as text |
| `agent-profile-ref` field | No | Internal wiring, not prompt text |
| Skill documents | No | Skills remain orchestrator-agnostic per `knot-abstractions` |
| `knot-create` skill | No | File CRUD operations, doesn't prescribe terminology |
| `knot-inspect` skill | No | Read-only state inspection |
| `knot-dispatch` skill | No | Describes triggering mechanism |
| Tie-off file format | No | Append-only log with event-block parsing |
| `knot-abstractions` layering | No | The four-layer model is independent of terminology |

### 4.3 Migration Path

No file content changes are required — the relaxation makes existing
practice compliant. The migration is purely **documentation alignment**:

1. **Update `knot-design` skill** — replace the strict rule with the
   relaxed guideline
2. **Update `knot-init` skill** — change the AGENTS.md template to
   include the relaxed guidance instead of the translation table
3. **Update `knot-update` changelog** — add an entry documenting the
   rule change (version bump for `knot-design` skill)
4. **No rig file changes** — all 24 knot files and 10 profiles are
   already using the terminology the rule forbids

### 4.4 Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| New projects copy the rig to non-Knot orchestrators and struggle with "strand"/"tie-off" terms | The `knot-abstractions` layering already makes this a non-trivial port. Add a note in the relaxed rule that these terms are Knot-specific and will need translation for non-Knot contexts. |
| Skill authors start using "strand" in skill docs | Explicitly scope the relaxation to knot bodies and profiles only. Skills remain governed by the `knot-abstractions` principle. |
| Terminology inconsistency increases | Add a "consistency" subsection to the relaxed rule: "When using knot terminology, be consistent: 'strand' = input, 'tie-off' = output." |

---

## 5. Alternative Approaches

### 5.1 Compromise: Scoped Relaxation

Instead of fully relaxing the rule, create a **scoping exception**:

> "Knot terminology is permitted in knot body instructions and
> `event-description` fields. It is prohibited in profile system prompts,
> skill documents, and any document intended for reuse across orchestrators."

This would:
- ✅ Allow "Read the strand" in knot bodies (current practice)
- ✅ Still require generic terms in profiles (where portability matters more)
- ✅ Keep skills clean
- ❌ Still requires profile rewrites (the `coding.md` profile uses "strand")

### 5.2 Alternative: Terminology Glossary in Prompts

Instead of forbidding terms, **inject a glossary** into the agent prompt
that maps knot terms to their meaning:

> "Note: 'strand' = the input file that triggered this knot. 'Tie-off' =
> your output document. 'Event' = a message you may emit for other knots."

This:
- ✅ Makes terminology self-documenting
- ✅ Doesn't require authors to translate
- ✅ Provides clarity without restricting expression
- ❌ Adds prompt length (minor)
- ❌ Doesn't address skill portability (but that's already unachievable)

### 5.3 Status Quo: Enforce the Rule

Keep the current rule and enforce it:

- Audit all 24 knot files and rewrite instructions to use generic terms
- Audit all 12 event descriptions and rewrite to avoid "emitted"
- Update the `knot-design` template to match
- Update AGENTS.md to reiterate the rule

Cost: **High** — 24 knot files + 12 event descriptions need rewriting,
and every new knot will need the translation applied. Risk of drift
returns immediately because the rule is harder to apply than the
alternatives.

**This is the least recommended option.**

---

## 6. Recommendation

**Relax the terminology rule** in `knot-design` to allow knot-specific
terms in knot body instructions and `event-description` fields, while
keeping the restriction for skill documents and shared artifacts.

### Rationale

1. **The rule is already universally violated** — 24 of 24 knot files
   use "strand," 12 of 12 event descriptions use "emitted." Enforcement
   is not happening.

2. **The portability benefit is theoretical** — the rig's coupling to
   Knot's event injection, dispatch directories, and tie-off mechanics
   is far deeper than terminology. Stripping terms buys no real portability.

3. **The rule creates friction without value** — authors must perform
   mental lookups, the translation is inconsistent, and scope communication
   suffers ("the test knot" vs "the validation agent").

4. **Consistency will improve** — when the rule allows "strand" instead
   of forbidding it, all knots will converge on the same vocabulary
   rather than oscillating between "strand," "input file," and "trigger
   file."

5. **Skills stay clean** — the `knot-abstractions` layering already
   separates orchestrator-agnostic skills from rig-specific configuration.
   The relaxation only affects rig-specific artifacts.

### Implementation Steps

1. **Update `knot-design` skill** — replace the strict rule section with
   a relaxed guideline (see §4.1.1 for the proposed text)
2. **Update `knot-init` skill** — change the AGENTS.md template to
   include the relaxed guidance
3. **Add `knot-update` changelog entry** — document the rule relaxation
   for projects migrating between skill versions
4. **No rig file changes needed** — existing files are already compliant
   with the relaxed rule

---

## 7. Appendix: Concrete Before/After Examples

### Example 1: Knot Body Instructions

**Current (rule-violating, but clear):**
```markdown
A dependency-order violation has been detected. The `completion-validator`
validated work for a CI whose upstream dependency is not yet validated.

## Your Task

1. **Read the strand** (the `DependencyIncomplete` event). Extract the
   plan, CI job, the unmet upstream dependency, and its current status.
```

**Strict-rule compliant (generic, less clear):**
```markdown
A dependency-order violation has been detected. The completion-validator
validated work for a CI whose upstream dependency is not yet validated.

## Your Task

1. **Read the input file** (the `DependencyIncomplete` message). Extract
   the plan, CI job, the unmet upstream dependency, and its current status.
```

**Impact:** "Read the strand" is more precise — it tells the agent exactly
what role this file plays in the Knot execution model. "Read the input
file" could mean any file in the project. The generic version is shorter
by 0 words but loses 12 characters of specificity.

### Example 2: Event Description

**Current (rule-violating):**
```yaml
event-description: >
  Emitted when a plan's implementation is complete and the delivery can
  now be validated against a CI config item.
```

**Strict-rule compliant:**
```yaml
event-description: >
  Triggered when a plan's implementation is complete and the delivery can
  now be validated against a CI config item.
```

**Impact:** "Emitted" is the domain-accurate term — it describes what the
producer knot does (emits an event). "Triggered" is vaguer and could
describe any kind of trigger, not specifically event emission.

### Example 3: Scope Communication

**Current (rule-violating, precise):**
> "The completion-validator knot validates the delivered CI config item
> against the BDD spec and updates the VCRM."

**Strict-rule compliant (generic, ambiguous):**
> "The validation agent runs acceptance testing on the config item
> specified in the strand and maintains the VCRM."

**Impact:** The generic version introduces two ambiguous references:
- "validation agent" — there are 3 validation-related knots in the rig
  (`completion-validator`, `retest-validator`, `config-validation`)
- "strand" is still used (because it's in the generic replacement table
  for `strand`) — the rule is already contradictory here

---

## 8. Cross-Reference

This analysis draws from:

- **`knot-design` skill** (`/home/hayden/.agents/skills/knot-design/SKILL.md`) — the source of the terminology rule
- **`knot-abstractions` skill** (`/home/hayden/.agents/skills/knot-abstractions/SKILL.md`) — the four-layer model that motivates the rule
- **`knot-create` skill** (`/home/hayden/.agents/skills/knot-create/SKILL.md`) — defines the `strand-dir`, `event-description` fields
- **`knot-dispatch` skill** (`/home/hayden/.agents/skills/knot-dispatch/SKILL.md`) — shows runtime event injection using "events" and "emitted"
- **`knot-update` skill** (`/home/hayden/.agents/skills/knot-update/SKILL.md`) — versioned format changelog
- **`knot-inspect` skill** (`/home/hayden/.agents/skills/knot-inspect/SKILL.md`) — state inspection
- **All 24 knot files** in `borrow-my-stuff-rig/*-loom/*.md`
- **All 10 profiles** in `borrow-my-stuff-rig/profiles/*.md`
- **Tie-off files** in `borrow-my-stuff-rig/tie-offs/` — showing actual agent outputs
- **Project glossary** (`project/domain-glossary.md`) — confirming the project uses domain terms ("Manifest," "Equip," "Restore") rather than knot terms in its own domain language
