# Plan: Knot Lint — Static Rig Consistency Audit, with the Behaviour-in-Skill Rule

> **Status:** in progress — phases 0, 1 and 2 complete (2026-10-23),
> phase 3 pending. Phase 0: the rule text and the measurable boundary
> (prose/numbered-step thresholds) are live in `knot-design` and
> `docs/design-guide.md`. Phase 1: the lint engine landed at
> `.agents/skills/knot-lint/scripts/knot-lint.py` (Python 3 stdlib,
> read-only) with the fixture rigs (`references/fixtures/`), the golden
> finding set, and `--selftest` (green: good rig 0 findings, bad rig 8
> findings, exact match); the thresholds were calibrated against the
> fixture rigs and the two live calibration rigs — no threshold had to
> move (the knot repo's rig: wiring-only knots at 2 words of prose; the
> software-factory rig: rig-document floor at 158 measured words, 33
> documents over the knot boundary). Phase 2: the first dated report is
> committed at `project/lint/knot-lint-2026-10-23.md` (software-factory
> rig: 37 findings — K1 x33, K2 x3, K5 x1; K3/K4 clean. This repo's
> rig: 0 findings). The 2026-10-22 baseline's shared-editor
> `discoveries.md` and the format-bootstrap circularity were both
> rectified in the software-factory rig between baselines.

## Problem

Knot definitions currently carry full operational behaviour inline. In the
software-factory rig, loom knots and profiles run 134–232 lines of
procedural prose (e.g. `test-planning-loom/coding-test-issue-logger.md`,
165 lines of write contract); the demo rig's knots are 7-line wiring files.
The consequences, observed 2026-10-22:

1. **Behaviour is defined in several places at once, with duplicated write
   contracts.** `project/coding/discoveries.md` is referenced by four rig
   documents; two of them (`profiles/coding.md`,
   `strategy-loom/strategy-on-gate-failure.md`) carry independent edit
   rights over it. A workspace-wide measurement found **14 project
   documents each referenced by ≥2 rig documents** (`master-plan.md` by 13,
   `VCRM.md` by 9). Most are readers (healthy — the fact cannot fork), but
   nothing distinguishes readers from editors, and nothing flags the
   shared-editor cases.
2. **Behaviour defined inline in a knot cannot be versioned, audited, or
   reused the way skill behaviour can.** The skill ecosystem (agents repo,
   `skills-library/`, the dated `skills-creation` conformance record) is
   the only place behaviour gets linted; rig documents are outside it.
3. **Nothing lints the rig as a set of documents that must agree.** There is
   no check for: inline behaviour in knot files, shared project-document
   references (reader vs editor), emitted events without a `strand-dir`
   consumer (or vice versa), or format definitions that point at the file
   they govern (`coding.md` declares the register's entry format "already
   declared at the top of `discoveries.md`" — the format's source of truth
   is the runtime file itself).
4. **The convention is missing.** No normative rule says knot files are
   *wiring* (frontmatter, event/strand declarations, profile reference, a
   one-line intent) and that procedural behaviour belongs in a skill
   referenced by the profile or the knot.

## Target

When this plan is done:

- **The rule is normative.** `knot-design` (agents repo) carries the
  behaviour-in-skill rule — a knot file declares identity, wiring, and at
  most a one-line intent; procedural behaviour lives in a skill. The knot
  repo's `docs/design-guide.md` carries the same rule at repo level. Both
  name the measurable boundary the lint applies (word/procedure thresholds
  set by Phase 1, recorded in both).
- **`knot-lint` exists** as a sub-skill under the `knot` master
  (`agents/skills-library/knot-lint/`): `scripts/knot-lint.py`
  (stdlib-only Python, the `knot-visualise` precedent) runs a static audit
  of a rig directory and emits an **advisory report** (the
  `project-lint-files` shape: report only, never modifies; rectification is
  a separate, user-walked act). Check families:
  1. **Inline behaviour** — knot/profile files exceeding the
     declared boundary (prose beyond the intent line, numbered procedures,
     imperative write contracts).
  2. **Shared-document table** — every `project/` document referenced by
     ≥2 rig documents, each reference classified reader vs editor from the
     referrer's wording; ≥2 editors is a finding.
  3. **Event pairing** — every emitted event name has a `strand-dir`
     consumer and every `strand-dir` has a named producer.
  4. **Format-bootstrap circularity** — a rig document that defines a
     document's format by pointing at that document itself.
  5. **Naming leg** — the shared-document table handed to the
     state/log classification (the `skill-data-filenames` /
     `project-lint-files` defect family).
