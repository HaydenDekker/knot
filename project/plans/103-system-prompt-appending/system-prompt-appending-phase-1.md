# Phase 1: pi-stdio emits `--append-system-prompt`

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [ ] `PiStdioAgentRunner`: compose `system_prompt + "\n\n" + profile_prompt`; push `--append-system-prompt <path>` (verified: pi reads the arg as file contents when the path exists — resource-loader.js existsSync/readFileSync)
- [ ] stdin carries only base prompt + trigger line (no profile, no instruction blocks)
- [ ] Mock-CLI tests: argv contains `--append-system-prompt`; stdin excludes `# Tie-Off` / profile text; trigger line present
- [ ] Verification — `cargo test` green

## Deviations
## Discoveries
## Notes
