# Phase 4: `queue-busy` observation on dispatch

**Plan:** [Queue-Backed Pending-Event Visibility for Producers](queued-event-visibility-plan.md)

**Runs only after Phase 3 is green and committed.**

## Checklist
- [x] At tie-off dispatch, compare the queue count for the dispatched event
      type(s) before/after the session; if the queue *gained* same-type entries
      for that knot during the session, append a `queue-busy=<n>` note to the
      `EventsDispatched` log line. No extra agent turn, no bounce-back.
- [x] Thread the observation through the `EventsDispatched` emission site
      (`src/application/usecases/process_strand.rs`, where
      `LoomEvent::EventsDispatched` is built) and its log-line render
      (`src/adapters/service_log.rs`) — the two touch points recorded in Phase 0.
- [x] Keep the existing `EventsDispatched` line format stable for the
      no-growth case (the note is additive); confirm the render test still
      matches.
- [x] Write the dispatcher log-line test(s): a queue that grew for the
      dispatched type during the session → the `EventsDispatched` line carries
      `queue-busy=<n>`; a queue that did not grow → no note. (The phase's pane:
      the dispatcher log-line test.)
- [x] Verification — `cargo clippy --all-targets` clean (0 errors; warning
      profile unchanged — the one hit is the pre-existing
      `too_many_arguments` on `handle_success`, now 12/7 vs. 11/7);
      `cargo test --lib service_log` (41 passed) + the targeted
      `process_strand` dispatch tests (115 passed, incl. 3 new) +
      `cargo test --lib events` (139 passed) + full lib (1148 passed) +
      `cargo test --test pipeline` (19) + `--test persistent_queue` (8) green;
      commit sha `⟨TBD⟩` recorded as this phase's external-verdict request. The
      ON-COMMIT suite is not required by this item.

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
- **Note format (concretisation of `queue-busy=<n>`):** the note is
  per-dispatched-event-type and type-qualified:
  `queue-busy=<event-type>:<n>` (e.g. `queue-busy=PlanCreated:2`), appended
  after the `dispatch=…` entries, sorted by event id. One `EventsDispatched`
  line can dispatch several types, so an unqualified count would be
  ambiguous.
- **Carried on the event variant:** `LoomEvent::EventsDispatched` gains
  `queue_busy: Vec<(String, u32)>` with `#[serde(default,
  skip_serializing_if = "Vec::is_empty")]` — the field is omitted from JSON
  (and the rendered line) when empty, so the no-growth line stays
  byte-identical to the pre-0.61.0 format. Legacy activity entries (no field
  in the JSON) deserialize as empty — graceful degradation, same precedent as
  the 3-tuple → 4-tuple dispatch expansion in 0.33.0 (pinned by
  `loom_event_events_dispatched_queue_busy_serde`).
- **Baseline capture point:** the per-type before-count
  (`count_event_types` over `pending_strand_paths()`) is taken at session
  start in `execute_inner` (before config resolution / agent run), threaded
  through `handle_success` into `dispatch_agent_events` as
  `queue_before: Option<&HashMap<String, usize>>`.
- **Own dispatches are not counted as "gained during the session":** the
  session's own dispatched entries land in the queue *after* the session
  (this is the after-moment — the debounced watcher has normally not queued
  them yet when the after-count is read), so `n = after − before` (saturating)
  measures queue growth *during* the session from other sources (debounced
  backlog files materialising during a long session, external touches,
  parallel producers). If the watcher *has* already queued the session's own
  entries, the note over-counts by at most the session's own dispatch count —
  an accepted bias for a passive observation (documented in the code).
- **Degenerate-case note:** with no queue wired (test fixtures, queue-less
  contexts) or with a queue that did not grow, `queue_busy` is empty and the
  line is unchanged — the observation is strictly additive.

## Notes
- This is the passive observability that decides whether a parse-time re-ask
  (explicitly out of scope here) is ever warranted.
- The current (in-flight) strand is itself a queue entry until late removal;
  its type is what the knot *consumed*, so it normally does not interfere
  with the counts of the types the knot *emits* (and it cancels out in the
  before/after difference even when a knot consumes and emits the same type).
