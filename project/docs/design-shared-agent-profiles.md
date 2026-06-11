# Design: Shared Agent Profiles

## What It Is

Agent profiles are named, shareable entities stored as `.md` files with YAML frontmatter in `rig/profiles/`. Knot definitions reference profiles by name instead of embedding their entire agent configuration inline. Profiles are resolved at processing time, so edits to profile files are reflected in the next strand invocation without restart.

## What It Does

- **Named agent profiles** — Profiles store `provider`, `model`, `tools`, and `system_prompt` as a reusable unit
- **Profile references in knots** — Knots use `agent-profile-ref: {name}` instead of inline `agent-config`
- **Dynamic resolution** — Profiles are read from disk at processing time; changes are picked up immediately
- **Inline overrides** — Knots can override specific profile fields via inline `agent-config` (e.g., swap the model)
- **Mutual exclusivity** — A knot cannot have both `agent-profile-ref` and `agent-config`
- **Profile CRUD via HTTP** — REST endpoints for creating, listing, reading, and deleting profiles
- **Backward compatible** — Knots with inline `agent-config` continue to work unchanged

## Profile File Format

Profiles are stored as Markdown files with YAML frontmatter in `rig/profiles/{name}.md`:

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

Required fields: `name`, `provider`, `model`, `system-prompt`. Optional: `tools` (list of tool names, defaults to empty).

The frontmatter parser (`parse_agent_profile`) validates all required fields are present and non-empty (whitespace-only is rejected).

## Knot Profile References

### Profile-Only Knot

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

### Profile + Inline Override

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

The inline `agent-config` overrides specific profile fields. In this example, the knot uses `fast`'s provider and tools but overrides the model to `claude-sonnet`.

### Mutual Exclusivity

A knot cannot have both `agent-profile-ref` and `agent-config`. The parser (`parse()`) emits `KnotFileError::BothProfileAndConfig`. A knot must also have at least one — `KnotFileError::MissingAgentConfigOrProfileRef` if neither is present.

## Components

| Component | Location | Role |
|-----------|----------|------|
| `AgentProfile` | `src/domain/value_objects.rs` | Domain entity: `name`, `provider`, `model`, `tools`, `system_prompt` |
| `AgentProfileError` | `src/domain/value_objects.rs` | Validation errors: `MissingName`, `EmptyProvider`, `EmptyModel`, `MissingSystemPrompt` |
| `parse_agent_profile()` | `src/domain/knot_file.rs` | Parses `{name}.md` files with YAML frontmatter into `AgentProfile` |
| `KnotFile.agent_profile_ref` | `src/domain/knot_file.rs` | Optional profile reference in knot frontmatter |
| `AgentProfileRepository` trait | `src/application/ports.rs` | Port: `get()`, `list()`, `save()`, `delete()` |
| `FileSystemAgentProfileRepository` | `src/adapters/outbound/profile_repo.rs` | File-system-backed implementation using `rig/profiles/` |
| `ProcessStrand::resolve_agent_config()` | `src/application/usecases.rs` | Resolves profile + inline config into final `AgentConfig` at processing time |
| Profile handlers | `src/adapters/inbound/loom.rs` | `list_profiles`, `get_profile`, `create_profile`, `delete_profile` |
| Profile types | `src/adapters/inbound/types.rs` | `ProfileRequest`, `ProfileResponse` for JSON API |
| Profile routes | `src/adapters/inbound/router.rs` | `GET/POST /profiles`, `GET/DELETE /profiles/{name}` |

## Profile Resolution Algorithm

When `ProcessStrand::execute()` processes a strand:

