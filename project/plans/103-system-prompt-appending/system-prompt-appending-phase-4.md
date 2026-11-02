# Phase 4: Resume re-supplies the frozen prompt + drift detection

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [x] Retry attempts re-supply the FROZEN prompt (the spawn-time render threaded through the retry loop — never re-rendered for the wire)
- [x] Drift check: `resolve_config_and_build` passes a re-render closure; the retry loop compares current render vs frozen; `[KNOT][PROMPT] type=PROMPT-DRIFT` logged once per attempt when they differ; frozen stands
- [x] `inject_event_request` follow-up: empty system prompt (session already carries the frozen one)
- [x] Tests: retry passes frozen text; drift warning fires on changed render; no drift when unchanged
- [x] Verification — `cargo test` green

## Deviations
- [x] DONE — the drift check is a `DriftCheck = Arc<dyn Fn() -> String +
      Send + Sync>` closure supplied by the usecase (it owns the
      instruction source / BuildContext), threaded as the last arg of
      `execute_with_resume`. The retry loop calls it fresh each retry and
      compares against the frozen `system_prompt`; divergence records
      `PromptDriftDetected` (service line `PROMPT-DRIFT ...`).
## Discoveries
- The frozen artifact already rides `agent_config.extra_args` into every
  retry (Phase 3), so "resume re-supplies the frozen prompt" is free —
  Phase 4 is purely the drift SURFACE, not a re-supply mechanism.
- The drift closure adds one instruction re-render per RETRY only (never
  on the first hop), so the plan 102 two-fetches-per-session budget is
  untouched on the happy path.
## Notes
