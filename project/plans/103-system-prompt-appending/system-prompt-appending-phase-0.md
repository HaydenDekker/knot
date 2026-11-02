# Phase 0: Split the payload at the domain/application boundary

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [x] Add `system_prompt: String` to `ExecutionContext` (`src/application/ports.rs`); document `profile_prompt` as system-prompt-delivered (last section)
- [x] Add `system_prompt: String` param to `AgentRunner::execute_with_config` / `execute_with_config_and_observer` (default impls build the ctx)
- [x] `AgentEventsContextProvider::build_split_context()` → `SplitPromptContext { instructions, runtime }`; `build_full_context` composes from it (wire unchanged)
- [x] `resolve_config_and_build` routes instructions → `system_prompt`, pending-events section → user prompt (order: delivery render, pending, base)
- [x] `session_resume::{execute_with_resume, execute_with_resume_internal}` thread `system_prompt`; `inject_event_request` passes empty (session already carries it)
- [x] Adapters accept the field; Phase 0 keeps the old wire (prepend after profile → byte-identical prompt)
- [x] Update all `ExecutionContext` literal sites (adapters' tests, ports tests, test_fixtures, server.rs)
- [x] New unit tests: split names instructions vs runtime; gating preserved
- [x] Verification — `cargo test` green (wire unchanged)

## Deviations
- [x] DONE — `listener_context` kept on `ResolvedExecution` as the COMPOSED whole
      (instructions + runtime) so the event-enforcement follow-up repeat is
      unchanged; the split itself is the single build (one `build_split_context`
      call, instruction-fetch budget stays two pulls per session).
## Discoveries
- Pre-existing bug found and fixed: `build_listener_context_with_instructions`
  grouped by a `HashMap` and iterated it directly despite the comment claiming
  "preserving insertion order" — the subscriber-bullet run was non-deterministic
  per process (the plan 102 byte-identical test flaked ~1-in-4). Now a `Vec`
  with first-seen-wins.
- The plan_102 fetch-count tests immediately caught a double-fetch regression
  (build_context + build_split_context) — resolved by composing
  `listener_context` from the split instead of a second build.
- `system_event_subscriptions` mock recorded only the first 300 chars of the
  prompt; with the instruction blocks now ahead of the delivery render the
  `Event: QueueIdle` line fell outside the window — mock widened to 4000.
## Notes
