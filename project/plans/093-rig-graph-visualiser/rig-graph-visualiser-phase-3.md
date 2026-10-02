# Phase 3: End-to-End Check and Deployment

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [ ] Run `python3 scripts/rig-graph.py --out /tmp/rig-graph.html` against the
      real rig in this repo; sanity-check the rendered graph JSON (nodes/edges
      match `rig/`) and that the HTML is standalone (openable without network)
- [ ] Commit the e2e verification (no new code expected)
- [ ] Deploy the skill per AGENTS.md "Skill Installation":
      - `knot-visualise` → `~/.agents/skills-library/knot-visualise/`
      - updated `knot` master → `~/.agents/skills/knot/`
      - diff verification of both (source vs deployed, byte-identical)
- [ ] Verification — script runs clean against the real rig; deployed skill diffs
      clean; commit sha recorded. Hand off to `project-plan-completion` for
      version handling, knowledge extraction, and branch merge to `main`.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
