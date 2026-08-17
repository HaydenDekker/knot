# Phase 4: Skills, glossary, docs, changelog

**Plan:** [Rig Repository Separation](rig-repo-separation-plan.md)

## Checklist
- [x] `knot-init/SKILL.md` — running-check path moves to `tie-offs/<rig>/state.json`; new "Rig Repository" section; terminology lines updated — done, v4.0.0, compat Knot 0.31.0+
- [x] `knot-init/knot-glossary.md` — tie-off path + `tie-offs/` runtime-tree description — done, new **Runtime Tree** term; all paths + relationship diagram re-rooted
- [x] `knot-manage/SKILL.md` — two-repo review workflow; paths updated; "no git repo" error table adjusted — done, v1.1.0; added post-migration watcher re-trigger caveat
- [x] `knot-inspect/SKILL.md` — path references — done, v3.3.0
- [x] `knot-analyst/SKILL.md` — path references — done, v1.2.0
- [x] `knot-dispatch/SKILL.md` — path references, quick-reference commands — done, v1.1.0
- [x] `knot-create/SKILL.md` — path references (state.json, tie-offs) — done, v5.5.0; domain model + example layout split into rig source + runtime tree
- [x] `knot-design/SKILL.md` — path references — done, v1.5.0
- [x] `knot-abstractions/SKILL.md` — architecture diagram + runtime data boundary — done, v1.2.0
- [x] `knot-update/SKILL.md` — new 0.31.0 changelog entry (what moved, why, auto-migration, manual untrack, watcher re-trigger caveat, verification) — done, v1.7.0; historical entries left untouched (immutable history)
- [x] `docs/configuration/rig-structure.md` — directory tree + Git-Friendly section — done; tree split into `rig/` (source, own git) + `tie-offs/rig/` (runtime); new "Rig Repository and Runtime Tree" section replacing Git-Friendly
- [x] `docs/concepts.md`, `docs/getting-started.md`, `docs/troubleshooting.md` — path references — done; concepts gained runtime-tree hierarchy + two-repo git versioning note
- [x] `docs/workflows/*.md`, `docs/configuration/knots.md`, `docs/release-notes.md` — path references / release entry — done; release notes gained v0.31.0 entry (historical entries untouched)
- [x] `README.md`, `AGENTS.md` — terminology + path references — done; README concept table gained Runtime tree row; AGENTS.md install snippet fixed (see Deviations)
- [x] Publish all updated skills to `~/.agents/skills/` and diff-verify — done, all 9 skills OK
- [x] Final verification: grep for stale `rig/state.json` / `rig/tie-offs` path references in skills + docs — clean (remaining hits are intentional: knot-update historical changelog entries, release-notes "Before" columns)

## Deviations

- **`cp -r` nesting bug in skill installation** — the documented
  installation command `cp -r .agents/skills/$skill ~/.agents/skills/$skill`
  nests the directory when the destination already exists (produces
  `~/.agents/skills/knot-init/knot-init/`), silently leaving the old
  SKILL.md in place. Discovered while publishing this phase. Fixed in
  `knot-init` SKILL.md (step 4a + manual install) and `AGENTS.md` to use
  `mkdir -p` + `cp -r .agents/skills/$skill/. ~/.agents/skills/$skill/`.

## Discoveries

- Historical changelog entries in `knot-update` (and release notes)
  reference the pre-0.31.0 paths. Those are immutable history — the
  0.31.0 entry supersedes them for verification purposes. Stale-path
  greps must exclude them.
- The `knot-init` AGENTS.md template (appended to *other* projects) also
  needed the terminology update — done in the skill's step 4b template.

## Notes

- Skill versions bumped minor except `knot-init` (major, 3.4.0 → 4.0.0)
  because its core running-check path changed.
- `rig share` unchanged; docs note the zip now equals the rig git's
  tracked content.
