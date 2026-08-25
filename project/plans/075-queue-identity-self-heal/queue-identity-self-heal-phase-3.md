# Phase 3: Documentation, skills, changelog

**Plan:** [Queue Entry Identity Self-Heal — Filename Is the Event ID](queue-identity-self-heal-plan.md)
**Commit:** `ed9a29c` (branch `refactor/queue-identity-self-heal-plan`)
**Date:** 2026-08-25

## Checklist
- [x] `.agents/skills/knot-dispatch/SKILL.md` — Event Queue "Key properties" extended with a **Reorderable by rename** bullet: renaming a queued event's file reorders the FIFO (the supported way to front a queued event, e.g. a manual rectify); the filename is the queue entry's identity and the queue repairs the file's internal id to the stem on the next scan (atomic rewrite, one `[queue] repaired …` warning per repaired file); `queued_at` preserved
- [x] `.agents/skills/knot-dispatch/SKILL.md` frontmatter — version `1.3.0` → `1.4.0`, compatibility `Knot 0.35.0+` → `Knot 0.37.0+` (see Deviations 1)
- [x] `docs/concepts.md` — Event Queue section: one name/id sentence added alongside (not disturbing) 076's wake-guarantee sentence: the filename stem is the queue entry's identity; renaming reorders the FIFO and the queue repairs the internal id to the stem on the next scan (`queued_at` preserved)
- [x] `.agents/skills/knot-update/SKILL.md` — new entry "Queue Entry Identity Self-Heal — Filename Is the Event ID (Knot 0.37.0, 2026-08-25)" placed **above** the existing "Persistent Queue Wake (Knot 0.37.0)" entry — a single combined 0.37.0 release covering both plans. Full established format: what changed (scan-time repair, exact warning format, idempotency, rename-as-supported-operation, graceful vanished-head), why (phantom-head wedge + bookkeeping amplification, the 2026-08-25 incident), affected documents: none, before/after table, "Migration: none required" with the self-heal-on-first-scan bullets
- [x] `.agents/skills/knot-update/SKILL.md` frontmatter — version `1.14.0` → `1.15.0`; compatibility stays `Knot 0.37.0+`
- [x] Updated skills published globally per AGENTS.md — `knot-dispatch` and `knot-update` copied to `~/.agents/skills-library/`, diff-verified (`knot-dispatch: OK`, `knot-update: OK`)
- [x] `cargo build` clean; `cargo test --no-fail-fast` fully green — 29 binaries, 1214 passed, 0 failed
- [ ] Version bump (MINOR, 0.36.0 → 0.37.0) + `docs/release-notes.md` entry — **deferred to the completion flow**, which covers both plans 075 + 076 as a single release (see Deviations 2)

## Deviations
1. **knot-dispatch frontmatter bumped (1.3.0 → 1.4.0, compatibility → "Knot 0.37.0+").** Not literally on the plan's checklist, but follows the project convention established in plan 073 phase 5 (knot-dispatch 1.2.0 → 1.3.0 / "0.35.0+" when its content documented that version's behaviour): the skill now documents a 0.37.0-only operation (rename-as-supported-reorder), so its compatibility must state 0.37.0+.
2. **Version bump and release notes not in this phase's commit.** Per the plan's own wording ("part of plan completion via the project-plan-completion skill") and the 074/076 precedent. The knot-update changelog carries both 0.37.0 entries ahead of time; the completion flow applies the `Cargo.toml` bump, writes the release-notes entry for the combined release, merges to `main`, and marks both plans ✅ Complete.

## Discoveries
- The known `thinking_level` environment flake (recorded in the Phase 1 record) recurred once during this phase's full-suite runs, as anticipated at this gate — the final run and the orchestrator's verification gate were green. Confirms the flake is load-induced and unrelated to either plan's changes.
- With both plans' docs in place, the operator-facing story is complete: `knot-dispatch` tells operators rename reorders the FIFO (supported), `concepts.md` carries the invariant (filename stem = identity), and `knot-update` tells upgrading projects exactly what 0.37.0 does to drifted queue files (self-heal on first scan, warning per file, no migration).

## Notes
- All implementation phases are complete. Remaining work is the combined completion flow for plans 075 + 076: bump 0.36.0 → 0.37.0 (MINOR, single bump per the coordination notes), `docs/release-notes.md` entry covering both plans, merge to `main`, delete branches, update master plan index, `cargo install --path .`.
