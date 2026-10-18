# Phase 4: `queue-busy` observation on dispatch

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 3 is green and committed.**

## Checklist
- [ ] At tie-off dispatch, compare the queue count for the dispatched event
      type(s) before/after the session; if the queue *gained* same-type entries
      for that knot during the session, append a `queue-busy=<n>` note to the
      `EventsDispatched` log line. No extra agent turn, no bounce-back.
- [ ] Thread the observation through the `EventsDispatched` emission site
      (`src/application/usecases/process_strand.rs`, where
      `LoomEvent::EventsDispatched` is built) and its log-line render
      (`src/adapters/service_log.rs`) — the two touch points recorded in Phase 0.
- [ ] Keep the existing `EventsDispatched` line format stable for the
      no-growth case (the note is additive); confirm the render test still
      matches.
- [ ] Write the dispatcher log-line test(s): a queue that grew for the
      dispatched type during the session → the `EventsDispatched` line carries
      `queue-busy=<n>`; a queue that did not grow → no note. (The phase's pane:
      the dispatcher log-line test.)
- [ ] Verification — `cargo clippy --all-targets` clean; `cargo test --lib
      service_log` + the targeted `process_strand` dispatch test green; commit
      sha `⟨TBD⟩` recorded as this phase's external-verdict request. The
      ON-COMMIT suite is not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
- This is the passive observability that decides whether a parse-time re-ask
  (explicitly out of scope here) is ever warranted.
