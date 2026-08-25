# Phase 3: Documentation, changelog, version

**Plan:** [Consumer Persistent Wake — No Lost Queue Notifications](consumer-persistent-wake-plan.md)
**Commit:** `e151fcc` (branch `refactor/consumer-persistent-wake-plan`)
**Date:** 2026-08-25

## Checklist
- [x] `docs/concepts.md` — Event Queue section: "A queued event always wakes the processor — the only empty-queue state is a genuinely empty `events/` directory." appended to the section's intro paragraph
- [x] `.agents/skills/knot-update/SKILL.md` — new changelog entry "Persistent Queue Wake — No Lost Queue Notifications (Knot 0.37.0, 2026-08-25)": what changed (armed-at-call `notified()`, arm-before-check in both consumer loops), why (removes the latent dependency on tokio 1.52.3's stored-permit behaviour rather than fixing a live bug), the wake guarantee, affected documents (none), before/after table, "Migration: none required" with the queue-file schema-unchanged note. Follows the 0.36.0 entry format
- [x] `.agents/skills/knot-update/SKILL.md` frontmatter — version `1.13.0` → `1.14.0`, compatibility `Knot 0.36.0+` → `Knot 0.37.0+`
- [x] Updated skill published globally per AGENTS.md — `cp -r .agents/skills/knot-update/. ~/.agents/skills-library/knot-update/`, diff-verified byte-identical (`knot-update: OK`)
- [x] `cargo build` clean; `cargo test` green (docs/skills-only phase, suite unaffected)
- [ ] Version bump (MINOR, 0.36.0 → 0.37.0) + `docs/release-notes.md` entry — **deferred to the project-plan-completion flow** (see Deviations 1)

## Deviations
1. **Version bump and release notes not in this phase's commit.** The plan phrases item 3 as "via the project-plan-completion skill", and the plan-074 precedent put the bump + release notes in the completion commit, not the final phase. The knot-update changelog entry is written for 0.37.0 ahead of time; the completion flow applies the `Cargo.toml` bump, writes the release-notes entry, merges to `main`, and marks the plan ✅ Complete.
2. **Production skill was behind the project copy** — `~/.agents/skills-library/knot-update/` was at 1.12.0 while the project copy was 1.13.0 (the 1.13.0 bump from a prior plan had not been deployed). The Phase 3 publish synced both the 1.13.0 state and the new 1.14.0 entry in one copy.

## Discoveries
- The final phase's publish step is the natural place to catch production-sync drift (see Deviation 2) — a prior plan's skill bump had skipped deployment.
- The 0.37.0 changelog entry explicitly records the Phase 1 finding (tokio 1.52.3 stored-permit behaviour) so a future reader does not mistake this release for a field-incident fix: it removes a latent dependency and makes the guarantee explicit and version-independent.

## Notes
- All implementation phases are complete. Remaining work is the completion flow: bump 0.36.0 → 0.37.0 (MINOR — plan 075 has not landed, per the plan's coordination note), `docs/release-notes.md` entry, merge `refactor/consumer-persistent-wake-plan` to `main`, delete branch, update master plan index.
