# Phase 5: Loom Highlight Includes All Feeding Nodes

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [x] `scripts/rig-graph-template.html` — in `highlight()`'s loom branch:
      snapshot the loom's knot ids, then for every edge whose **target** is
      a loom knot, add the edge's **source** to the focused-node set (the
      feeding node — knot, input, or system); incident edges unchanged;
      node/edge highlight branches unchanged
- [x] `scripts/test-rig-graph.py` — wiring marker for the end-node
      extension (comment marker in the loom branch); existing suite green
- [x] `.agents/skills/knot-visualise/SKILL.md` — interaction contract:
      loom highlight includes all nodes feeding into the loom (upstream
      one hop); version 1.1.0 → 1.2.0
- [x] Regenerate `~/workspace/finance/budget-app-frontend/rig-graph.html`
- [x] Re-deploy `knot-visualise` to `~/.agents/skills-library/` (diff verify)
- [x] Version bump 0.52.1 → 0.53.0 (Cargo.toml + release notes), commit sha
      recorded (commit sha `fa21de9` — single-commit task on main, no branch per branch contract)

## Deviations
<!-- Small single-commit change on main; no feature branch (branch contract:
single-commit tasks may skip). -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- The loom's knot ids are snapshotted *before* the edge pass so
  feeding nodes added mid-pass cannot pull in further edges (strictly one
  hop upstream).
