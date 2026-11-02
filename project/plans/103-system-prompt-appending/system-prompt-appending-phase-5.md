# Phase 5: Retire old invariants + changelog

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [ ] Update `agent_config_build_cli_args_no_system_prompt_flag` + `profile_ref_cli_args_no_system_prompt_flag`: keep the no-REPLACE invariant, note the append flag is adapter-level (positive assertions live in adapter tests)
- [ ] `knot-update` changelog entry: prompt placement change — instruction blocks move to the system prompt; first-user-message parsers must read `tie-offs/<rig>/prompts/` instead
- [ ] Full `cargo test` + `cargo clippy` green
- [ ] Verification — whole suite green, clippy clean

## Deviations
## Discoveries
## Notes
