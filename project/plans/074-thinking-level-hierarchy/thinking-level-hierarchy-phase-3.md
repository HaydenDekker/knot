# Phase 3: Hierarchy resolution in resolve_for_knot

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [x] Failing tests: the full `resolve_for_knot` matrix — (profile, alias) ∈ {None, None} → None; {Some, None} → profile; {None, Some} → alias; {Some, Some} → profile (override) — written first; the {None, Some} alias-default case confirmed red, the other three are green regression guards
- [x] Failing tests: direct-spec profiles resolve their own level with the registry unconsulted (own level with a same-named alias carrying a different level; no level with the registry carrying one)
- [x] Implement the `profile.or(alias)` rule in the `model-ref` branch of `resolve_for_knot`
- [x] Compile and verify all Phase 3 tests green; run the full suite — 6 new tests pass; 858 lib + all integration suites green, 0 failures

## Deviations
None — implemented exactly as planned.

## Discoveries
- Only the {None, Some} case was genuinely red pre-implementation (the passthrough from Phase 2 already satisfies the other three matrix cells and both direct-spec cases) — the extra tests pin the rule against future regressions.
- The match in `resolve_for_knot` now yields a `(provider, model, thinking_level)` triple so the effective level is computed at the same site as the model selection — no second pass over `model_ref` needed.

## Notes
- `ThinkingLevel: Copy` (Phase 1) makes `self.thinking_level.or(resolved.thinking_level)` a by-value operation with no clones.
- Doc comment on `resolve_for_knot` now records the effective-level rule for both branches (model-ref: `profile.or(alias)`; direct-spec: profile only, registry unconsulted).
- The Phase 2 test `resolve_for_knot_copies_profile_thinking_level` kept (direct-spec passthrough); its stale "Phase 2 scope" comment updated.
