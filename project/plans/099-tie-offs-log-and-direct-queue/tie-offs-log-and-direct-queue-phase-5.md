# Phase 5: Cutover and migration

**Plan:** [Unified Tie-Off Log and Direct Event Queueing](tie-offs-log-and-direct-queue-plan.md)

**Runs only after Phase 4 is green and committed.** One planned change,
never piecemeal (Phase 0 decision D4: hard cutover, no config flag).

## Checklist
- [ ] Startup detects retired-layout artefacts (per-knot tie-off files,
      per-`EventId` dispatch directories under the runtime root) and
      logs one-line `[KNOT][CONFIG]` deprecation diagnostics naming
      them — no backfill, no read; the old files stay in git history
      and the service owns the write format
- [ ] The `FileSystemEventDispatcher` file path retires with its
      diagnostics (the adapter and its now-orphaned behaviour
      deprecate; the queue delivery is the sole delivery mechanism for
      agent + system events); `tests/event_fanout.rs` — the last suite
      pinning the dispatch-file shape — re-targets to the queue
      delivery (or retires with the behaviour, the decision recorded
      here)
- [ ] The manual dispatch path: the direct queue write is the sole
      manual mechanism and accepts the `Event` kind (D8) — a
      hand-written entry file in `events/` with a delivery loads and
      delivers; `knot step --event <entry>` addresses queue entries by
      id — verify unchanged against the new kind
- [ ] The trace-chain acceptance test: one emission traced service log
      → commit trailers → `tie-offs.log` section, asserting no
      persistent payload duplication (the event body appears in
      exactly one durable place: the commit) and that the
      `session:` field links to `pi --session-id`
- [ ] Field-rig acceptance run — the full trace chain verified end to
      on a live rig (performed elsewhere, per the repository's
      no-live-runs rule — the recorded evidence lands here)
- [ ] Verification — clippy clean for the new code; the acceptance
      test + the re-targeted suites green; commit sha recorded as this
      phase's external-verdict request. The ON-COMMIT suite is not
      required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- Cross-repo repointing is **not** in this phase's file set (plan
  Notes): the `knot-manage`, `knot-dispatch`, `knot-inspect`,
  `knot-analyst` skills, the `knot-template` templates, and the
  loom/profile event URIs in rig definitions land as their own planned
  changes in the `agents` project and the affected rigs.
- The read norm travels with that skills repointing, not with this
  code.
