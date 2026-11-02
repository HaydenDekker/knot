# Phase 1: pi-stdio emits `--append-system-prompt`

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [x] `PiStdioAgentRunner`: compose `system_prompt + "\n\n" + profile_prompt`; push `--append-system-prompt <path>` (verified: pi reads the arg as file contents when the path exists — resource-loader.js existsSync/readFileSync)
- [x] stdin carries only base prompt + trigger line (no profile, no instruction blocks)
- [x] Mock-CLI tests: argv contains `--append-system-prompt`; stdin excludes `# Tie-Off` / profile text; trigger line present
- [x] Verification — `cargo test` green

## Deviations
- [x] DONE — Phase 1 passes the composed system prompt as inline TEXT via
      `--append-system-prompt`; the artifact-path switch lands in Phase 3.
## Discoveries
- [x] `runner_passes_event_metadata` and `execute_captures_stdout` encoded
      the old wire (profile in stdin) — rewritten for the new placement; new
      `runner_delivers_system_prompt_via_append_flag` +
      `runner_omits_append_flag_when_system_prompt_empty` cover the flag.
## Notes
