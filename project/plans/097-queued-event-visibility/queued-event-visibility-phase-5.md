# Phase 5: End-to-end verification

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 4 is green and committed.**

## Checklist
- [ ] Add a full-pipeline integration test (`tests/` — alongside
      `pipeline.rs` / `persistent_queue.rs`): producer emits → the entry lands
      in the queue → producer re-enters (same strand, `Modified`) → the
      **queued** entry (not a directory file) appears in the prompt with the
      norm; a mock runner shows the re-entry emitting `event: None` when the
      queued event covers the same facts.
- [ ] Assert the cap end-to-end: seed a backlog beyond `PENDING_INDEX_CAP` and
      confirm the prompt lists count + capped newest-N, not all N.
- [ ] Assert omit-when-empty end-to-end: no queued entry of the knot's
      emittable types → no `## Queued Events` section in the prompt.
- [ ] Verification — `cargo clippy --all-targets` clean; the new integration
      test green (the integration test — the phase's pane) + `cargo test --test
      pipeline` still green; commit sha `⟨TBD⟩` recorded as this phase's
      external-verdict request. The ON-COMMIT suite is not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- Per AGENTS.md, do NOT start the live service or trigger a live rig run in
  this repository — the integration test (mock runner) is the acceptance
  vehicle, and live runs are performed elsewhere.
