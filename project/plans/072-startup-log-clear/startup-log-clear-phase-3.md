# Phase 3: Documentation, skills, changelog

**Plan:** [Clear Loom-Logs and Rig-Log at Startup](startup-log-clear-plan.md)

## Checklist
- [ ] `docs/concepts.md` — replace "survives server restarts" wording with per-run semantics
- [ ] `knot-inspect` skill — annotate loom-log description: "cleared at knot startup (per-run scope)"
- [ ] `knot-analyst` skill — annotate log descriptions; change "last 24 hours" guidance to "since the last startup"
- [ ] `.agents/skills/knot-update/SKILL.md` — new changelog entry for 0.34.0; bump skill version + compatibility line
- [ ] Record completed behaviour in `docs/release-notes.md` (with version bump, 0.33.0 → 0.34.0, via plan completion)
- [ ] Publish updated skills globally (`~/.agents/skills/`) and verify with diff

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
