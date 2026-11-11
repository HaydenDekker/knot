# ISSUE-001 — Event payload contracts have no declared home: `knot-design` never states that the **subscriber's** `event-description` frontmatter is the single normative home, and its style example frames emission producer-side ("if a producer knot should know about your output")

**Status:** OPEN — raised 2026-11-11 (owner ruling recorded the same day as rig standards-gap register reg-003); rectification pass awaits owner dispatch.
**Raised:** 2026-11-11
**Raised by:** the user, closing rig plan 23 reg-003 (software-factory-rig `project/standards-gaps.md`) — the ruling walk after plan 23 closed.
**Landing target:** the `knot-design` skill (agents repo `skills-library/knot-design`, currently v1.8.0) — the standard statement; plus a `knot-lint` check candidate (§5); the rig-side body sweep is tracked by reg-003 in the rig, not here.
**Severity:** medium — no runtime cost today, but the gap is a **K2 single-editor breach with no yardstick**: payload contracts are restated where events are produced and consumed (measured: the `PlanScoped` key list appears three times across the rig), every restatement can drift, and `knot-design` — the normative source for the wiring boundary — contains no rule either way.

---

## 1. The ruling (owner, 2026-11-11) — narrow version

1. **The subscriber declares the event.** `strand-dir` + `event-description`
   frontmatter is the **single normative home** of what an event carries. It
   is injected into the subscriber's prompt, so the consumer always has the
   contract in guaranteed context — the event is defined by what its
   consumer expects.
2. **The producer never knows the consumers.** A producer knot names the
   event it emits (a wiring verb — legal); it never enumerates payload keys
   "for" consumers or describes consumer expectations.
3. **No knot or profile body restates an event payload contract.** Bodies
   may use event *names* and the read/emit verbs — knot-design's existing
   encouraged-terminology rule stands — but a payload/key list in a body is
   a finding: the body says "read the strand" / "emit `X`", nothing more.
   (Narrow: contract restatement is the defect class. Event terminology and
   wiring verbs in bodies stay legal.)

## 2. Measured hits

**The skill (`skills-library/knot-design/SKILL.md` v1.8.0):**

- **L78 — the style example contradicts the ruling by framing:**
  "Emit an event in your tie-off **if a producer knot should know about
  your output**." Events are not framed by producer-to-producer knowledge;
  they exist because a subscriber declared interest. This sentence teaches
  the producer-knows model.
- **Wiring boundary section** ("It declares exactly three things …
  Wiring — `agent-profile-ref`, `strand-dir`, event subscriptions") never
  says the `event-description` is *the* home of the payload contract, nor
  that bodies must not restate it. The measurable-boundary table bounds
  prose volume but says nothing about contract duplication.

**The rig (software-factory-rig, 2026-11-11 — evidence, fixed by the reg-003 sweep, not by this issue):**

- `strategy-loom/strategy-on-gap.md` states the `PlanScoped` key list
  **twice in one file** (fast path + full task: `cycle, target, ci-tags,
  uat-cells, prd, scope, architectural-considerations`).
- `author-loom/plan-author.md` step 4: "Emit `PlanAuthored` with plan facts
  `(slug, file, phase structure, commit, scope)`" — a producer-side payload
  list riding in the body.
- `strategy-loom/strategy-on-request.md` step 1 enumerates the seven
  delivery-context fact keys in the body.
- Ledger: plan 23 `units.md` PA-K2d, PS-R6c, PS-G4 (+ coarse VD-K1/TP-K3);
  register: reg-003.

## 3. Why it matters

1. **Drift without an owner.** Three statements of one contract, no doc
   authoritative — the exact K2 condition the rig's lint stands against,
   with no standard to lint *against*.
2. **Plan 24 builds on this.** The author-loom split (rig plan 24) designs
   new events (`PlanPhased` fan-out + set-complete join); its payload
   declarations need the subscriber-home convention stated *before* the
   design, not after.
3. **The example teaches the wrong model.** L78 is the skill's one
   load-bearing "Good" example — a designer following it frames every new
   event producer-side.

## 4. What is NOT in scope

- The rig-side body sweep (key-list deletions in the three files above) —
  reg-003's own action under the ruling, tracked in the rig.
- The **broad** rule ("no event content of any kind in bodies") — explicitly
  ruled *out* 2026-11-11; naming verbs and wiring prose stay legal.
- `knot-template`'s Event Content Guide — it already ties extra fields to
  "the shape the description asks for"; consistent with the ruling. Verify,
  don't rewrite.
- ISSUE-004 (harness) — the skills-terminology direction (Knot terms leaking
  *out* of the rig); this issue is the rig's own contract-home gap.

## 5. Remediation (owner dispatch)

1. **`knot-design` v1.8.0 → v1.9.0** (agents repo; skill-change convention:
   `version:` bump + `knot-design vX.Y.Z: <change>` commit):
   - re-cut the L78 example to the subscriber-declared framing;
   - add the rule under *Behaviour-in-Skill — the Wiring Boundary*: the
     subscriber `event-description` is the single normative home of the
     payload contract; producer bodies name the emit, never the payload;
     no body (knot or profile) restates keys — "read the strand".
2. **Coordination — knot plan 104** (`104-skill-face-standard-compliance`)
   already targets `knot-design` 1.8.0 → 1.9.0 with hooks beside the two
   sections this issue edits. Land as **one** amendment pass, or sequence
   deliberately — the version bump happens once.
3. **Candidate follow-up (separate ruling):** a `knot-lint` check —
   an event name carrying a parenthetical/key-list payload statement in a
   knot/profile body is a K2-class finding. Cheap to detect; makes the
   ruling self-enforcing.

**Verification pane:** re-scan the rig (event key lists in bodies: zero);
`knot-design` states the home + the two prohibitions; example carries no
producer-knows framing; plan 104's pass stays conflict-free.

## 6. Dependencies / relations

- **Rig plan 23 reg-003** (`software-factory-rig/project/standards-gaps.md`)
  — this issue is its skill-side vehicle; the ruling is recorded there.
- **Rig plan 24** (`author-loom-plan-phase-split-2026-11-10.md`) — consumes
  the convention for the `PlanPhased` design; dependency now runs the other
  way: 24's design should follow the ruling, not wait on the skill edit.
- **Knot plan 104** — same skill, same target version; coordinate the pass.
