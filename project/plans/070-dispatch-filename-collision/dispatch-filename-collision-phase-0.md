# Phase 0: Filename derivation helper (TDD)

**Plan:** [Unique Event Dispatch Filenames — Per-Batch Sequence Suffix](dispatch-filename-collision-plan.md)

## Checklist
- [ ] Write failing tests for `event_file_name(timestamp, seq)` in `src/adapters/outbound/event_dispatcher.rs`: `seq = 0` → `event-{ts}.md`, `seq = 1`/`2`/`999` → `event-{ts}-{seq:03}.md`, colon/space substitution preserved
- [ ] Implement pure helper `pub(crate) fn event_file_name(timestamp: &str, seq: u32) -> String`
- [ ] Refactor `dispatch()` to call the helper with `seq = 0` — on-disk names byte-identical to today
- [ ] Compile and verify no errors
- [ ] Run full lib test suite

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
