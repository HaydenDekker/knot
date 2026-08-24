# Phase 4: State visibility

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [ ] Failing tests: a written state snapshot shows the effective `thinking-level` on profile entries in all three shapes — profile-only, alias-default, and omitted when neither sets one (key absent, not null, matching the `timeout` convention)
- [ ] Add the `RigStateProfile` field (serde `thinking-level`, skip-if-none)
- [ ] Implement the effective-level mapping in `write_state.rs`
- [ ] Compile and verify all Phase 4 tests green; run the full suite

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
