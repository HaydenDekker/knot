# Phase 1: ThinkingLevel value object + registry parsing

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [ ] Failing tests: `ThinkingLevel` serde round-trips against the YAML tokens (`off`, `minimal`, `low`, `medium`, `high`, `xhigh` — the `xhigh` variant needs an explicit `#[serde(rename = "xhigh")]`; blanket kebab-case would yield `x-high`); `Display` yields the exact CLI token; unknown tokens are rejected
- [ ] Failing tests: `ModelRegistry::from_yaml` accepts `thinking-level` on an alias, leaves `None` when absent, and rejects an invalid value with `InvalidThinkingLevel` naming the alias and the bad value
- [ ] Implement the `ThinkingLevel` enum (`off | minimal | low | medium | high | xhigh`) in `src/domain/value_objects.rs`
- [ ] Add `ModelRef.thinking_level: Option<ThinkingLevel>` (serde `thinking-level`, default, skip-if-none)
- [ ] Add `ModelRegistryError::InvalidThinkingLevel { alias, value }` + Display
- [ ] String→enum conversion in `from_yaml` — raw entry stays `Option<String>` so the error is precise (matching the existing provider/model pattern)
- [ ] Update existing `ModelRef { … }` struct literals for the new field (value_objects tests, process_strand tests, write_state tests)
- [ ] Compile and verify all Phase 1 tests green; run the full suite

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
