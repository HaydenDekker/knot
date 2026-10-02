# Phase 2: Skill Authoring

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [x] Write `.agents/skills/knot-visualise/SKILL.md` — done: frontmatter in
      house format (name/description with USE FOR + DO-NOT-USE cross-refs to
      knot-inspect/knot-create/knot-dispatch/knot-analyst, license, metadata),
      workflow (ask for `--out` first), CLI contract table, graph-model
      vocabulary (node kinds, edge forms, state overlay, duplicate-id
      rule), interpretation guide (loops, fan-out, orphaned inputs,
      unresolved targets, terminal producers), troubleshooting table
- [x] Update `.agents/skills/knot/SKILL.md` — done: routing-table row +
      "visualising the rig graph" added to the master description's
      capability list and USE FOR keywords
- [x] Update `AGENTS.md` — done: Knot Skills list entry; `knot-visualise`
      added to **both** deploy and verify `for skill in ...` loops
- [x] Verify consistency — done: `grep -c knot-visualise` → AGENTS.md: 3
      (skills list + deploy loop + verify loop), router: table row, skill
      file present; frontmatter matches sibling format
- [x] Verification — documentation-only phase (no code change); grep
      consistency check green; commit sha `9418439` recorded as this
      phase's external-verdict request.

## Deviations
<!-- None -->

## Discoveries
- AGENTS.md's Skill Installation section has **two** skill-name loops
  (deploy + verify); a new sub-skill must be added to both or the verify
  loop silently skips it.

## Notes
- The skill targets the production layout (sub-skill →
  `~/.agents/skills-library/`), matching every other knot sub-skill's
  router row.
