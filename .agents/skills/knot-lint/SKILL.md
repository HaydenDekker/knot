---
name: knot-lint
description: "Run a static consistency audit of a Knot rig directory and emit an advisory report (the report never modifies the rig — rectification is a separate, user-walked act). The stdlib-only Python script `scripts/knot-lint.py` checks five check families: the inline-behaviour boundary for knot/profile files (K1), the single-editor shared project document (K2), event pairing of emissions against `strand-dir` subscriptions (K3), format-bootstrap circularity (K4), and the state/log classification of the shared-document table (K5). Ships with the good/bad fixture rigs, the golden finding set, and `--selftest`."
license: MIT
metadata:
  author: Knot Team
  version: "0.1.0"
  compatibility: "Knot 0.62.0+ (reads rig files only; the service need not be running)"
---

# Knot Lint Skill

Lint a rig as a **set of documents that must agree** — the static
complement to `knot-inspect` (state), `knot-visualise` (graph), and
`knot-analyst` (runtime log). The audit encodes the normative rules
`knot-design` carries: the **behaviour-in-skill rule** (a knot file is
wiring — identity, event/strand declarations, at most a one-line
intent; procedure belongs in a skill), the **single-editor norm**, and
**no format-bootstrap circularity**.

**Advisory only.** The lint reads rig and project files and writes a
report — it never touches a rig document. Each finding carries one
primary rectification; rectification is a separate, user-walked act
(the `project-lint-files` convention: report only, data not standard).

---

## The script

`scripts/knot-lint.py` (stdlib-only Python 3). Runs read-only against
a rig directory — **the Knot service need not be running.**

```
python3 scripts/knot-lint.py [RIG_DIR] [--project-dir DIR] [--report OUT] [--json] [--selftest [FIXTURES_DIR]]
```

- `RIG_DIR` — the rig directory (default: `rig` in the cwd; pass an
  absolute path for a rig elsewhere).
- `--project-dir` — the `project/` document tree (default: auto — the
  rig dir itself if it holds `project/`, else the parent's `project/`).
- `--report OUT` — also write the markdown report to a file.
- `--json` — machine-readable audit to stdout.
- `--selftest` — run the fixture rigs (`references/fixtures/` by
  default) against the golden finding set.

**Exit codes:** 0 = no FAIL findings, 1 = FAILs present, 2 = usage
error.

## The check families

| Code | Family | Severity | What it flags |
|------|--------|----------|---------------|
| K1 | Inline behaviour | WARN | A knot file with prose beyond the intent line (default: > 80 measured words or ≥ 3 numbered steps) or a profile beyond its looser bound (> 400 words or ≥ 5 steps). |
| K2 | Shared-document table | FAIL | A `project/` document referenced by ≥ 2 rig documents of which ≥ 2 hold write rights (reader vs editor classified from the referrer's line wording). |
| K3 | Event pairing | WARN | K3A: an emitted event name with no `strand-dir` consumer. K3B: a `strand-dir` subscription whose producer token names no knot, no loom, the rig id, or the engine's rig-event token `knot`. |
| K4 | Format-bootstrap circularity | FAIL | A rig document that defines a document's format by pointing at that document itself (or at a runtime data file as the format's source of truth). |
| K5 | Naming leg | WARN | A shared-document table row whose file name classifies as **log** and is written under read pressure (the `skill-data-filenames` / `project-lint-files` defect family). |

Measured prose excludes frontmatter, code fences, and table lines.
The thresholds are the **measurable boundary** `knot-design` records
(calibrated 2026-10-23, plan 098 phase 1); the report's Provenance
section states the values used.

## Engine facts the pairing check encodes

- `RIG_EVENT_ENGINE_TOKEN` is `"knot"` (`src/domain/value_objects.rs`)
  — rig-scoped system events (e.g. `QueueIdle`) resolve to
  `event:knot:<Event>`, which is a named producer.
- `` `event: None` `` in a rig document is the "emit nothing"
  sentinel, not an event name.
- Emission evidence is **static** (`emit(s) \`Event\``,
  `event: Event` patterns) — runtime-injected emission is invisible to
  a static audit, so K3A is a lower bound, never a proof of absence.

## Report convention

A dated run is committed under the project's lifecycle as **data, not
standard** — the convention the first run established is
`project/lint/knot-lint-<YYYY-MM-DD>.md`: the run header (date,
thresholds, rigs audited), a calibration-validation section when
thresholds were in play, one section per rig with the full lint
output, and a reproducibility block. The report is advisory: it
records what was measured, never what was changed.

## Verification

```
python3 scripts/knot-lint.py --selftest
# selftest: OK (good rig: 0 findings; bad rig: 8 findings, exact match)
```

The selftest audits the on-disk fixture trees — `good-rig` (wiring
only: three knots with filesystem or event `strand-dir`s and one small
profile, no procedural prose) must report zero findings; `bad-rig`
must report
exactly the golden set (K1 x2, K2 x2, K3A, K3B, K4, K5). A lint
regression is red without a live rig.
