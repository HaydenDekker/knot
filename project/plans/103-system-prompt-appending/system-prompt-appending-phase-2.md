# Phase 2: pi-json and pi-rpc adapters

**Plan:** [System-Prompt Appending](system-prompt-appending-plan.md)

## Checklist
- [x] `PiJsonAgentRunner`: same flag composition (shared `build_prompt_with_context` updated)
- [x] `PiRpcAgentRunner`: delegates to the json builder; `--mode rpc` args keep the flag
- [x] Mock tests for both adapters mirroring Phase 1 assertions
- [x] Verification — `cargo test` green

## Deviations
- [x] DONE — `compose_system_prompt` lives on `PiJsonAgentRunner`
      (pub(crate)); `pi-rpc` reuses it. The flag is appended after the
      mode args in both adapters.
## Discoveries
- [x] No json/rpc test asserted the profile-in-stdin shape (only stdio
      did) — the switch was quiet at this layer; explicit flag-delivery tests
      added for both adapters.
## Notes
