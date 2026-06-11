# ADR-004: Shared Agent Profiles

**Date**: 2026-06-11
**Status**: Accepted

## Context

Knot knot definitions previously embedded their entire agent configuration inline (`agent-config: { provider, model, tools }`). When multiple knots need the same agent — for example, the same GPT-4o model with the same tools — every knot file must be edited to change the agent target. This violates DRY principles and is error-prone. The user needs a way to define an agent profile once at the rig level and have multiple knots reference it by name.

## Decision

Agent profiles are stored as top-level `.md` files with YAML frontmatter in `rig/profiles/{name}.md`. Knot definitions reference profiles via `agent-profile-ref: {name}` instead of (or in addition to) inline `agent-config`. Profiles are resolved at processing time in `ProcessStrand::execute()`, so edits to profile `.md` files are picked up on the next strand event without restart.

### Profile File Format

```yaml
---
name: fast
provider: openai
model: gpt-4o
tools:
  - fs
system-prompt: |
  You are a fast reviewer. Keep responses concise and direct.
---

# Fast Profile

Lightweight profile for quick reviews.
```

### Knot Using a Profile Reference

```yaml
---
name: my-knot
agent-profile-ref: fast
strand-dir: strands
prompt-template:
  input-bundling: full-file
  instructions: |
    Review this document for clarity.
---
```

### Override Model from Profile

```yaml
---
name: my-knot
agent-profile-ref: fast
agent-config:
  model: claude-sonnet
strand-dir: strands
prompt-template:
  input-bundling: full-file
  instructions: |
    Review this document for clarity.
---
```

### Mutual Exclusivity

A knot file **cannot** have both `agent-profile-ref` and `agent-config` fields. The parser emits `KnotFileError::BothProfileAndConfig` if both are present. A knot must have at least one of `agent-config` or `agent-profile-ref` (`KnotFileError::MissingAgentConfigOrProfileRef`).

### Resolution Strategy

At processing time in `ProcessStrand::execute()`:

1. If knot has `agent_profile_ref`: load profile from `AgentProfileRepository`, merge profile fields as base config
2. If knot has inline `agent_config`: use as-is (backward compat)
3. If knot has both (programmatic `Knot` construction bypassing mutual exclusivity): inline overrides profile
4. If knot has neither: error

## Consequences

### Positive

- **Single source of truth**: Change an agent provider or model in one profile file and all referencing knots pick up the change on next invocation
- **DRY configuration**: No need to duplicate provider/model/tools across knot files
- **Dynamic updates**: Profiles are read from disk at processing time — no restart needed
- **Inline overrides**: Knots can override specific profile fields (e.g., swap the model)
- **Full lifecycle**: Profile CRUD via HTTP API (`GET/POST/DELETE /profiles`)
- **Testable**: Port-based design allows mock `AgentProfileRepository` in tests

### Negative

- **Added complexity**: New domain entity (`AgentProfile`), new port (`AgentProfileRepository`), new file format, new HTTP endpoints
- **More error paths**: Profile not found, parsing failures, mutual exclusivity violations
- **File system coupling**: Profiles are stored as files on disk; `list()` scans the directory, `get()` reads individual files
- **Serialization detail**: `system_prompt` (snake_case in Rust) must be serialized as `system-prompt` (kebab-case in YAML) to round-trip through the parser

### Neutral

- **Backward compatible**: Existing knots with `agent-config` work unchanged
- **Performance**: Profile resolution adds a repository lookup per strand — negligible since profiles are small and resolution is synchronous

## Alternatives Considered

### 1. Inline Config Only (No Profiles)

**Rejected**: Doesn't solve the DRY problem. Users must edit every knot file when agent config changes.

### 2. Knot-Level Config File Reference

Store knot configs in YAML files that reference a shared profile. Knot YAML would contain `config-ref: fast.yaml`.

**Rejected**: Adds an extra indirection layer. The profile is conceptually the agent config — storing it as a separate config file that references another file is too many levels. Direct profile reference (`agent-profile-ref: fast`) is simpler.

### 3. Registry-Based Profiles

Maintain profiles in-memory in a `ProfileRegistry` that is updated via HTTP.

**Rejected**: In-memory registries lose state on restart and require explicit sync. File-based profiles are durable, version-controllable, and don't need a sync mechanism. The `AgentProfileRepository` abstraction makes swapping implementations trivial if needed.

### 4. Profile Resolution at Discovery Time

Resolve profiles when knots are registered/discovered, cache the resolved config in the `Knot` entity.

**Rejected**: This defeats the primary benefit — dynamic updates. If the profile changes on disk, registered knots wouldn't see it until restart. Resolution at processing time ensures every strand invocation gets the latest profile state.

## Implementation

Implemented in Plan 23 (shared-agent-profiles.md) across 6 phases:

1. **Phase 0**: Domain — `AgentProfile` entity + `parse_agent_profile()` parser
2. **Phase 1**: Domain — `KnotFile` extension with `agent-profile-ref` + mutual exclusivity
3. **Phase 2**: Outbound — `AgentProfileRepository` trait + `FileSystemAgentProfileRepository`
4. **Phase 3**: Application — `ProcessStrand` profile resolution with inline overrides
5. **Phase 4**: Inbound — HTTP CRUD endpoints for `/profiles`, knot handler updates
6. **Phase 5**: Integration tests — 9 end-to-end tests covering CRUD, resolution, overrides, backward compat

**Total**: 331 tests pass (262 unit + 61 integration). 20 files changed, ~4,570 insertions.

## Related Documents

- **Plan**: [shared-agent-profiles.md](../plans/shared-agent-profiles.md) — implementation plan
- **PRD**: [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md) — parent feature
- **Design Document**: [design-shared-agent-profiles](../docs/design-shared-agent-profiles.md) — produced by this plan
