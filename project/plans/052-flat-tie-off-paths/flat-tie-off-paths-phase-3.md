# Phase 3: Documentation — Update Domain Glossary, User Docs, Skills

**Plan:** [Flatten Tie-Off Paths](flat-tie-off-paths-plan.md)

## Checklist
- [x] Update `project/domain-glossary.md` — "Knot" section: change tie-off path to `rig/tie-offs/{loom-id}/{knot-name}-tie-off.md`
- [x] Update `project/domain-glossary.md` — "Tie-off Directory" section: remove knot-name subdirectory from description
- [x] Update `project/domain-glossary.md` — "Tie-Off Events" section: update layout diagram and consumer `strand-dir` reference path
- [x] Update `project/domain-glossary.md` — "Tie-off" section: update path description
- [x] Update `project/domain-glossary.md` — "Term Relationships" diagram: flatten the tie-off tree (remove knot-name directory level)
- [x] Update `docs/concepts.md` — change `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md` to `rig/tie-offs/{loom-id}/{knot-name}-tie-off.md`
- [x] Update `docs/configuration/rig-structure.md` — update directory tree and path examples to flat layout
- [x] Update `docs/configuration/knots.md` — update tie-off path and directory tree to flat layout
- [x] Update `docs/troubleshooting.md` — update any tie-off path references to flat layout
- [x] Update `.agents/skills/knot-inspect/SKILL.md` — update tie-off path examples in state JSON and loom-log examples
- [x] Update `.agents/skills/knot-create/SKILL.md` — update rig structure diagram and tie-off event descriptions
- [x] Review `AGENTS.md` — check for tie-off path references and update if needed (no references found)
- [x] Search project docs and skills for any remaining nested tie-off path patterns (`tie-offs/.*\/.*\/.*-tie-off`) — also updated `docs/getting-started.md`, `docs/workflows/file-generation-workflow.md`, `docs/workflows/review-workflow.md`, `docs/api-reference.md`

## Deviations

## Discoveries

## Notes
- The domain-glossary "Tie-off Directory" section was reworded from "path under `rig/tie-offs/{loom-id}/{knot-name}/`" to "path under `rig/tie-offs/{loom-id}/`" to clarify that tie-offs now live flat in the loom-level directory.
- The knot-create skill's event `mkdir` commands changed from `rig/tie-offs/{loom-id}/{knot-name}/{event-type}` to `rig/tie-offs/{loom-id}/{event-type}` — event subdirectories are now direct children of the loom's tie-off directory alongside the flat tie-off files.
- Consumer `strand-dir` references simplified from `../../tie-offs/<loom-id>/<knot-name>/<event-type>` to `../../tie-offs/<loom-id>/<event-type>` (one level shallower).
- `AGENTS.md` contains no tie-off path references — no changes needed.
- Additional files discovered by broad search: `docs/getting-started.md`, `docs/workflows/file-generation-workflow.md`, `docs/workflows/review-workflow.md`, `docs/api-reference.md` — all updated.

## Deviations

## Discoveries

## Notes
