# Phase 2: Profile parsing, config fields, CLI emission

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [ ] Failing tests: `parse_agent_profile` accepts `thinking-level` in frontmatter, defaults to `None`, and rejects invalid values with `AgentProfileError::InvalidThinkingLevel`
- [ ] Failing tests: `build_cli_args()` omits `--thinking` when unset and emits `--thinking <level>` between `--model <model>` and `--tools` when set (including `off`)
- [ ] Add the `thinking-level` frontmatter field to `parse_agent_profile` (`src/domain/knot_file.rs`) + `AgentProfileError::InvalidThinkingLevel(String)` + Display
- [ ] Add `AgentProfile.thinking_level` + fluent `with_thinking_level` builder (like `with_timeout`)
- [ ] Add `AgentConfig.thinking_level` (serde default, skip-if-none)
- [ ] `build_cli_args` `--thinking` emission (explicit `off` emits the flag; omission emits none)
- [ ] Compile and verify all Phase 2 tests green; run the full suite

## Deviations
<!-- Record any deviations from the original plan -->

## Discoveries
<!-- Record any new information found during implementation -->

## Notes
<!-- Implementation notes, gotchas, lessons learned -->
