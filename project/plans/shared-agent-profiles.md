# Plan: Shared Agent Profiles

## Related PRD

This plan contributes to [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md), Story 8: Shared Agent Profiles.

This plan implements the technical constraint ("new domain entity + profile reference mechanism") and the two remaining success criteria: "Agent profiles are shareable, named entities that multiple knots can reference" and "Updating a shared agent profile's LLM target is reflected in all knots that reference it on their next invocation."

## Problem

Currently, each knot definition embeds its agent configuration inline (`agent-config: { provider, model, tools }`). When multiple knots need the same agent — for example, the same GPT-4o model with the same tools — every knot file must be edited to change the agent target. This is error-prone and defeats DRY principles. The user wants to define an agent profile once at the rig level and have knots reference it by name, so that updating the profile's provider or model is instantly reflected in all referencing knots on their next invocation (dynamic, read-at-processing-time).

## Target

When this plan is complete:

- A new `AgentProfile` domain entity exists, representing the shared configuration: provider, model, tools, and system-prompt.
- Agent profiles are stored as top-level `.md` files in `rig/profiles/{name}.md` with YAML frontmatter.
- Knot definitions can optionally reference a profile via `agent-profile-ref: {name}` instead of (or in addition to) inline `agent-config`.
- Profiles are resolved at processing time — edits to a profile `.md` file are picked up on the next strand event without restart.
- Inline knot config overrides profile fields when both are present.
- HTTP API includes endpoints for CRUD on profiles, and knot endpoints accept `agent_profile_ref`.
- Mutual exclusivity validation: a knot file cannot have both `agent-profile-ref` and `agent-config` fields.

### Agent Profile File Format

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

### Knot Overriding Profile Fields

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

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Module | What it covers | Status |
|-------------|---------------|--------|
| `domain::value_objects::tests` | `AgentConfig` construction, serialization, `build_cli_args` | ✅ Green — verifies current `AgentConfig` behavior |
| `domain::knot_file::tests` | Knot frontmatter parsing, validation, error cases | ✅ Green — validates current inline `agent-config` parsing |
| `adapters::outbound::loom_repository::tests` | `scan()`, path resolution, knot file discovery | ✅ Green — tests `scan_knot_files` and `resolve_path` |
| `application::ports::tests` | Port contract (object safety, mock implementations) | ✅ Green — verifies port traits are callable |
| `adapters::inbound::loom::tests` | HTTP handler responses (POST/GET/DELETE looms/knots) | ✅ Green — tests knot creation/update via JSON |
| `adapters::subprocess::tests` | Subprocess execution, prompt passing, timeout | ✅ Green — tests agent CLI invocation |
| `integration/auto_discovery_and_knot_crud.rs` | Full end-to-end loom+knot lifecycle via HTTP | ✅ Green — integration tests with mock ports |

## Test Gaps

- No `AgentProfile` entity or value object exists — no unit tests for it
- No profile parser function — no tests for reading profile `.md` files
- No `AgentProfileRepository` port — no tests for profile discovery/lookup
- No profile resolution logic in `ProcessStrand` — no tests for profile+knot merging
- No HTTP endpoints for profiles — no tests for `GET/POST/DELETE /profiles`
- No tests for profile override (knot with profile ref + inline model)
- No tests for mutual exclusivity validation (both `agent-profile-ref` and `agent-config`)
- No tests for dynamic profile update pickup at processing time

## Phases

### Phase 0: Domain — AgentProfile Entity + Profile Parser

**Layer:** Domain (value objects, knot file parser)

Build the `AgentProfile` entity and parser. This is the foundation — everything else depends on it.

