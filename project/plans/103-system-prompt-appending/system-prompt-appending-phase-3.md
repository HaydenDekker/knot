# Phase 3: Frozen prompt artifact + deterministic session mapping

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [ ] Mint the pi session id at spawn (`--session-id <knot-minted-id>` via extra_args; pi accepts any id — `options?.id ?? createSessionId()`, no format validation)
- [ ] Write the fully-composed system prompt to `tie-offs/<rig>/prompts/<session-id>.md` before spawn (runtime root via `derive_runtime_root`)
- [ ] Adapters pass the artifact PATH to `--append-system-prompt` (no argv bloat; pi reads file contents)
- [ ] Service-log record: `[KNOT][PROMPT] type=PROMPT-FROZEN session=… knot=… file=…` (new `log_prompt_event` helper)
- [ ] Tests: artifact written, content matches, record emitted, id threaded to the runner
- [ ] Verification — `cargo test` green

## Deviations
## Discoveries
## Notes
