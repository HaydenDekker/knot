# Phase 1: Port + use-case sequencing

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [x] Add `seq: u32` parameter to `EventDispatcherPort::dispatch` in `src/application/ports.rs` (0 = plain name, `i ≥ 1` = `-{i:03}` suffix); port docs updated to state the batch-position contract and that the adapter owns name materialisation
- [x] Pass `seq` through in `FileSystemEventDispatcher` — `event_file_name(&timestamp, seq)`; all 12 unit-test call sites pass `0` (names byte-identical to before)
- [x] Extend `MockEventDispatcher` to record `seq` — recorded tuple is now `(event, consumer_knot_id, consumer_loom_id, rig_dir, seq)`; `get_dispatches()` type updated
- [x] Fix existing destructuring: `process_strand.rs` (5 builder return types, 6 destructure sites) and `tests/event_enforcement.rs` (2 sites); `tests/model_aliases.rs` needed no changes (never destructures the tuple)
- [x] Rewrite `dispatch_events_to_consumers` as collect → group by `(consumer_loom_id, event_id)` → dispatch with per-group seq (0 for singleton groups, 1..N for groups > 1), preserving scan order (tie-off block order for events, loom-store order for consumers) within a group
- [x] Test: 4 same-id events → 1 consumer: mock records seq 1,2,3,4 — `event_dispatch_same_id_multiple_events_same_consumer_gets_sequences` (replays the incident: `retest-validator` × 4 `ValidationFail` → `uat-gap-assessment`); also asserts payload order and the 4-entry `EventsDispatched` log
- [x] Test: 1 event → 2 consumer knots in the same loom: seq 1,2 — `event_dispatch_two_consumer_knots_same_loom_get_sequences`
- [x] Test: existing fan-out tests keep passing with seq 0 — added explicit `seq == 0` assertions to `event_dispatch_full_flow`, `event_dispatch_fan_out_two_looms`, and the loom-level singleton test
- [x] Test: different event ids to the same loom stay seq 0 — `event_dispatch_different_event_ids_same_loom_stay_seq_zero`
- [x] Compile and verify no errors
- [x] Run full test suite — lib 805 passed (1 pre-existing failure), all 19 integration targets pass

## Deviations
- The group-key closure `|m: &Match<'_>| (...)` failed to compile (lifetime elision: the returned `&str`s borrow from the closure argument, not from a named lifetime). Moved the key into a `Match::group_key()` method with an explicit `&'a` lifetime instead. Same logic, cleaner lifetimes.

## Discoveries
- `Match<'a>` (local struct) + `group_key()` keeps the two-pass algorithm free of string cloning: group keys are `(&str, &str)` borrowed from the `matches` vec, which itself borrows from `all_looms`/`events`.
- The recorded mock tuple gained a 5th element rather than a named struct — consistent with the existing tuple style in this fixture; a struct would be nicer but is out of scope for this phase.

## Notes
- Dispatch order within a group = scan order = tie-off block order (events iterated first), so suffix order matches the producer's emission order, as the plan requires.
- The `EventsDispatched` dispatches vector is still the 3-tuple here — it becomes the 4-tuple (with file path) in Phase 4.