- [x] Add `AgentProfile` to `value_objects.rs` with fields: `name`, `provider`, `model`, `tools` (Vec), `system_prompt`
- [x] Implement `AgentProfile::new()` with validation (all required fields non-empty)
- [x] Add `parse_agent_profile(content: &str) -> Result<AgentProfile, AgentProfileError>` to `knot_file.rs`
- [x] `AgentProfileError` enum: `MissingName`, `EmptyProvider`, `EmptyModel`, `MissingSystemPrompt`, `InvalidFormat`
- [x] Profile file format: YAML frontmatter with `name`, `provider`, `model`, optional `tools`, required `system-prompt`. No `prompt-template` section needed (that lives in the knot).
- [x] Unit tests in `value_objects.rs` for `AgentProfile` construction, serialization, validation errors
- [x] Unit tests in `knot_file.rs` for profile parsing: valid profiles, missing fields, malformed YAML
- [x] `cargo test` passes

### Phase 1: Domain — Knot Parser Extension for `agent-profile-ref`

**Layer:** Domain (knot file parser)

Extend knot parsing to accept `agent-profile-ref` and handle mutual exclusivity.

- [x] Add `agent_profile_ref: Option<String>` to knot frontmatter parsing (`RawFrontmatter`)
- [x] Add `KnotFileError::BothProfileAndConfig` — validation error when both `agent-profile-ref` and `agent-config` are present
- [x] Update `parse()` to emit `BothProfileAndConfig` error when both fields exist
- [x] Add tests for mutual exclusivity validation
- [x] Add test for valid knot with `agent-profile-ref` (no `agent-config`)
- [x] Add test for valid knot with both (should fail)
- [x] Add test for existing knot with `agent-config` only (should still work — backward compat)
- [x] `cargo test` passes

### Phase 2: Outbound — AgentProfileRepository Port + Implementation

**Layer:** Application port + Outbound adapter

Create the port trait and filesystem-backed implementation for profile storage and lookup.

- [x] Add `AgentProfileRepository` trait to `ports.rs`:
  - `get(name: &str) -> Result<Option<AgentProfile>, PortError>`
  - `list() -> Result<Vec<AgentProfile>, PortError>`
  - `save(profile: AgentProfile) -> Result<(), PortError>`
  - `delete(name: &str) -> Result<(), PortError>`
- [x] Add `PortError::ProfileNotFound(String)` variant
- [x] Implement `FileSystemAgentProfileRepository` in `adapters/outbound/profile_repo.rs`:
  - Profiles stored in `{rig}/profiles/` directory
  - File naming: `{profile-name}.md`
  - `get()`: read `{rig}/profiles/{name}.md`, parse with `parse_agent_profile()`
  - `list()`: scan `{rig}/profiles/` for `.md` files, parse each
  - `save()`: write YAML frontmatter to `{rig}/profiles/{name}.md`
  - `delete()`: remove file
  - Handle non-existent profiles directory gracefully (return empty for list, None for get)
- [x] Update `adapters/outbound/mod.rs` to export `profile_repo` module
- [x] Update `adapters/inbound/types.rs` — import `AgentProfile` from domain
- [x] Mock `AgentProfileRepository` in `ports.rs` tests (follow existing mock pattern)
- [x] Unit tests for `FileSystemAgentProfileRepository`: create/get/list/delete profiles, non-existent dir handling, file parsing
- [x] `cargo test` passes

### Phase 3: Application — Profile Resolution in ProcessStrand

**Layer:** Application (use cases)

Wire `AgentProfileRepository` into `ProcessStrand` and resolve profiles at processing time. This is the critical phase — it makes profiles dynamic.

- [ ] Add `Arc<dyn AgentProfileRepository>` to `ProcessStrand` struct
- [ ] Update `ProcessStrand::execute()`:
  - If knot has `agent_profile_ref`: load profile from repo
  - Merge profile fields into agent config (profile is the base)
  - Inline `agent-config` fields override profile fields
  - If no profile ref: use inline `agent-config` as-is (backward compat)
  - Build CLI args from resolved config + prompt template (existing logic)
- [ ] Add `resolve_agent_config(knot: &Knot, profile_repo: &dyn AgentProfileRepository) -> Result<AgentConfig, PortError>` helper
- [ ] Profile resolution errors: profile not found → log error, return `PortError::ProfileNotFound`
- [ ] Add test: knot with profile ref → resolved config uses profile fields
- [ ] Add test: knot with profile ref + inline model override → resolved config uses profile provider/tools but inline model
- [ ] Add test: knot without profile ref → uses inline agent-config (backward compat)
- [ ] Add test: knot with profile ref but profile doesn't exist → error
- [ ] Update `usecases.rs` to wire `AgentProfileRepository` into composition (where `ProcessStrand` is constructed)
- [ ] `cargo test` passes

