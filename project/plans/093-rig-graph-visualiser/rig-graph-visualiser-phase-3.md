# Phase 3: End-to-End Check and Deployment

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [x] Run `python3 scripts/rig-graph.py --out /tmp/rig-graph-e2e.html`
      against the real rig in this repo — done: 4 nodes / 2 edges
      (`new-loom:review-knot`, `workflow-loom:review-knot` + their
      `src/*` inputs), node ids unique, embedded data parses, HTML
      standalone (offline by the Phase-1 guarantee)
- [x] Commit the e2e verification — no new code; covered by the phase
      commit below
- [x] Deploy the skill per AGENTS.md "Skill Installation":
      - `knot-visualise` → `~/.agents/skills-library/knot-visualise/` — done
      - updated `knot` master → `~/.agents/skills/knot/` — done
      - diff verification — both `OK` (byte-identical)
- [x] Verification — script runs clean against the real rig; deployed skill
      diffs clean; commit sha `140b391` recorded. Hand off to
      `project-plan-completion` for version handling, knowledge extraction,
      and branch merge to `main`.

## Deviations
<!-- None -->

## Discoveries
- The real rig in this repo is a tiny 2-knot rig (both knots share the
  name `review-knot` across looms), so the e2e graph is small — the fixture
  rigs in `tests/fixtures/rig-graph/` exercise the full topology surface
  instead.

## Notes
- Deployment followed the AGENTS.md copy + diff-verify pattern exactly.
