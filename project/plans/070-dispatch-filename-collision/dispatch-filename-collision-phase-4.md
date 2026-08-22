# Phase 4: Observability + documentation

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [ ] Extend `LoomEvent::EventsDispatched.dispatches` to `Vec<(String, String, String, String)>` (4th = created file path, absolute); update doc comment
- [ ] Carry the `dispatch()` return path through `dispatch_events_to_consumers` into the `EventsDispatched` entry
- [ ] Update all `EventsDispatched` consumers/tests (process_strand tests, loom-log readers) for the 4-tuple
- [ ] Loom-log test: legacy 3-tuple `EventsDispatched` line is skipped with a warning while subsequent lines read fine (0.30.1-style graceful degradation)
- [ ] Update doc comments: `ports.rs` port docs, `event_dispatcher.rs` module docs, `events.rs` event docs — new filename contract `event-{ts}[-NNN].md`
- [ ] `knot-update` changelog entry: `EventsDispatched` tuple expansion (internal runtime artifact, no migration, warnings self-settle) + new `event-{ts}-NNN.md` fan-out filename shape
- [ ] Record separator decision (hyphen) in this phase doc
- [ ] Compile and verify no errors
- [ ] Run full test suite (lib + integration)

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
