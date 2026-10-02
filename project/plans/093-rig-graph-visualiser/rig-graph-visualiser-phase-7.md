# Phase 7: Arced Parallel Edges

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [x] `scripts/rig-graph-template.html`:
      - per-unordered-pair edge count over string ids (pre-simulation);
        edges of a multi-edge pair spread into perpendicular quadratic
        arcs (`arcGeom`), lone edges straight
      - `link` and `linkHit` become `path` elements; tick sets `d`;
        labels at the arc midpoint (quadratic t = 0.5)
      - `· loom` label suffix dropped — the dotted style alone marks
        loom-wide scope; `· all` suffix kept for wildcards
- [x] `scripts/test-rig-graph.py` — wiring markers (`pairCount`,
      `arcGeom`, idx assignment); existing suite green
- [x] `.agents/skills/knot-visualise/SKILL.md` — graph-model notes:
      parallel edges arc; **dotted = loom-wide (no label suffix)**,
      `· all` suffix = wildcard; version 1.3.0 → 1.4.0
- [x] Regenerate `~/workspace/finance/budget-app-frontend/rig-graph.html`
- [x] Re-deploy `knot-visualise` to `~/.agents/skills-library/` (diff verify)
- [x] Version bump 0.54.0 → 0.55.0 (Cargo.toml + release notes), commit sha `03ade24` recorded (single-commit task on
      main, no branch per branch contract)

## Deviations
<!-- Small single-commit change on main; no feature branch (branch contract:
single-commit tasks may skip). -->

## Discoveries
- "Arrowhead disappeared on click" was the *other* overlapping edge
  fading — correct single-edge selection behaviour, masked by the overlap.
  The fix is geometric separation, not selection logic.

## Notes
- Pair counting must run before `forceLink` rewrites `source`/`target`
  from id strings to node objects (ids were captured as the raw
  strings); `arcGeom` runs post-init where they are objects.
