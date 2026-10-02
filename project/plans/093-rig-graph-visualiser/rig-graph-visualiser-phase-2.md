# Phase 2: Skill Authoring

**Plan:** [Rig Graph Visualiser](rig-graph-visualiser-plan.md)

## Checklist
- [ ] Write `.agents/skills/knot-visualise/SKILL.md` (house frontmatter format of
      the sibling knot skills): workflow, CLI contract (`--out` required, `--rig`,
      `--no-state`, `--json`), graph-model vocabulary (knot/input/system nodes;
      edge forms; unresolved edges), choosing the output location with the user,
      interpretation guide (loops, fan-out, orphaned inputs, unresolved targets),
      DO-NOT-USE cross-references (knot-create, knot-inspect, knot-dispatch)
- [ ] Update `.agents/skills/knot/SKILL.md` — add `knot-visualise` to the routing
      table and to sibling DO-NOT-USE mentions where the router distinguishes
      sub-skills
- [ ] Update `AGENTS.md` — add `knot-visualise` to the Knot Skills list and to the
      deploy + verify copy loops (skill names list in both `for skill in ...`
      blocks)
- [ ] Verify consistency — skill frontmatter matches sibling format; router table,
      AGENTS.md skills list, and deploy/verify loops all contain `knot-visualise`
      (grep check across the three files)
- [ ] Verification — documentation-only phase (no code change); grep
      consistency check above green; commit sha recorded as this phase's
      external-verdict request.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
