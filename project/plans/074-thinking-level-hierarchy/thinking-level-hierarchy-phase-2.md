# Phase 2: Profile parsing, config fields, CLI emission

**Plan:** [Thinking Level — Alias Default with Profile Override](thinking-level-hierarchy-plan.md)

## Checklist
- [x] Failing tests: `parse_agent_profile` accepts `thinking-level` in frontmatter, defaults to `None`, and rejects invalid values with `AgentProfileError::InvalidThinkingLevel` — written first, confirmed red (E0433/E0599/E0609: missing variant, builder, fields)
- [x] Failing tests: `build_cli_args()` omits `--thinking` when unset and emits `--thinking <level>` between `--model <model>` and `--tools` when set (including `off`)
- [x] Add the `thinking-level` frontmatter field to `parse_agent_profile` (`src/domain/knot_file.rs`) + `AgentProfileError::InvalidThinkingLevel(String)` + Display (`agent profile has an invalid thinking-level '{value}'`)
- [x] Add `AgentProfile.thinking_level` + fluent `with_thinking_level` builder (like `with_timeout`)
- [x] Add `AgentConfig.thinking_level` (serde default, skip-if-none)
- [x] `build_cli_args` `--thinking` emission (explicit `off` emits the flag; omission emits none)
- [x] Compile and verify all Phase 2 tests green; run the full suite — 16 new tests pass; 852 lib + all integration suites green, 0 failures

## Deviations
- `resolve_for_knot` passes `self.thinking_level` through verbatim in both branches for now (plan defers the hierarchy rule to Phase 3); pinned by `resolve_for_knot_copies_profile_thinking_level`.

## Discoveries
- 18 `AgentConfig { … }` struct literals across 6 files (pi_json, pi_stdio, ports ×2, session_resume ×11, tests/adapters ×2, tests/session_resume) needed the new field — updated mechanically with a sed on the uniform `extra_args:` literal lines (the only other `extra_args:` matches were a field declaration and two assertion-message strings, which the pattern did not touch).
- `pi_json_adapter::stop_reason_error_excluded` (tests/adapters.rs, mock-CLI subprocess) failed once mid-suite and passed on immediate re-run — flaky, unrelated to this change (config there has `thinking_level: None`, so argv is unchanged).

## Notes
- Frontmatter validation reuses `ThinkingLevel::parse` (Phase 1) — the raw string is kept in `RawProfileFrontmatter` so the error carries the offending token, mirroring the registry pattern.
- Serde key is `thinking-level` on both `AgentProfile` and `AgentConfig`; omitted from serialised output when `None` (legacy profile JSON/YAML parse unchanged, pinned by the legacy-defaults tests).
