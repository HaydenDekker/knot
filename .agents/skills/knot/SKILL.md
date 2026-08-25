---
name: knot
description: "Master router for all Knot agent-orchestration (rig) work. Routes to sub-skills for initialising rigs, creating/modifying looms, knots and profiles, dispatching strands and events, inspecting rig state, reviewing tie-offs and git output, analysing productivity and blockers, designing looms and knots, understanding Knot abstractions, and migrating between Knot versions. USE FOR: rig, loom, knot, strand, event, tie-off, profile, dispatch, knot step, rig state, rig review, rig productivity, blockers, rig design, knot migration. DO NOT USE FOR: project document lifecycle (use the project master)."
---

# Knot (master)

Router for the Knot domain. The sub-skills below are **not** in your system
prompt — read the relevant one with the `read` tool before acting. Never
guess a sub-skill's workflow from this index.

## Routing table

| Task | Read |
|------|------|
| Initialise a rig in the current directory | `/home/hayden/.agents/skills-library/knot-init/SKILL.md` |
| Create / modify / delete looms, knots, profiles | `/home/hayden/.agents/skills-library/knot-create/SKILL.md` |
| Trigger knots: strand files, events, `knot step` | `/home/hayden/.agents/skills-library/knot-dispatch/SKILL.md` |
| Inspect rig state, looms, activity, profiles | `/home/hayden/.agents/skills-library/knot-inspect/SKILL.md` |
| Review rig work: git history, tie-offs, producer→consumer comms | `/home/hayden/.agents/skills-library/knot-manage/SKILL.md` |
| Analyse rig productivity, project progress, blockers | `/home/hayden/.agents/skills-library/knot-analyst/SKILL.md` |
| Design looms and knots: idempotency, naming, loops | `/home/hayden/.agents/skills-library/knot-design/SKILL.md` |
| Understand Knot's layered architecture and abstractions | `/home/hayden/.agents/skills-library/knot-abstractions/SKILL.md` |
| Migrate project documents across Knot binary versions | `/home/hayden/.agents/skills-library/knot-update/SKILL.md` |

## Typical flows

- **New rig**: init → design → create → dispatch first strand
- **Day-to-day**: dispatch → inspect → manage (review output)
- **Troubleshooting**: inspect → analyst → manage
- **Rig changes**: design → create → dispatch

## Rules

1. Read the sub-skill **fully** before executing any of its steps.
2. For multi-step flows, read each sub-skill before starting that step.
3. If a sub-skill references another sub-skill or project doc (e.g. the
   domain glossary), read that too before proceeding.
4. Verify state after file changes by reading `tie-offs/<rig>/state.json`.