1. **Knot has `agent_profile_ref` only**: Load profile from `AgentProfileRepository`, build `AgentConfig` from profile fields (provider, model, tools from profile; goal from knot's prompt instructions)
2. **Knot has `agent_config` only**: Use inline config as-is (backward compatibility)
3. **Knot has both (programmatic `Knot` construction)**: Profile is the base; inline fields override
4. **Knot has neither**: Return `PortError::AgentExecutionFailed`

This means:
- **Dynamic updates work**: Every strand invocation reads fresh from disk
- **Override is additive**: Inline config supplements, not replaces, the profile base
- **No caching**: Profiles are not cached; `get()` always reads the latest file

## HTTP API

### List Profiles

```
GET /profiles
→ 200: [{"name":"fast","provider":"openai","model":"gpt-4o","tools":["fs"],"system_prompt":"..."}]
```

### Get Profile

```
GET /profiles/:name
→ 200: {"name":"fast","provider":"openai","model":"gpt-4o","tools":["fs"],"system_prompt":"..."}
→ 404: {"error":"profile not found: unknown"}
```

### Create Profile

```
POST /profiles/:name
Body: {"provider":"openai","model":"gpt-4o","tools":["fs"],"system_prompt":"..."}
→ 201: {"created":true}
→ 400: {"error":"provider must not be empty"}
```

### Delete Profile

```
DELETE /profiles/:name
→ 204: (no body)
→ 404: {"error":"profile not found: fast"}
```

## Configuration

### Profile Storage Directory

Profiles are stored in `{rig}/profiles/` where `{rig}` is the project root configured in `AppConfig`. The directory is created on first `save()` if it doesn't exist.

### File Naming

Profiles are named by their `name` field: `rig/profiles/{name}.md`. The name is also used in the URL path and as the `agent-profile-ref` value in knot files.

## Test Coverage

### Unit Tests

- `AgentProfile::new()` — valid construction, validation errors (empty/whitespace fields), serialization, tools
- `parse_agent_profile()` — valid profiles (with/without tools, multiline system prompt), missing fields, malformed YAML, no frontmatter, no closing delimiter
- `FileSystemAgentProfileRepository` — CRUD operations, non-existent directory handling, malformed file skipping, non-.md file skipping
- `KnotFile::parse()` — profile-only knots, mutual exclusivity, backward compatibility, missing both fields
- `ProcessStrand::resolve_agent_config()` — profile-only, inline-only, override, profile-not-found, neither-set, shared across knots, dynamic pickup

### Integration Tests

- `tests/shared_agent_profiles.rs` — 9 tests covering profile CRUD, knot creation with profile reference, profile override at processing time, dynamic profile update, profile-not-found error handling, backward compatibility, disk-to-API consistency

**Total: 331 tests pass** (262 unit + 61 integration).

## Gotchas

1. **Route ordering**: `/profiles` must be registered before `/profiles/{name}` to avoid POST `/profiles/{name}` hitting the parameterized route (which doesn't have POST). In axum, both `GET` and `POST` handlers are on separate route registrations: `.route("/profiles", get(list_profiles))` and `.route("/profiles/{name}", get(get_profile).delete(delete_profile).post(create_profile))`.

2. **Serialization key**: `AgentProfile.system_prompt` (snake_case) must serialize as `system-prompt` (kebab-case) in YAML to round-trip through `parse_agent_profile()`. Added `#[serde(rename = "system-prompt")]` to the field.

3. **Mock repository**: All existing `AppContext` constructions across unit tests and integration tests must include `profile_repo: Arc::new(MockProfileRepository::default())`. Missing this causes compile errors since `profile_repo` is a required `AppContext` field.

4. **`Knot.agent_config` is now `Option<AgentConfig>`**: Downstream code that accessed `knot.agent_config.model` directly now needs `knot.agent_config.model` (which may be `None`). The `resolve_agent_config()` method always returns a concrete `AgentConfig`, so it should be used in processing paths.

## Related Documents

- **ADR**: [adr-004-shared-agent-profiles](../adrs/adr-004-shared-agent-profiles.md) — why profiles were chosen
- **PRD**: [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md) — parent feature
- **Plan**: [shared-agent-profiles.md](../plans/shared-agent-profiles.md) — implementation plan
