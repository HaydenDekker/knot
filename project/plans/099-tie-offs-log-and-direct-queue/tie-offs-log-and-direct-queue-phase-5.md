# Phase 5: Cutover and migration

**Plan:** [Unified Tie-Off Log and Direct Event Queueing](tie-offs-log-and-direct-queue-plan.md)

**Runs only after Phase 4 is green and committed.** One planned change,
never piecemeal (Phase 0 decision D4: hard cutover, no config flag).

## Checklist
- [x] Startup detects retired-layout artefacts (per-knot tie-off files,
      per-`EventId` dispatch directories under the runtime root) and
      logs one-line `[KNOT][CONFIG]` deprecation diagnostics naming
      them — no backfill, no read; the old files stay in git history
      and the service owns the write format
      (`detect_retired_layout_artifacts` in `src/server.rs`, called
      from `run_startup`)
- [x] The `FileSystemEventDispatcher` file path retires with its
      diagnostics — the adapter is **deleted**
      (`src/adapters/outbound/event_dispatcher.rs`, 994 lines);
      `QueueEventDispatcher` is the sole delivery mechanism for agent +
      system events. `tests/event_fanout.rs` **re-targeted** to the
      queue delivery (not retired): the four-block fan-out now asserts
      four payload-bearing queue entries + the `EventsDispatched` log +
      four consumer completions driven via `TryFrom<PendingEvent>`
- [x] The manual dispatch path: the direct queue write is the sole
      manual mechanism and accepts the `Event` kind (D8) — verified by
      `tests/step.rs::manual_delivery_entry_steps_by_id`: a
      hand-written `Event`-kind entry with a `delivery` object in
      `events/` loads, delivers its payload to the agent prompt, and is
      removed after the run; `knot step --event <id>` addresses the
      entry by id (exact / prefix / strand-filename resolution
      unchanged, works on the new kind)
- [x] The trace-chain acceptance test:
      `tests/trace_chain.rs::trace_chain_links_service_log_commit_and_tie_off_log`
      — one emission traced service log → commit trailers →
      `tie-offs.log` section. Asserts the consumer's section header
      carries `event-id=` / `source=` / `session=` /
      `original-strand=`; the git commit carries the matching trailer
      block (`event:` / `source:` / `session:` / `original-strand:`);
      the session id is **identical** on both surfaces (the
      `pi --session-id` link); and **no persistent payload
      duplication** — the `tie-offs.log` section carries the slim
      `event-audit` line (not the full event block), the full event
      content lives in the commit body
- [ ] Field-rig acceptance run — the full trace chain verified end to
      on a live rig (performed elsewhere, per the repository's
      no-live-runs rule — the recorded evidence lands here)
- [x] Verification — clippy clean for the new code (the only new
      warning, an orphaned `derive_runtime_root` import, removed); the
      acceptance test + the re-targeted suites green; full suite
      **1514 passed / 0 failed**; commit sha recorded as this phase's
      external-verdict request. The ON-COMMIT suite is not required by
      this item.

## Deviations
- **`event_fanout.rs` re-targeted, not retired.** The plan offered
  either. The fan-out property (four same-second emissions all survive)
  is still worth pinning and now pins it on the queue-entry shape, so
  the suite was kept and re-targeted.
- **Test-harness queue unification.** `tests/helpers.rs` now wires the
  dispatcher's `DiskBackedEventQueue` as the `ProcessStrand`'s strand
  queue when `with_real_event_dispatcher` is used (an explicitly
  provided `with_strand_queue` still takes precedence). This mirrors
  production, where the emitter and the strand processor share the one
  `AppContext.event_queue`. Several suites depended on this to see
  deliveries in the pending-visibility index.

## Discoveries
- **The event-URI subscription format is colon-separated**
  (`event:<producer-knot>:<EventId>` in the knot's `strand-dir`),
  while the **queue descriptor is slash-separated**
  (`event:<producer-knot>/<EventId>`). The trace-chain test's consumer
  knot must use the colon form in `strand-dir:`; the descriptor that
  surfaces in the log/commit is the slash form.
- **The mock `pi` must emit valid JSON-L.** Raw newlines inside a JSON
  string break the `pi-json` parse (the whole stdout falls through as
  response text). The trace-chain mock escapes them as `\\n` so the
  `agent_end` `content[].text` parses and the event block is seen.
- **`step_execute_one`'s `ctx` parameter is already unused** (pre-
  existing warning) — the queue is passed directly; left as-is.

## Notes
- Cross-repo repointing is **not** in this phase's file set (plan
  Notes): the `knot-manage`, `knot-dispatch`, `knot-inspect`,
  `knot-analyst` skills, the `knot-template` templates, and the
  loom/profile event URIs in rig definitions land as their own planned
  changes in the `agents` project and the affected rigs.
- The read norm travels with that skills repointing, not with this
  code.

## External verdict request
**Commit sha:** `<pending>` — recorded in the follow-up docs commit.
