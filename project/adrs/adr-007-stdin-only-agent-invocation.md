# ADR-007: Stdin-Only Agent Invocation

**Date:** 2026-06-16
**Status:** Accepted
**Plan:** [Simplify Agent Invocation](plans/simplify-agent-invocation.md)

## Context

The agent profile's prompt was injected into `pi` via `--system-prompt` CLI flag. This created two problems:

1. **Duplication**: Knot instructions appeared in both the system prompt (merged via `resolve_agent_config`) and the user message (stdin), wasting context tokens.
2. **Invisible in session**: Pi does not persist `--system-prompt` into the session `.jsonl` file — the profile's persona instructions were sent to the LLM but never recorded, making sessions opaque when inspected.

## Decision

Remove `--system-prompt` entirely. All agent prompt content is delivered via stdin as a single coherent user message:

```
<profile-prompt>

<knot instructions>

**<knot-name>** triggered by **<event-type>** on **<strand-path>**

<@strand-file>
```

The `AgentProfile.system_prompt` field was renamed to `profile_prompt` (YAML key: `profile-prompt`) to clarify this is a profile-level prompt segment, not an HTTP/API system role message.

## Consequences

- **Positive**: No prompt duplication. Profile prompt is now visible in session files. Simpler code path — no CLI arg construction for the prompt.
- **Breaking change**: Existing `rig/profiles/*.md` files using `system-prompt:` YAML key must be updated to `profile-prompt:`. Knot will fail to parse profiles with the old key.
- **Neutral**: The profile prompt in stdin is effectively the same as a system prompt for the LLM — it's the first text in the conversation. The difference is it's now recorded in session storage.

## Implementation

- `AgentConfig::build_cli_args()` no longer accepts `system_prompt` parameter — signature simplified to `build_cli_args(&self) -> Vec<String>`
- `ExecutionContext` gained `profile_prompt: String` field
- `SubprocessAgentRunner::build_prompt_with_context()` accepts `profile_prompt` and prepends it to the prompt chain
- `resolve_agent_config()` return type simplified from `(AgentConfig, String, Option<Duration>)` to `(AgentConfig, Option<Duration>)`
- All tests updated (303+ unit + integration tests pass)
