# Phase 0: Domain — StrandSource resolution methods and tests

**Plan:** [Loom-Level Event Subscriptions](loom-level-events-plan.md)

## Checklist
- [x] Add `EventSubscription` enum to `value_objects.rs` with `KnotLevel` and `LoomLevel` variants
- [x] Add `is_loom_target(&self) -> bool` method to `StrandSource`
- [x] Add `resolve_for_producer(&self, ..., ...) -> Option<EventSubscription>` method to `StrandSource`
- [x] Write test: `strand_source_from_str_loom_uri`
- [x] Write test: `strand_source_is_loom_target_true_for_loom_suffix`
- [x] Write test: `strand_source_is_loom_target_false_for_knot_name`
- [x] Write test: `strand_source_resolve_for_producer_knot_match`
- [x] Write test: `strand_source_resolve_for_producer_loom_match`
- [x] Write test: `strand_source_resolve_for_producer_no_match`
- [x] Write test: `strand_source_resolve_loom_suffix_takes_precedence`
- [x] Compile and verify no errors
- [x] Run full test suite (all existing StrandSource tests continue to pass — 65/65 lib tests green)

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
