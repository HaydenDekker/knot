# Phase 0: Verify the current pending-set wiring

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

## Checklist
- [x] Re-confirm the provider wiring against the working tree and record it
      below (Drafting-time Verification): `AgentEventsContextProvider::scan_pending_events`
      (`src/application/usecases/context_providers.rs`) prefers
      `StrandQueueAccessor::pending_strand_paths()` when a queue reference is
      wired, falls back to `scan_dispatch_directory` otherwise, and filters by
      frontmatter `target-knot` (producer) + `extract_emitted_event_ids`
      scoping. Record any delta from the plan's verified state (the working tree
      at drafting — `c63608c` plus the in-flight uncommitted 096-series changes;
      record that base in Discoveries).
- [x] Record the queue entry's fields (`PendingEvent`, `src/domain/pending_event.rs`;
      state mirror `RigStateStrandQueueEntry`, `src/domain/entities.rs`): id,
      kind, loom_id, knot_id (**target** loom/knot — not the producer),
      strand_path, queued_at. Confirm no producer field exists.
- [x] Record the consumption/removal semantics: `pop` deletes the head disk file
      (`tie-offs/<rig>/events/*.json`); late removal via `delete` at the git
      commit point (at-least-once). Note the in-flight blind spot (popped entries
      invisible to `pending_strand_paths`).
- [x] Record the `EventsDispatched` emission site (`src/application/usecases/process_strand.rs`,
      `LoomEvent::EventsDispatched` construction) and its log-line render
      (`src/adapters/service_log.rs`) — the two touch points for Phase 4.
- [x] Decide (record in Deviations) and note for Phase 1: the
      producer-identity source for the index — frontmatter-only `target-knot`
      read (default) vs. a producer field added to the queue entry.
- [x] Decide (record in Deviations): keep or drop the dispatch-dir scan fallback
      (default: keep for queue-less contexts, capped like the queue path).
- [x] Baseline — `cargo test --lib context_providers` + `cargo test --test
      persistent_queue` + `cargo test --test pipeline` green on the untouched
      tree (the phase's pane: the existing suite green + the recorded findings).
- [x] Verification — `cargo clippy --all-targets` clean; `cargo test --lib
      context_providers` green (no files changed in this phase — findings
      committed only); commit sha `⟨see git log: the docs commit for this
      phase⟩` recorded as this phase's external-verdict request. The
      ON-COMMIT suite is not required by this item.

## Deviations
- **D1 — producer identity stays in the frontmatter (default chosen).**
  `PendingEvent` (and the `RigStateStrandQueueEntry` state mirror) store the
  *target* loom/knot only; adding a producer field would touch the queue
  schema, `state.json`, and both queue adapters for a one-key read. The
  index keeps a frontmatter-only `target-knot` read (metadata, no body, no
  `description`/`timestamp` carry-over); the domain builder stays read-free.
- **D2 — keep the dispatch-dir scan fallback, capped.** Dropping it would
  change behaviour for queue-less contexts (tests, partial wiring) with no
  scope benefit. The fallback feeds the same capped index as the queue path.

## Discoveries
- **Base:** working tree at `c63608c` + the in-flight uncommitted 096-series
  changes (the `knot-template` content-section refactor, release notes,
  `Cargo.toml` version) — the verification target the plan records. Baseline
  green: `context_providers` 9 passed, `persistent_queue` 8 passed,
  `pipeline` 19 passed; `cargo clippy --all-targets` clean.
- This repo has no `project/test/run-scope.md` and no `scripts/test-data/`
  registration infra; verification items use the targeted `cargo test`
  filters as the phase's ALWAYS-equivalent (same as plan 093).
- Three `StrandQueueAccessor` implementors: `DiskBackedEventQueue`
  (`src/adapters/outbound/disk_event_queue.rs`), `InMemoryEventQueue`
  (`src/application/in_memory_event_queue.rs`), and `OrderingQueue`
  (`tests/late_removal.rs`). A new trait method needs a **default
  implementation** or every test double churns.
- `EventsDispatched` touch points for Phase 4: emission in
  `ProcessStrand::dispatch_agent_events` (`process_strand.rs` ~L823–886,
  called from `handle_success`, `process_strand_helpers.rs` ~L412); the
  variant carries `dispatches: Vec<(event_id, consumer_knot, consumer_loom,
  file_path)>`; log-line render in `service_log.rs` ~L351. The session runs
  in `session_resume::execute_with_resume` inside `resolve_and_execute`
  (`process_strand_helpers.rs`) — the pre-session queue snapshot for the
  `queue-busy` comparison is taken there.
- Confirmed: `pop` removes the head `events/*.json` file; the in-flight
  blind spot (popped entries invisible to `pending_strand_paths`) holds.
- `ProcessStrand` holds `strand_queue: Option<Arc<dyn StrandQueueAccessor>>`
  (line 71) — the accessor is available at both the snapshot and dispatch
  sites, so Phase 4 needs no new wiring.

## Notes
- This phase changes no files except this document; it commits findings only.
