# Phase 1: Context injection — loom-level subscriptions in `build_listener_context`

**Plan:** [Loom-Level Event Subscriptions](loom-level-events-plan.md)

## Checklist
- [x] Update `build_listener_context` signature to accept `loom_id: &LoomId`
- [x] Add loom-level matching logic (`producer_knot.ends_with("-loom") && producer_knot == loom_id.0`)
- [x] Update caller in `process_strand.rs` to pass `loom_id`
- [x] Update all 16 existing test calls to pass `loom_id`
- [x] Write test: `build_listener_context_loom_level_consumer_matches_producer_in_loom`
- [x] Write test: `build_listener_context_loom_level_consumer_no_match_different_loom`
- [x] Write test: `build_listener_context_mixed_knot_and_loom_consumers`
- [x] Write test: `build_listener_context_all_knots_in_loom_get_injection`
- [x] Compile and verify no errors
- [x] Run full test suite (20/20 build_listener_context tests green, all existing tests pass)

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
