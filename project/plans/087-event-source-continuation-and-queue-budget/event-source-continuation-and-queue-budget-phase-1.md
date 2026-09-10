# Phase 1: Event-Source Delivery (D1)

**Plan:** [Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget](087-event-source-continuation-and-queue-budget-plan.md)

## Checklist

- [ ] In `dispatch_self_continuation` (`src/application/usecases/process_strand_helpers.rs` `~936`), replace the v1 short-circuit:

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
- [ ] Keep the existing `create_dir_all` + `fs::write` unchanged — the file lands in the watched dir and is picked up by the existing watcher → debounce → `DiskBackedEventQueue` path (appended to the end, FIFO)
- [ ] Filesystem-strand behaviour is **preserved**: the continuation still lands in the knot's own strand dir (assert in the preservation test below)
- [ ] The `[task-loop] handoff` service-log line gains a `source=filesystem|event` tag so the two delivery paths are greppable
- [ ] TDD: an **event-source continuation** test is written **red** first — an event-source knot whose tie-off declares `TasksIncomplete occurred: true` produces a continuation file at `derive_runtime_root(rig_dir)/<loom_id>/<event_id>/` (and the existing filesystem test still passes) — then the implementation turns it green
- [ ] `cargo build` clean; `cargo clippy` clean; `cargo test` green

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- **Delivery is knot-scoped:** the watch is bound per `(loom_id, knot_id)`, so the continuation re-triggers the knot that produced it — the cross-fire a loom-level subscription would cause is eliminated (086's knot-scoped matching).
- **Shared event-id (accepted in v1):** the event dispatch dir `tie-offs/<rig>/<loom>/<event-id>/` is per `(loom, event-id)`, not per knot. If another knot in the same loom subscribes to the same `event-id`, it watches the same dir, so a continuation written there would also re-trigger *it* (identical to how a normal dispatch fans out — see `dispatch_fan_out_two_consumers_same_event`). Locked decision: accept and document this in v1; a knot-scoped subdir would change the watcher contract and is out of scope. Checked 2026-09-10 against this repo's rig: no loom has two knots on the same event-id and no event-source knot exists, so the cross-fire case does not arise here.
- This is the **core v1 gap** the plan closes: `dispatch_self_continuation` previously returned `Ok(None)` for event-source knots, so the batch paused with work remaining.
