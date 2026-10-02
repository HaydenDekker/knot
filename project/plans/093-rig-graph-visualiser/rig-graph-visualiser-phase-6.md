# Phase 6: Edge Event Labels on Hover/Highlight + Wide-Scope Marker

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [x] `scripts/rig-graph.py` — loom-level subscriptions tag their fan-out
      edges `"scope": "loom"`; wildcard subscriptions tag `"scope": "all"`;
      knot-level / system / input edges unchanged
- [x] `scripts/rig-graph-template.html`:
      - `.edge-label` hidden by default (`opacity: 0`); `.lit` and
        `.hover` classes reveal
      - hover handlers on the wide `linkHit` lines (identity-based label
        lookup, `filter((e) => e === d)`)
      - `highlight()` toggles `lit` on labels of edges in the active set
        (edge, node, and loom modes)
      - scoped edges render dotted (`.scoped-edge`), labels suffixed
        `· loom` / `· all`
- [x] `scripts/test-rig-graph.py` — extractor: `scope` tags on loom and
      wildcard fan-out edges, absent on knot-level edges; wiring markers:
      `edge-label.lit`, `linkHit.on("mouseover"`, `d.scope`, `scoped-edge`;
      existing suite green
- [x] `.agents/skills/knot-visualise/SKILL.md` — reading guide: labels
      hidden by default, revealed on hover or highlight; dotted edges =
      wide (loom/wildcard) subscription; version 1.2.0 → 1.3.0
- [x] Regenerate `~/workspace/finance/budget-app-frontend/rig-graph.html`
- [x] Re-deploy `knot-visualise` to `~/.agents/skills-library/` (diff verify)
- [x] Version bump 0.53.0 → 0.54.0 (Cargo.toml + release notes), commit sha `858d1c4` recorded (single-commit task on
      main, no branch per branch contract)

## Deviations
<!-- Small single-commit change on main; no feature branch (branch contract:
single-commit tasks may skip). -->

## Discoveries
- The arrows the user read as "`lifecycle-on-block` sends to
  `plan-implementer`" are the fan-out of `plan-implementer`'s own
  `event:planning-loom:PlanReady` loom-wide subscription — real data, but
  visually indistinguishable from a knot-specific subscription; hence the
  `scope` marker.

## Notes
- 70 of 85 edges in the budget-app rig are loom-scope — this rig
  mostly wires loom-wide subscriptions, so the dotted marker is the
  dominant edge style there (knot-specific edges are the minority and now
  read as the solid ones).
