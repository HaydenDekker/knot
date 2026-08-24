# Phase 3: Hierarchy resolution in resolve_for_knot

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [ ] Failing tests: the full `resolve_for_knot` matrix — (profile, alias) ∈ {None, None} → None; {Some, None} → profile; {None, Some} → alias; {Some, Some} → profile (override)
- [ ] Failing tests: direct-spec profiles resolve their own level with the registry unconsulted; an alias without a level still lets the profile's level through
- [ ] Implement the `profile.or(alias)` rule in the `model-ref` branch of `resolve_for_knot`
- [ ] Compile and verify all Phase 3 tests green; run the full suite

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
