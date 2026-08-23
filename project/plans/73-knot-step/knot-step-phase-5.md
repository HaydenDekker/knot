# Phase 5: Documentation, skills, PRD, changelog

**Plan:** [Knot Step — Single-Event Stepping and Late Queue Removal](knot-step-plan.md)

## Checklist
- [x] Commit the outstanding uncommitted `knot-dispatch` event-enforcement wording (plan 059, complete 2026-07-14) as its own commit — done, commit `f1c1c19`, so the phase-5 commit is clean
- [x] `docs/concepts.md` — new **Event Queue** section (after The Processing Flow): at-least-once; event file removed just-before-commit on success and at the point of failure on failure/skip; exactly-once removal invariant; in-flight events survive a crash and are re-queued (idempotent re-run); short `knot step` pointer
- [x] `knot-dispatch` skill (v1.2.0 → 1.3.0, compatibility Knot 0.35.0+) — new **Stepping: `knot step`** section (single-event stepping, `--rig`, `--event` resolution incl. ambiguity, empty-queue behaviour + 5× debounce wait, exit codes, service-not-running caveat, what a step does, how to get an event queued incl. direct queue write, agent workflow, quick-reference commands) + fixed stale pop-removal wording (3 places) + fixed stale 500ms debounce value (3 places — actual default is 100ms) + Prerequisites clarifier
- [x] `knot-update` skill (v1.10.0 → 1.11.0, compatibility Knot 0.35.0+) — new 0.35.0 changelog entry at top: `step` command + late removal; before/after table; **no document migration — pending events from older versions read identically** (queue JSON schema unchanged)
- [x] `project/prds/prd-persistent-events.md` — the popped-removal goal revised to late-removal semantics; new **Story 6: Step Through the Queue Manually** (6 scenarios); success-criterion line "removed on pop" revised to match
- [x] `knot-manage` (v1.1.0 → 1.2.0), `knot-analyst` (v1.3.0 → 1.4.0), `knot-init` glossary (skill v4.1.0 → 4.2.0), `project/domain-glossary.md` — fixed stale "removed when popped / auto-removed on pop" wording and the `pop()`-reads-fresh reference (now `front()`); domain-glossary Last Updated bumped
- [x] `project/plans/master-plan.md` — Last Updated note now "phases 1-5 complete; awaiting plan completion"
- [x] Consistency verification — grep for stale queue-removal wording across docs/skills/project: clean (remaining "popped" hits are the historical v0.30.0 release notes, the new "before 0.35.0" changelog wording, and the PRD's original design-decision discussion — all intentional); `cargo check` green; no Rust changes in this phase so no test run (suite green as of phase 4 commit `cb15433`)

## Deviations
- **Scope addition — stale wording beyond the four named files** (justified by plan target 5: "Docs, skills, PRD, and changelog are corrected to the new semantics"): the pop-removal wording also appeared in `knot-manage`, `knot-analyst`, the `knot-init` glossary, and `project/domain-glossary.md`; the knot-dispatch skill also carried a stale 500ms debounce window (actual `DEFAULT_DEBOUNCE_WINDOW` is 100ms). All fixed.
- **Outstanding plan-059 work committed separately** (`f1c1c19`): the working tree held uncommitted knot-dispatch event-enforcement wording from plan 059 (complete 2026-07-14) that predated the phase 4 commit; committing it separately kept the phase-5 commit scoped to this plan.
- **Version bump and release notes deferred to plan completion** — per the plan, the Cargo.toml MINOR bump (0.34.0 → 0.35.0), `docs/release-notes.md` entry, design-document extraction, and global skill publication happen via the project-plan-completion skill. The knot-update 0.35.0 entry and the skills' `compatibility: "Knot 0.35.0+"` reference the version that bump will land.

## Discoveries
- **`project/domain-glossary.md` is stale beyond this plan.** Its Events Directory entry (and State Writer entry) still use pre-0.31.0 `rig/` paths (`rig/events/`, `rig/state.json`) — stale since the rig/project repository split. Only the 0.35.0 removal-timing lines were corrected; a full path re-root of that glossary is a follow-up.
- **The PRD's "Design decision" lines (push/pop persistence, HTTP DELETE) record the original design discussion** and were left as-is — the plan scoped the PRD revision to the one goal plus the new user story.
- **`docs/release-notes.md` v0.30.0 entry** ("When an event is processed (popped), its file is removed from disk") is a historical record of that release and was left as-is; the 0.35.0 release notes at plan completion supersede it.
- **Step stdout contract** (pinned in the skill from `step_execute_one`): `[step] processing event <id> (loom=<loom>, knot=<knot>): <path>` then `[step] event <id> processed` on stdout, or `[step] event <id> failed: …` on stderr; empty queue prints `queue empty` (exit 0).

## Notes
- Skill version bumps this phase: knot-dispatch 1.2.0 → 1.3.0 (compat 0.35.0+), knot-update 1.10.0 → 1.11.0 (compat 0.35.0+), knot-manage 1.1.0 → 1.2.0, knot-analyst 1.3.0 → 1.4.0, knot-init 4.1.0 → 4.2.0 (glossary content).
- Global skill publication (`cp -r .agents/skills/<skill> ~/.agents/skills/<skill>`) deliberately **not** done this phase — the plan assigns it to plan completion alongside the version bump.
- The direct-queue-write recipe in the skill (`{unix_timestamp_ms}-{4hex}.json` in `tie-offs/<rig>/events/`, filename-sort FIFO) mirrors the phase 4 test fixture technique — the disk is the queue and `load_persisted()` runs at step startup, so a hand-written event file is picked up without a watcher.