- **The first dated report** covers the workspace's rigs (software-factory
  rig, this repo's rig), classified and committed to this repo's project
  lifecycle as data, not standard.

## Existing Tests

| Test surface | What it covers | Status |
|---|---|---|
| Knot Rust suite (`cargo test`) | Engine behaviour (dispatch, queue, tie-off) | ✅ Green — no overlap with a static document audit |
| `knot-visualise` (plan 093) | Graph extraction from the same rig files | ✅ Shipped — stdlib-only Python + D3 render; no checks, no fixtures |
| `skills-creation` conformance record | Dated re-measure of skill descriptions | ✅ Shipped — the dated-report precedent for the skills side |
| `project-lint-files` v0.2.0 | Advisory file-hygiene lint (write side + session read side) | ✅ Shipped — the advisory-only + one-rectification-per-finding shape |

## Test Gaps

- No **fixture rigs**: nothing a lint can be certified against (one
  good rig that reports zero findings, one bad rig with a known,
  enumerated finding set).
- No **golden finding set** per check family (expected finding, expected
  severity, expected rectification line).
- No **selftest**: the lint must run its fixtures and diff against the
  goldens, so a lint regression is red without a real rig.
- No measured **threshold calibration**: the inline-behaviour boundary
  (word count / procedure markers) must be set against the fixtures *and*
  the two real rigs so the first report's findings are defensible, not
  artefacts of a guessed number.

## Phases

### Phase 0: The rule lands (normative)

The behaviour-in-skill rule is written into `knot-design` (agents repo —
rule text + the measurable boundary as a placeholder the lint will
calibrate) and into this repo's `docs/design-guide.md` (same rule, repo
level). `knot-design` version bump + `skills <skill> vX.Y.Z: …` commit
per the agents-repo convention; conformance record re-measured.
**Pane:** conformance re-measure — the rule present at both sites, versions
bumped, conformance record dated.

### Phase 1: The lint engine + fixture rigs

`knot-lint.py` (stdlib-only) implements check families 1–5, with a
`--selftest` that builds the fixture rigs (`references/fixtures/good-rig/`,
`references/fixtures/bad-rig/`), runs the audit, and diffs against the
golden finding sets. Thresholds calibrated here against the fixtures and
the two real rigs; the calibrated boundary is written back into
`knot-design` and the design guide (closes the Phase 0 placeholder).
**Pane:** `knot-lint.py --selftest` green — the good rig reports zero
findings and the bad rig reports exactly the golden set.

### Phase 2: The first dated report (read-only)

The lint runs against the software-factory rig and this repo's rig
(read-only; nothing in either rig is modified). Findings are classified
(reader/editor split of the 14-document shared table, inline-behaviour
list, event-pairing gaps, circular references) and committed as a dated
report under this repo's `project/` lifecycle — data, not standard, the
`project-lint-files` convention. The calibrated thresholds from Phase 1
are validated against the real finding density; the report records any
threshold the real rigs force to move.
**Pane:** the dated report exists, with the per-rig tables and one primary
rectification per finding.

### Phase 3: Skill packaging + master routing

`knot-lint` becomes a proper sub-skill (`SKILL.md` — when to run it, the
check families, the advisory-only boundary, the report location
convention), registered in the `knot` master's routing table; version
0.1.0; agents-repo commit + sync per the harness convention.
**Pane:** `skills-creation` conformance audit of `knot-lint` passes and
the master router carries the routing line.

## Notes

- **Cross-repo scope, single usecase.** The actor goal is one — "a rig
  maintainer lints a rig and gets an advisory report against the
  behaviour-in-skill rule" — but the file set spans the agents repo
  (skills) and this repo (design guide, the dated report). The plan is
  hosted here; skill work lands in the agents repo under its own sync
  convention (edits via `~/.agents`, commit + push, then `./sync.sh`
  fast-forwards the submodule and bumps the pointer).
- **Advisory only.** The lint never modifies a rig document — the
  software-factory rig is live production machinery, and the
  `project-lint-files` precedent (report only; rectification is a
  separate, user-walked act) is the right shape for a first version.
- **Plan 097 is In Progress** in this repo; this plan touches none of its
  surfaces (skills + docs + reports only, no engine code).
- The 14-document shared-reference table measured on 2026-10-22 is the
  calibration baseline for check family 2; it should be re-measured by
  Phase 2, not trusted from this plan.
- Open question for Phase 1: whether profiles (wider than knots — they
  carry ownership and test-scope tables) get the same inline-behaviour
  threshold as loom knots, or a separate, looser one. The first report's
  defensible thresholds decide.
