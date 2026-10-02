# Phase 0: Graph Extraction Script

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [ ] Create `scripts/rig-graph.py` — stdlib-only Python 3:
      - flat `key: value` frontmatter parser (split on `---`, strip quotes; no PyYAML)
      - rig walk: looms = dirs ending `-loom` under `--rig` (default `rig/`); knots = `.md` files inside
      - edge resolution for all five `strand-dir` forms (knot-level, loom-level `-loom` suffix, `*` wildcard, rig-level `knot` target, plain path)
      - unresolved targets flagged `unresolved: true`, not dropped
      - state overlay from `tie-offs/<rig-basename>/state.json` (status + last_event_at), absent file tolerated, `--no-state` disables
      - graph JSON: `{"nodes": [...], "edges": [...]}`
      - CLI: `--out` (required unless `--json`), `--rig`, `--no-state`, `--json` (print JSON, skip HTML)
      - non-zero exit + clear message: missing rig dir, no looms found
- [ ] Create fixtures `tests/fixtures/rig-graph/`:
      - `rig/` with looms covering: knot-level subscription, loom-level subscription, wildcard subscription, rig-level (`event:knot:...`) subscription, plain-path `strand-dir`, an unresolved target, a knot with no subscribers
      - `tie-offs/rig-graph/state.json` matching the fixture rig (status + last_event_at)
      - a second fixture rig with no `state.json` (overlay-absent case)
      - a broken case: a non-existent rig dir (error exit code)
- [ ] Write `scripts/test-rig-graph.py` (unittest, stdlib only) with fixture-driven assertions:
      - node set (knot/input/system nodes, attributes)
      - edge set per subscription form, with event-name labels
      - unresolved flags
      - state overlay applied (with state.json) / omitted (absent + `--no-state`)
      - missing-rig error exit code
- [ ] Verification — `python3 scripts/test-rig-graph.py` green (the targeted scope for this
      phase — no Rust code touched, so `cargo` gates are unaffected); commit sha
      recorded as this phase's external-verdict request. The ON-COMMIT Rust suite is
      not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
