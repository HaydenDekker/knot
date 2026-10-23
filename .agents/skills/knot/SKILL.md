---
name: knot
description: "Master router for all Knot agent-orchestration (rig) work: the rig lifecycle (init, service start/stop, version migration), rig modelling (looms, knots, profiles; loom-and-knot design; the software-factory rig — the GProc generic process layer), execution (strand and event dispatch), the read side (state inspection, graph visualisation, tie-off and git-output review, productivity and blocker analysis, static consistency lint), the Knot abstractions, and the tie-off content section. Project document lifecycle of the served project: the project master."
license: MIT
metadata:
  author: AI Workflow Team
  version: "1.0.0"
  compatibility: "the ~/.agents skill set (skills/ + skills-library/ + project/)"
---

# Knot (master)

Router for the Knot domain. The sub-skills below are **not** in your system
prompt — read the relevant one with the `read` tool before acting. Never
guess a sub-skill's workflow from this index.

## Routing table

| Task | Read |
|------|------|
| Initialise a rig in the current directory | `/home/hayden/.agents/skills-library/knot-init/SKILL.md` |
| Start / stop / restart the service, read the service log | `/home/hayden/.agents/skills-library/knot-start/SKILL.md` |
| Create / modify / delete looms, knots, profiles | `/home/hayden/.agents/skills-library/knot-create/SKILL.md` |
| Trigger knots: strand files, events, `knot step` | `/home/hayden/.agents/skills-library/knot-dispatch/SKILL.md` |
| Inspect rig state, looms, activity, profiles | `/home/hayden/.agents/skills-library/knot-inspect/SKILL.md` |
| Visualise the rig graph (knots + event/strand edges) as HTML | `/home/hayden/.agents/skills-library/knot-visualise/SKILL.md` |
| Review rig work: git history, tie-offs, producer→consumer comms | `/home/hayden/.agents/skills-library/knot-manage/SKILL.md` |
| Analyse rig productivity, project progress, blockers | `/home/hayden/.agents/skills-library/knot-analyst/SKILL.md` |
| Lint rig consistency: inline behaviour, shared docs, event pairing | `/home/hayden/.agents/skills-library/knot-lint/SKILL.md` |
| Design looms and knots: idempotency, naming, loops | `/home/hayden/.agents/skills-library/knot-design/SKILL.md` |
| Build/design a software-factory rig: GProc layer, interfaces | `/home/hayden/.agents/skills-library/knot-software-factory/SKILL.md` |
| Understand Knot's layered architecture and abstractions | `/home/hayden/.agents/skills-library/knot-abstractions/SKILL.md` |
| Customise the tie-off content section (the producer's header/content + the event-content guide; the contract/format/rules are hard-coded) | `/home/hayden/.agents/skills-library/knot-template/SKILL.md` |
| Migrate project documents across Knot binary versions | `/home/hayden/.agents/skills-library/knot-update/SKILL.md` |

## Typical flows

- **New rig**: init → design → create → start → dispatch first strand
- **Day-to-day**: start → dispatch → inspect → manage (review output)
- **Troubleshooting**: inspect → analyst → manage (service log via start)
- **Rig changes**: design → create → dispatch

## Rules

1. Read the sub-skill **fully** before executing any of its steps.
2. For multi-step flows, read each sub-skill before starting that step.
3. If a sub-skill references another sub-skill or project doc (e.g. the
   domain glossary), read that too before proceeding.
4. Verify state after file changes by reading `tie-offs/<rig>/state.json`.