### Phase 4: Inbound — HTTP Endpoints for Profiles

**Layer:** Inbound adapter (handlers + router)

Add REST API for profile CRUD and update knot handlers to support `agent_profile_ref`.

- [ ] Add profile types to `adapters/inbound/types.rs`:
  - `ProfileRequest`: `provider`, `model`, `tools`, `system_prompt` (name derived from URL path)
  - `ProfileResponse`: `name`, `provider`, `model`, `tools`, `system_prompt`
- [ ] Add profile handler functions in `adapters/inbound/loom.rs` (or new file `profiles.rs`):
  - `list_profiles` — `GET /profiles`
  - `get_profile` — `GET /profiles/{name}`
  - `create_profile` — `POST /profiles/{name}`
  - `delete_profile` — `DELETE /profiles/{name}`
- [ ] Each handler uses `AgentProfileRepository` port via `AppContext`
- [ ] `create_profile` validates mutual exclusivity of `system_prompt` (required, non-empty)
- [ ] `delete_profile` returns 404 if profile not found
- [ ] Update `AppContext` in `types.rs` to include `Arc<dyn AgentProfileRepository>`
- [ ] Update router in `router.rs`:
  - Add profile routes: `/profiles`, `/profiles/{name}`
  - Add `AgentProfileRepository` to `AppContext` state
  - Register OpenAPI schema for `AgentProfile`
- [ ] Update knot handlers (`create_knot`, `update_knot`) to accept optional `agent_profile_ref` field in `KnotRequest`
- [ ] Update `generate_knot_file()` to write `agent-profile-ref` instead of `agent-config` when profile ref is present
- [ ] Unit tests for profile handlers (mock `AgentProfileRepository`)
- [ ] `cargo test` passes

### Phase 5: Composition Root + Integration Tests

**Layer:** Composition root + integration tests

Wire `AgentProfileRepository` into the application and write end-to-end tests.

- [ ] Update composition root (`lib.rs` / `server.rs`) to instantiate `FileSystemAgentProfileRepository`
- [ ] Pass it into `AppContext` and `ProcessStrand`
- [ ] Write integration tests in `tests/shared_agent_profiles.rs`:
  - Test: create profile via `POST /profiles/{name}` → verify `GET /profiles/{name}` returns it
  - Test: list profiles via `GET /profiles` → returns all profiles
  - Test: delete profile via `DELETE /profiles/{name}` → verify 404 on subsequent get
  - Test: create knot with `agent_profile_ref` via `POST /looms/{id}/knots` → knot file has profile ref in frontmatter
  - Test: profile resolved at processing time — profile updated on disk, next strand uses new model
  - Test: profile not found → strand processing logs error, tie-off records failure
  - Test: backward compat — knot without profile ref still processes with inline config
- [ ] Wire profile repository into `build_test_context()` for handler tests
- [ ] Update all existing tests that create `AppContext` to include mock profile repo (no-op implementation)
- [ ] Run full test suite — all passing
- [ ] `cargo test` passes (including integration tests)

## Notes

- Profile resolution happens **at processing time** in `ProcessStrand::execute()`, not at discovery time. This ensures that edits to profile `.md` files are picked up on the next strand event without service restart.
- The `goal` field currently in `AgentConfig` is not used by the agent CLI — it's metadata only. Profiles will use `system_prompt` (which IS used as the `--system-prompt` CLI argument). This is a clean separation: `system_prompt` from the profile becomes the system prompt; `goal` from the knot (if present) is ignored in CLI args.
- `tools` defaults to empty Vec in both profiles and knots, matching current behavior.
- The profile repository scans the rig's `profiles/` directory on `list()` calls, and reads individual files on `get()` — both parse fresh from disk each time.
