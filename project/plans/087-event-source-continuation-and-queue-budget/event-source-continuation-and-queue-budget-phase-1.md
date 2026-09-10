# Phase 1: Event-Source Delivery (D1)

**Plan:** [Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget](087-event-source-continuation-and-queue-budget-plan.md)

## Checklist

- [x] In `dispatch_self_continuation` (`src/application/usecases/process_strand_helpers.rs` `~936`), replace the v1 short-circuit:

      ```rust
      let strand_dir = match &knot.strand_source {
          StrandSource::Filesystem(path) => path.clone(),
          _ => { /* v1: return Ok(None) */ }
      };
      ```

      with an event-source target resolution:

      ```rust
      use crate::domain::knot_file::derive_runtime_root;
      let target_dir = match &knot.strand_source {
          StrandSource::Filesystem(path) => path.clone(),
          StrandSource::EventUri { event_id, .. } =>
              derive_runtime_root(&ps.rig_dir).join(loom_id.0).join(event_id),
      };
      ```

      (All inputs are already in scope: `ps.rig_dir`, `loom_id`, and the destructured `event_id` field — the same `derive_runtime_root` + `EventUri { event_id }` shape `ensure_event_uri_watch` in `src/application/usecases/loom/mod_watchers.rs:79` and `FileSystemEventDispatcher::dispatch` already use.)
- [x] Keep the existing `create_dir_all` + `fs::write` unchanged — the file lands in the watched dir and is picked up by the existing watcher → debounce → `DiskBackedEventQueue` path (appended to the end, FIFO)
- [x] Filesystem-strand behaviour is **preserved**: the continuation still lands in the knot's own strand dir (asserted in `dispatch_self_continuation_filesystem_lands_in_strand_dir`)
- [x] The `[task-loop] handoff` service-log line gains a `source=filesystem|event` tag so the two delivery paths are greppable
- [x] TDD: an **event-source continuation** test is written **red** first — an event-source knot whose tie-off declares `TasksIncomplete occurred: true` produces a continuation file at `derive_runtime_root(rig_dir)/<loom_id>/<event_id>/` (and the existing filesystem test still passes) — then the implementation turns it green
- [x] `cargo build` clean; `cargo clippy` clean; `cargo test` green

## Deviations

- **Test seam is `dispatch_self_continuation` directly, not `execute()`**: the delivery is a pure function of `(ps.rig_dir, knot.strand_source, loom_id)` plus the stamped budget/start, so the unit test calls `dispatch_self_continuation` on a `ProcessStrand` built with a **writable temp `rig_dir`** (new `build_process_strand_at` helper; the existing `build_process_strand` hardcodes `/rig`, whose runtime root is not writable in tests). The event-source test asserts the file path prefix (`derive_runtime_root(rig_dir)/<loom_id>/<event_id>/`) **and** the stamped front-matter (`event-id`, `target-knot`, `continuations: 1`, `budget-secs: 1200`, `batch-start-epoch`, `## Next Task Context`). The end-to-end re-trigger (watcher → debounce → queue → re-run) is the existing, already-covered dispatch pipeline (plan 065/068 seams), so no new integration seam is needed in this phase.
- **`PathBuf::join` takes `AsRef<Path>` by value** — `loom_id.0` behind a `&LoomId` must be `join(&loom_id.0)` (a reference), not `join(loom_id.0)` (a move out of a shared reference). The plan snippet's `.join(loom_id.0)` would not compile as written.
- **`#[allow(clippy::too_many_arguments)]` on neither function changed** — the signature of `dispatch_self_continuation` is unchanged by this phase (Phase 0's 8-arg shape with the stamped budget/start carries through).
- Net clippy diff vs Phase-0 baseline: **zero new warning bodies** (verified by normalised before/after clippy output: 387 = 387 total warnings, identical multiset of warning bodies).

## Discoveries

- The v1 short-circuit was the **only** `Ok(None)` return path left for a non-caps case: after this phase, `dispatch_self_continuation` returns `None` **only** when the `MAX_CONTINUATIONS` cap suppresses the hop (which records `BatchIncomplete (reason: caps)`). The Phase 2 suppression-ordering change (skipping the `LoomEvent::TasksIncomplete` + handoff log line on suppression) is the remaining observability gap — today the event/line are still written before the caps check.
- The `source=` tag is derived from `knot.strand_source` in `handle_success` (where the `[task-loop] handoff` line is emitted), not from the dispatch result — it tags the *delivery path of the knot*, independent of whether the caps check suppresses the file.
- Full suite: **1345 passed, 0 failed** (1343 + 2 new delivery tests).

## Notes

- **Delivery is knot-scoped:** the watch is bound per `(loom_id, knot_id)`, so the continuation re-triggers the knot that produced it — the cross-fire a loom-level subscription would cause is eliminated (086's knot-scoped matching).
- **Shared event-id (accepted in v1):** the event dispatch dir `tie-offs/<rig>/<loom>/<event-id>/` is per `(loom, event-id)`, not per knot. If another knot in the same loom subscribes to the same `event-id`, it watches the same dir, so a continuation written there would also re-trigger *it* (identical to how a normal dispatch fans out — see `dispatch_fan_out_two_consumers_same_event`). Locked decision: accept and document this in v1; a knot-scoped subdir would change the watcher contract and is out of scope. Checked 2026-09-10 against this repo's rig: no loom has two knots on the same event-id and no event-source knot exists, so the cross-fire case does not arise here.
- This phase closes the **core v1 gap**: `dispatch_self_continuation` previously returned `Ok(None)` for event-source knots, so the batch paused with work remaining — the `TasksIncomplete` loom event and `[task-loop] handoff` log line now always correspond to a written continuation (modulo the caps suppression, fixed by Phase 2's ordering).
- TDD evidence: `dispatch_self_continuation_event_source_lands_in_event_dir` was written and **failed red** before the implementation (panic at the v1-gap assertion: "an event-source knot whose work remains must get a continuation (v1 gap)"), while `dispatch_self_continuation_filesystem_lands_in_strand_dir` passed before and after (pinning the preserved behaviour). Both green after the short-circuit replacement.
