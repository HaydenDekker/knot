# Phase 4: Resume re-supplies the frozen prompt + drift detection

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [ ] Retry attempts re-supply the FROZEN prompt (the spawn-time render threaded through the retry loop — never re-rendered for the wire)
- [ ] Drift check: `resolve_config_and_build` passes a re-render closure; the retry loop compares current render vs frozen; `[KNOT][PROMPT] type=PROMPT-DRIFT` logged once per attempt when they differ; frozen stands
- [ ] `inject_event_request` follow-up: empty system prompt (session already carries the frozen one)
- [ ] Tests: retry passes frozen text; drift warning fires on changed render; no drift when unchanged
- [ ] Verification — `cargo test` green

## Deviations
## Discoveries
## Notes
