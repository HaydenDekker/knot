# Phase 1: Port + use-case sequencing

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [ ] Add `seq: u32` parameter to `EventDispatcherPort::dispatch` in `src/application/ports.rs` (0 = plain name, `i ≥ 1` = `-{i:03}` suffix); update port doc comment
- [ ] Pass `seq` through in `FileSystemEventDispatcher` (uses `event_file_name` from Phase 0)
- [ ] Extend `MockEventDispatcher` to record `seq` (recorded tuple gains the seq element); update `get_dispatches()` and its callers
- [ ] Fix existing destructuring of recorded tuples in `process_strand.rs` tests, `tests/model_aliases.rs`, `tests/helpers.rs`
- [ ] Rewrite `dispatch_events_to_consumers` as collect → group by `(consumer_loom_id, event_id)` → dispatch with per-group seq (0 for singleton groups, 1..N for groups > 1), preserving tie-off block / loom-store iteration order within a group
- [ ] Test: 4 same-id events → 1 consumer: mock records seq 1,2,3,4
- [ ] Test: 1 event → 2 consumer knots in the same loom: seq 1,2
- [ ] Test: existing fan-out tests (2 looms, 1 event) keep passing with seq 0
- [ ] Test: single dispatch in its own directory stays seq 0
- [ ] Compile and verify no errors
- [ ] Run full test suite (lib + integration)

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
