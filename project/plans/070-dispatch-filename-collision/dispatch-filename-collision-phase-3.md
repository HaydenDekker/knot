# Phase 3: Acceptance test — producer fan-out to consumer processing

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [ ] Extend `tests/helpers.rs` `ProcessStrandBuilder` with an option to wire the real `FileSystemEventDispatcher` (against a real temp rig dir) instead of the always-mocked dispatcher
- [ ] Add acceptance test (new `tests/event_fanout.rs`): producer loom + knot; mock agent tie-off with **four `ValidationFail` blocks** (distinct CI payloads, one shared event id); consumer loom + one knot subscribed to `ValidationFail`
- [ ] Assert 4 event files in `tie-offs/<rig>/<consumer-loom>/ValidationFail/` with 4 distinct names and 4 distinct payloads
- [ ] Assert the `EventsDispatched` loom-log entry lists 4 dispatches
- [ ] Drive the consumer side: `StrandEvent::Created` per created event file → consumer processes 4 strands (4 `KnotCompleted` entries); `extract_event_metadata` on each event file returns the shared event id + producer knot
- [ ] Compile and verify no errors
- [ ] Run full test suite (lib + integration)

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
