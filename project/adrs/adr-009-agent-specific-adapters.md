# ADR-009: Agent-Specific Adapters

**Date:** 2026-06-27
**Status:** Accepted

## Context

Knot invokes an external agent CLI to process strands. The current design uses `RigAgentConfig` with free-form fields `cli_path` and `cli_args`, treating the agent binary as an opaque command to which arbitrary flags are appended. Plan 46 (JSON-based Agent Adapter) then adds `invocation_mode` to select between `stdio` and `json` output parsing.

This creates three problems:

1. **Speculative generality** — `cli_path` and `cli_args` suggest any agent binary can be plugged in, but the invocation contract (stdin format, CLI flags, output format) is tightly coupled to `pi` specifically. The abstraction leaks.
2. **Split responsibility** — the rig config declares the binary path and flags, the adapter decides output parsing, and `invocation_mode` bridges the two. This means the rig config must know about implementation details (`--mode json`) just to select a parsing strategy.
3. **Footgun surface** — user-supplied `cli_args` can conflict with adapter-specific flags (e.g. user passes `--mode stdio` while `invocation_mode: json` appends `--mode json`), with no validation.

## Decision

Each adapter type is specific to a **target agent and communication protocol**. The adapter owns the full invocation contract: binary path, CLI flags, output parsing, and error handling. The rig config selects an adapter — it does not configure invocation details.

### Architecture Overview

```
RigAgentConfig
├── agent_adapter: AgentAdapter    # enum: PiStdio | PiJson
│
src/adapters/
├── pi_stdio.rs                    # PiStdioAgentRunner
│   └── invokes: pi -p --model <m>
│       stdin: prompt
│       stdout: plain text response
│
└── pi_json.rs                     # PiJsonAgentRunner
    └── invokes: pi -p --model <m> --mode json
        stdin: prompt
        stdout: JSON-L (session, token_usage, response)
```

### Rig Config Shape

```yaml
# rig agent config
agent-adapter: pi-json    # or "pi-stdio" (default)
```

No `cli_path`, no `cli_args`, no `invocation_mode`. The rig config selects *which adapter exists* — nothing more.

### Implications for Design

- **`RigAgentConfig`** shrinks from `{ cli_path, cli_args, invocation_mode }` to `{ agent_adapter: AgentAdapter }` — it is purely an adapter selector.
- **`AgentConfig`** (from `AgentProfile`) still provides `provider`, `model`, and `tools` — these are knot-level semantic config, not invocation mechanics. They flow through to `build_cli_args()` on the agent config (not rig config).
- **Each adapter hardcodes** its own `cli_path` (`"pi"`) and protocol-specific flags (`[]` for stdio, `["--mode", "json"]` for JSON-L).
- **Adding a new agent** (e.g., a different CLI tool) means creating a new adapter type and a new enum variant — not adding config fields. This is explicit and visible in code review.
- **Adding a new protocol** for the same agent (e.g., Pi adds `--mode stream`) means a new adapter type and enum variant.

### Configuration

- `agent_adapter` field on `RigAgentConfig` (YAML: `agent-adapter`)
- Default: `pi-stdio` (current behaviour)
- Values: `pi-stdio`, `pi-json`

### Testing Strategy

Each adapter is tested independently:
- Unit tests verify CLI arg construction, timeout handling, error paths
- Integration tests verify full pipeline with real `pi` binary
- Regression tests ensure `pi-stdio` remains unchanged

## Consequences

### Positive

- **Clear responsibility** — the adapter owns its entire contract. No split between "who spawns" and "who parses."
- **No footguns** — user cannot pass conflicting flags. The adapter is the single source of truth for how invocation works.
- **Explicit extension** — adding a new agent or protocol is a visible code change (new adapter + enum variant), not a config tweak whose boundaries are unclear.
- **Simpler config** — `RigAgentConfig` is a one-field selector. No need to document what `cli_args` can or cannot contain.

### Negative

- **Less flexible than advertised** — we cannot support arbitrary agent binaries at runtime. This is acceptable because the current contract (stdin prompt, `pi`-specific flags) is already `pi`-specific.
- **More types** — each agent + protocol combination is a distinct adapter struct. For two modes of one agent that's two structs instead of one with config. This is a worthwhile trade-off for clarity.

### Trade-offs Considered

| Alternative | Rejected Because |
|-------------|------------------|
| Generic `RigAgentConfig { cli_path, cli_args, invocation_mode }` | Speculates on multi-agent support that doesn't exist. Split responsibility between config and adapter. |
| Per-knot adapter selection | Invocation mechanics are rig-level concern. Knots define *what* to do, not *how* to talk to the agent. |
| Adapter factory with runtime config | Adds indirection for no benefit. The set of adapters is a compile-time decision. |

## References

- Plan 46: [JSON-based Agent Adapter](plans/agent-json-adapter.md)
- Source: `src/domain/value_objects.rs` (`RigAgentConfig`)
- Source: `src/adapters/subprocess.rs` (`SubprocessAgentRunner`)
- ADR-007: [Stdin-Only Agent Invocation](adr-007-stdin-only-agent-invocation.md)
