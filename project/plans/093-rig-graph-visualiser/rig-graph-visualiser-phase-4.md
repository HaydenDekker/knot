# Phase 4: Interactive Legend and Click Highlighting

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [x] Extend `scripts/rig-graph-template.html` — done:
      - legend loom rows: checkbox (filter: hides loom's knots + touching
        edges; input/system nodes stay; `display` toggling, simulation
        untouched) + name click (highlight loom: its knots + incident edges
        full opacity, rest faded to 0.15/0.05; re-click toggles off)
      - edge click via wide invisible hit lines (10 px, `pointer-events:
        stroke`) → highlight edge + both endpoint nodes
      - node click → highlight node + incident edges + direct neighbours
      - background click clears; selection modes mutually exclusive;
        filtering and highlighting compose (a filter that hides the active
        node highlight clears that highlight)
- [x] Extend `scripts/test-rig-graph.py` — done: `test_interaction_wiring`
      (checkbox generation, loom-row class, highlight/filter/clear symbols,
      background-clear handler, hit-line wiring, stopPropagation,
      hidden-loom visibility predicates); existing suite unchanged —
      34/34 green
- [x] Update `.agents/skills/knot-visualise/SKILL.md` — done: interaction
      contract table (filter/highlight/clear semantics), version 1.0.0 → 1.1.0
- [x] Regenerate `~/workspace/finance/budget-app-frontend/rig-graph.html` —
      done (38 nodes, 85 edges; new interactions live)
- [x] Re-deploy `knot-visualise` to `~/.agents/skills-library/` — done,
      diff verify `OK`
- [x] Verification — `python3 scripts/test-rig-graph.py` green (34/34; the
      targeted scope — no Rust code touched, `cargo` gates unaffected);
      commit sha `5b79dd6` recorded as this phase's external-verdict
      request.

## Deviations
- Plan files are normally static; Phase 4 was appended to the plan at the
  user's explicit request (reopened the completed plan).

## Discoveries
- The generated-HTML tests assert against the **source**, where the
  checkboxes are D3 calls (`.attr("type", "checkbox")`), not HTML markup —
  wiring assertions must target the JS symbols, not markup.
- The embedded-JSON round-trip regex anchors on the first line after the
  data script; it must track template app-script changes (`const loom` →
  `const svg`).

## Notes
- Highlighting fades via element `opacity` (composes with the links'
  `stroke-opacity`); filtering uses `display: none` and never touches the
  force simulation (cheap, no re-layout on filter).
