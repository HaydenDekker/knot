# Phase 3: Documentation, skills, changelog

**Plan:** [Clear Loom-Logs and Rig-Log at Startup](startup-log-clear-plan.md)

## Checklist
- [x] `docs/concepts.md` — Logs section rewritten: per-run semantics (truncated at every startup before discovery; log = current-run events only; tie-offs are the durable record); rig-log table row no longer says "append-only" across runs
- [x] `knot-inspect` skill — activity-log header + Activity Log Format section annotated "cleared at knot startup (per-run scope)"; version 3.4.0 → 3.5.0, compatibility → Knot 0.34.0+
- [x] `knot-analyst` skill — rig-log/loom-log headers + rig-log read guidance annotated per-run scope; "count `KnotFailed` … in the last 24 hours" → "since the last startup"; version 1.2.0 → 1.3.0, compatibility → Knot 0.34.0+
- [x] `.agents/skills/knot-update/SKILL.md` — new changelog entry "Per-Run Logs — Loom-Logs and Rig-Log Cleared at Startup (Knot 0.34.0, 2026-08-23)" (no document migration; first run discards earlier runs' log content; tie-offs unchanged; non-fatal clear); version 1.9.0 → 1.10.0, compatibility → Knot 0.34.0+
- [x] Record completed behaviour in `docs/release-notes.md` (v0.34.0 entry) + version bump 0.33.0 → 0.34.0 — done at plan completion (project-plan-completion)
- [x] Publish updated skills globally (`~/.agents/skills/`) and verify with diff — all 9 knot skills OK
- [x] Design knowledge extracted to `project/design/design-startup-log-clear.md` (startup sequence, ordering invariants, what the clear touches)

## Deviations
- Bumped the knot-inspect (3.4.0 → 3.5.0) and knot-analyst (1.2.0 → 1.3.0) skill versions and their compatibility lines to Knot 0.34.0+ — the plan only names the knot-update bump, but the annotations describe behaviour that only exists in 0.34+, and both skills track version/compatibility metadata (consistency with the knot-update precedent).

## Discoveries
- The pre-existing uncommitted local change to `.agents/skills/knot-dispatch/SKILL.md` (present before this plan started) was carried along in the global publish, per the AGENTS.md all-skills sync procedure. It is not committed on this branch.

## Notes
- Release-notes entry and the Cargo.toml version bump are deliberately deferred to the plan-completion step, per the plan's own wording.
