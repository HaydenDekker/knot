# Phase 3: Frozen prompt artifact + deterministic session mapping

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [x] Mint the pi session id at spawn (`--session-id <knot-minted-id>` via extra_args; pi accepts any id — `options?.id ?? createSessionId()`, no format validation)
- [x] Write the fully-composed system prompt to `tie-offs/<rig>/prompts/<session-id>.md` before spawn (runtime root via `derive_runtime_root`)
- [x] Adapters pass the artifact PATH to `--append-system-prompt` (no argv bloat; pi reads file contents)
- [x] Service-log record: `[KNOT][PROMPT] type=PROMPT-FROZEN session=… knot=… file=…` (new `log_prompt_event` helper)
- [x] Tests: artifact written, content matches, record emitted, id threaded to the runner
- [x] Verification — `cargo test` green

## Deviations
- [x] DONE — the minted session id is NOT seeded into the retry loop's
      `session_id` (kept capture-based): plan 081's fresh-restart semantics
      for never-captured sessions stand. The minted id rides only the
      first-spawn `extra_args` (`--session-id <minted>`) and the artifact
      name. Retries that push their own `--session-id <captured>` land on
      the same value (pi uses the exact id) — the duplicate is equal and
      harmless.
- [x] The composed prompt is frozen by the USECASE (it owns rig paths);
      the composition rule itself moved to the single domain function
      `domain::events::compose_system_prompt` — adapters delegate, and
      skip their own flag when extra_args already carries
      `--append-system-prompt` (frozen artifact wins).
## Discoveries
- `LoomEvent` matches are exhaustive in two places (service_log.rs,
  activity.rs `loom_id_of`) — a new variant must be armed in both.
- pi reads `--append-system-prompt <path>` as file contents when the
  path exists (verified in pi's resource-loader), so the artifact path
  rides argv with no argv bloat; the inline-text fallback covers the
  artifact-write-failure case (non-fatal, `[KNOT][EVENT] type=CONFIG
  source=prompt-freeze` warning).
## Notes
