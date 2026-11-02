# Phase 5: Retire old invariants + changelog

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [x] Update `agent_config_build_cli_args_no_system_prompt_flag` + `profile_ref_cli_args_no_system_prompt_flag`: keep the no-REPLACE invariant, note the append flag is adapter-level (positive assertions live in adapter tests)
- [x] `knot-update` changelog entry: prompt placement change — instruction blocks move to the system prompt; first-user-message parsers must read `tie-offs/<rig>/prompts/` instead
- [x] Full `cargo test` + `cargo clippy` green
- [x] Verification — whole suite green, clippy clean

## Deviations
- [x] DONE — the frozen-artifact directory is `tie-offs/<rig>/system-prompts/`
      (not `prompts/` as the checklist sketch said); the changelog and the
      Phase 3 code agree on `system-prompts/`.
- [x] The two named tests keep their no-REPLACE assertion (`--system-prompt`
      must never appear — it would clobber pi's base prompt) and are
      re-commented as the plan 103 INVERTED contract: the standing prompt
      rides `--append-system-prompt` at the adapter level; `build_cli_args`
      carries neither the persona nor the replace flag. Positive
      `--append-system-prompt` delivery is asserted by the Phase 1/2
      adapter tests (`*_delivers_system_prompt_via_append_flag`).
## Discoveries
- Version bumped 0.65.0 → 0.66.0. The change is transparent to rigs
  (no document-format change), so the changelog entry is a
  placement/observability note, not a migration.
- Clippy: 0 errors; the remaining warnings are pre-existing unused-imports
  in files this plan did not touch.
## Notes
