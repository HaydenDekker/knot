# Phase 4: State visibility

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [x] Failing tests: a written state snapshot shows the effective `thinking-level` on profile entries in all three shapes — profile-only, alias-default, and omitted when neither sets one (key absent, not null, matching the `timeout` convention) — written first, confirmed red (E0609: no field `thinking_level` on `RigStateProfile`); also added a profile-override case (profile level beats alias level) beyond the three required shapes
- [x] Add the `RigStateProfile` field (serde `thinking-level`, skip-if-none)
- [x] Implement the effective-level mapping in `write_state.rs`
- [x] Compile and verify all Phase 4 tests green; run the full suite — 4 new tests pass; 862 lib + all integration suites green, 0 failures

## Deviations
- **Unresolvable-alias edge case** (not explicit in the plan): when a `model-ref` profile's alias is missing from the registry, state shows the profile's own level (the alias default is unavailable) alongside the existing `null` provider/model — consistent with how that branch already degrades.

## Discoveries
- Eight pre-existing `RigStateProfile` struct literals needed the new field: four in `entities.rs` tests, two in `state_writer.rs` tests, and two in `tests/adapters.rs` integration tests. Updated via indentation-aware `sed`/`perl` anchored on the `model:`/`timeout:` lines (a blanket `timeout:` match was unsafe — an unrelated struct in `tests/adapters.rs` also has a `timeout:` field).
- The effective level is computed in the same `match` as model selection (now a 4-tuple `(model_ref, provider, model, thinking_level)`), so it stays in lockstep with how provider/model are resolved.

## Notes
- Key is omitted entirely when `None` (`skip_serializing_if`), never `null` — pinned by `build_state_thinking_level_omitted_when_neither_sets_it` asserting the JSON does not contain `thinking-level` at all.
- `process_strand.rs` is unchanged, per the plan — it already calls `resolve_for_knot` (Phase 3) for the actual invocation; only the state snapshot needed the mapping.
