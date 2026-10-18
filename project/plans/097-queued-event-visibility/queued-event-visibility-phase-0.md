# Phase 0: Verify the current pending-set wiring

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

## Checklist
- [ ] Re-confirm the provider wiring against the working tree and record it
      below (Drafting-time Verification): `AgentEventsContextProvider::scan_pending_events`
      (`src/application/usecases/context_providers.rs`) prefers
      `StrandQueueAccessor::pending_strand_paths()` when a queue reference is
      wired, falls back to `scan_dispatch_directory` otherwise, and filters by
      frontmatter `target-knot` (producer) + `extract_emitted_event_ids`
      scoping. Record any delta from the plan's verified state (the working tree
      at drafting — `c63608c` plus the in-flight uncommitted 096-series changes;
      record that base in Discoveries).
- [ ] Record the queue entry's fields (`PendingEvent`, `src/domain/pending_event.rs`;
      state mirror `RigStateStrandQueueEntry`, `src/domain/entities.rs`): id,
      kind, loom_id, knot_id (**target** loom/knot — not the producer),
      strand_path, queued_at. Confirm no producer field exists.
- [ ] Record the consumption/removal semantics: `pop` deletes the head disk file
      (`tie-offs/<rig>/events/*.json`); late removal via `delete` at the git
      commit point (at-least-once). Note the in-flight blind spot (popped entries
      invisible to `pending_strand_paths`).
- [ ] Record the `EventsDispatched` emission site (`src/application/usecases/process_strand.rs`,
      `LoomEvent::EventsDispatched` construction) and its log-line render
      (`src/adapters/service_log.rs`) — the two touch points for Phase 4.
- [ ] Decide (record in Deviations) and note for Phase 1: the
      producer-identity source for the index — frontmatter-only `target-knot`
      read (default) vs. a producer field added to the queue entry.
- [ ] Decide (record in Deviations): keep or drop the dispatch-dir scan fallback
      (default: keep for queue-less contexts, capped like the queue path).
- [ ] Baseline — `cargo test --lib context_providers` + `cargo test --test
      persistent_queue` + `cargo test --test pipeline` green on the untouched
      tree (the phase's pane: the existing suite green + the recorded findings).
- [ ] Verification — `cargo clippy --all-targets` clean; `cargo test --lib
      context_providers` green (no files changed in this phase — findings
      committed only); commit sha `⟨TBD⟩` recorded as this phase's
      external-verdict request. The ON-COMMIT suite is not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during verification -->
- (expected) This repo has no `project/test/run-scope.md` and no
  `scripts/test-data/` registration infra; verification items use the targeted
  `cargo test` filters as the phase's ALWAYS-equivalent (same as plan 093).

## Notes
- This phase changes no files except this document; it commits findings only.
