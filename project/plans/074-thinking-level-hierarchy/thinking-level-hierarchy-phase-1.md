# Phase 1: ThinkingLevel value object + registry parsing

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [x] Failing tests: `ThinkingLevel` serde round-trips against the YAML tokens (`off`, `minimal`, `low`, `medium`, `high`, `xhigh` — the `xhigh` variant needs an explicit `#[serde(rename = "xhigh")]`; blanket kebab-case would yield `x-high`); `Display` yields the exact CLI token; unknown tokens are rejected — written first, confirmed red (E0425/E0433: type not found)
- [x] Failing tests: `ModelRegistry::from_yaml` accepts `thinking-level` on an alias, leaves `None` when absent, and rejects an invalid value with `InvalidThinkingLevel` naming the alias and the bad value
- [x] Implement the `ThinkingLevel` enum (`off | minimal | low | medium | high | xhigh`) in `src/domain/value_objects.rs` — `Copy`, `rename_all = "kebab-case"` + explicit `#[serde(rename = "xhigh")]` on `XHigh`
- [x] Add `ModelRef.thinking_level: Option<ThinkingLevel>` (serde `thinking-level`, default, skip-if-none)
- [x] Add `ModelRegistryError::InvalidThinkingLevel { alias, value }` + Display (`model registry alias '{alias}' has an invalid thinking-level '{value}'`)
- [x] String→enum conversion in `from_yaml` via `ThinkingLevel::parse` — raw entry stays `Option<String>` (precise error, matching the existing provider/model pattern)
- [x] Update existing `ModelRef { … }` struct literals for the new field (value_objects tests ×2, process_strand test helper, write_state test)
- [x] Compile and verify all Phase 1 tests green; run the full suite — 8 new tests pass; 836 lib + all integration suites green, 0 failures

## Deviations
None — implemented exactly as planned.

## Discoveries
- `ThinkingLevel::parse` (exact, case-sensitive, no trim) is the lexical validator; `from_yaml` uses it directly so the error carries the raw offending value. `Display` is the inverse — the exact `pi --thinking` token.
- Baseline was fully green at start (the pre-existing `build_listener_context_prompt_includes_do_not_edit_guidance` failure noted in the 072 phase doc was already fixed in 59dd4c6).
- Three `ModelRef` struct literals outside `value_objects.rs` needed the new field (process_strand.rs `registry_from` test helper, write_state.rs test) — the plan anticipated these.

## Notes
- Enum is `Copy` — the Phase 3 `profile.or(alias)` rule can pass levels by value without clone noise.
