# Phase 3: Acceptance test — producer fan-out to consumer processing

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [x] Extend `tests/helpers.rs` `ProcessStrandBuilder` with `with_real_event_dispatcher(rig_dir: PathBuf)` — wires the real `FileSystemEventDispatcher` and passes the real rig dir to `ProcessStrand::new` (default remains the mock + `/rig`); result gains `rig_dir: Option<PathBuf>`. Real-dispatcher mode takes precedence over `with_tracking_event_dispatcher` if both are set
- [x] New `tests/event_fanout.rs` acceptance test `fan_out_four_events_same_second_all_delivered_and_processed`: producer loom `retest-loom` + knot `retest-validator`; mock agent tie-off with **four `ValidationFail` blocks** (distinct CI payloads, one shared event id); consumer loom `uat-gap-assessment-loom` + one knot `gap-assessor` subscribed to `ValidationFail`
- [x] Asserts 4 event files in `tie-offs/<rig>/uat-gap-assessment-loom/ValidationFail/` with 4 distinct `event-*.md` names and 4 distinct payloads (each file carries exactly one CI; all four CIs present)
- [x] Asserts the `EventsDispatched` loom-log entry (producer loom/knot) lists 4 dispatches
- [x] Drives the consumer side: `StrandEvent::Created` per created event file → consumer processes **4 strands** (4 `KnotCompleted` entries for `gap-assessor`); `extract_event_metadata` on each event file returns event-id `ValidationFail` + source knot `retest-validator`
- [x] Compile and verify no errors (one `mut`-binding warning in the new test fixed)
- [x] Run full test suite — 1076 passed across 24 ok targets (1 pre-existing lib failure)

## Deviations
- `extract_event_metadata` is not exported from the lib — added `pub use strand_event_metadata::{extract_event_metadata, extract_expected_event_ids};` to `src/application/usecases/mod.rs` (module stays private; functions re-exported, matching the module's existing re-export style).

## Discoveries
- `ProcessStrand::execute` does not validate the strand path against the knot's `strand-dir` — routing is the watcher's job; the use case only runs the generic strand-file check (text/temp/missing). So driving the consumer side with `StrandEvent::Created` per event file is a faithful simulation of the per-path notify events.
- The acceptance test is timing-agnostic: if the four dispatches straddle a wall-clock second, the plain names differ anyway, so "4 distinct files" still holds. The deterministic same-second coverage lives in the Phase 1 (seq assignment) and Phase 2 (taken-name fallback) unit tests; together they close the incident regardless of timing.
- The mock agent replays the same four-block tie-off for the consumer's own runs — harmless here because no knot subscribes to `gap-assessor`'s events (no listener context, no dispatch, no enforcement).

## Notes
- `with_real_event_dispatcher` deliberately replaces (not augments) the mock: a test either observes dispatches via the mock's recorded tuples or via the filesystem — not both.
