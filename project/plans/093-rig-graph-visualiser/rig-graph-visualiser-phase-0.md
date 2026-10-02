# Phase 0: Graph Extraction Script

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [x] Create `scripts/rig-graph.py` — stdlib-only Python 3 — done (frontmatter
      parser, rig walk, edge resolution for all five forms, state overlay,
      `--out`/`--rig`/`--no-state`/`--json` CLI, exit 2 on missing rig / no looms)
- [x] Create fixtures `tests/fixtures/rig-graph/` — done: `full/` (all five
      `strand-dir` forms, unresolved target, state.json overlay, twin rig via
      `dup/`), `plain/` (no state.json, lone knot with no subscribers)
- [x] Write `scripts/test-rig-graph.py` (unittest, stdlib only) — done, 26 tests
      covering node set, edge set per form, unresolved flags, state overlay
      applied/absent/`--no-state`, duplicate knot ids, error exit codes
- [x] Verification — `python3 scripts/test-rig-graph.py` green (26/26; the
      targeted scope for this phase — no Rust code touched, so `cargo` gates
      are unaffected); commit sha `4585498` recorded as this phase's
      external-verdict request. The ON-COMMIT Rust suite is not required by
      this item.

## Deviations
- Plan said one fixture rig; a third fixture (`dup/`) was added because the
  real rig in this repo already has duplicate knot names across looms (see
  Discoveries).

## Discoveries
- **Duplicate knot ids across looms are legal** (this repo's own rig has
  `review-knot` in both `new-loom` and `workflow-loom`). Node ids are
  therefore unique-ified: bare knot id when unique across the rig, else
  `<loom>:<knot>` (label stays the bare name). The state overlay is keyed by
  `(loom, knot)` from the nested `state.json` looms list for the same reason.
- A bare knot-id subscription (`event:<knot>:<Event>`) matches **every**
  knot with that id (Knot matches on id alone), so such a subscription fans
  out across looms when the id is duplicated.
- This repo has no `project/test/run-scope.md` and no `scripts/test-data/`
  registration infra; verification items use the targeted Python test run as
  the phase's ALWAYS-equivalent.

## Notes
- Frontmatter parser handles the `event:` URIs' embedded colons by splitting
  on the first colon only.
- The synthetic system node and unresolved-target nodes are added only when
  referenced by at least one edge (no phantom nodes).
