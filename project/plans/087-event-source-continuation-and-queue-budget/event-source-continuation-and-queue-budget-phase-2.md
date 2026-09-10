# Phase 2: Order, Commit, and Suppression Ordering (D3 + Observability)

**Plan:** [Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget](087-event-source-continuation-and-queue-budget-plan.md)

## Checklist

- [ ] **Confirm + document** that the continuation file is written to disk **before** the per-turn commit runs (it already is — `dispatch_self_continuation` at `handle_success` `~455` precedes the late-removal → commit → loop-peek sequence). Note the split explicitly in this phase doc's Notes: the project `git_versioner` commit (`git add -A` at `repo_root`, then `git reset -q -- rig/`) **captures** the continuation (the event dispatch dir `tie-offs/<rig>/<loom>/<event-id>/` is inside the project repo by construction, and the knot's strand dir — a project path — in the filesystem case); the rig's own git repo (plan 068) is source-only and never contains runtime artifacts
- [ ] Integration test asserting the continuation file appears in the **project's** commit after the turn (replay-by-commit source; the rig repo stays source-only)
- [ ] **Reorder suppression** in `handle_success` (`~402-483`): today `LoomEvent::TasksIncomplete` (`~433`, `continuations: N+1`) and the `[task-loop] handoff` line (`~473`) are written unconditionally for `occurred: true`, *before* `dispatch_self_continuation` returns `Ok(None)` on a cap. Reorder so that when the max-continuations cap (`next_continuations > MAX_CONTINUATIONS`) or an exhausted budget suppresses the continuation, the `BatchIncomplete` is recorded and the `LoomEvent::TasksIncomplete` + `[task-loop] handoff` line are **skipped** — so the log never claims a hop that did not happen. (e.g. pre-check the cap/budget before recording the event + log line, or return a distinguishable `Ok(None)` reason from `dispatch_self_continuation` and gate the event/log on it)
- [ ] Keep the commit-before-next-item order intact (existing late-removal → commit → loop-peek sequence)
- [ ] TDD: a **suppression ordering** test is written **red** first — on cap/budget-exhaustion, no spurious `TasksIncomplete` loom-event and no `[task-loop] handoff` line are emitted — then the reorder turns it green
- [ ] `cargo build` clean; `cargo clippy` clean; `cargo test` green

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- **Observability fix:** today, for an event-source knot the code records `LoomEvent::TasksIncomplete` (`continuations: incoming+1`) **and** writes the `[task-loop] handoff …` service-log line **before** calling `dispatch_self_continuation`, which then returned `Ok(None)`. The log/event thus *claimed* a handoff that never materialized into a file. Phase 1 makes the delivery succeed (the claim becomes true); this phase makes the claim honest in the *suppressed* case too — the event/line are now suppressed with the file.
- Replay = navigate the **project's** commits: the continuation file + the stamped queue entry appear in the commit, so a reviewer can step through the batch hop by hop.
