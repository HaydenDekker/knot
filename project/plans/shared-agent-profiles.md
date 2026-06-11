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

## Implementation Status: 🟡 In Progress (2026-06-11)

Phases 0–9 complete. Phases 10–11 remaining — remove inline `agent-config` from knots (profile is the only source) and fix remaining clippy issues.

**Result so far:** 352 tests pass (270 unit + 82 integration). New files: `src/adapters/outbound/profile_repo.rs` (FileSystemAgentProfileRepository), `tests/shared_agent_profiles.rs` (10 integration tests). Domain: `AgentProfile` entity + parser, `KnotFile` extends with `agent_profile_ref`, `KnotFileError::BothProfileAndConfig` + `MissingAgentConfigOrProfileRef`. Outbound: `AgentProfileRepository` trait + file-system impl. Application: `ProcessStrand` resolves profiles at processing time with inline overrides. Inbound: CRUD endpoints for `/profiles`, knot handlers accept `agent_profile_ref`.

## Issues Found in Code Review

### Critical (blockers)

1. **`generate_knot_file` writes both `agent-profile-ref` AND `agent-config`** — When a knot has `agent_profile_ref` set, `generate_knot_file()` produces a `.md` file containing both fields. But `KnotFile::parse()` rejects files with both as `KnotFileError::BothProfileAndConfig`. Any knot created via the HTTP API with a profile ref produces an unparsable file — the knot cannot be recovered on restart or by `ConfigEventHandler`.

2. **`KnotRequest.agent_config` is not optional** — `agent_config: AgentConfig` is a required field. The plan spec and target show a pure profile-ref knot with no inline config, but the HTTP API forces the caller to always supply `agent_config`. There is no way to create a knot that has *only* a profile reference through the API.

3. **`resolve_agent_config` silently discards `system_prompt`** — When resolving a profile-ref knot, `resolve_agent_config` builds `AgentConfig` from the profile's `provider`, `model`, `tools` — but uses `knot.prompt_template.instructions` as the `goal`, completely discarding the profile's `system_prompt`. The `system_prompt` is the profile's primary value (the agent's instructions/personality), and it is never passed to the CLI. `build_cli_args` always uses `template.instructions` for `--system-prompt`.

### Design

4. **Profile save loses markdown body** — `FileSystemAgentProfileRepository::save()` overwrites the file with minimal frontmatter + heading + system_prompt as body. Any custom markdown documentation the user wrote is lost.

5. **`extract_frontmatter_for_profile` duplicates `extract_frontmatter`** — Two nearly identical frontmatter extraction functions exist. The profile version mislabels structural errors (no frontmatter, no closing delimiter) as `AgentProfileError::MissingName`.

6. **`derive_tieoff_path` doc comment is a bad merge** — Two overlapping descriptions concatenated into one doc comment.

7. **Route: `POST /profiles` has no name** — Router wires `POST /profiles` → `create_profile` with no path parameter. The handler needs a name from the URL, so this route cannot work. The `create_profile` handler uses `Path(name)` which requires a path segment.

8. **`MockLoomRepository::save` is no-op** — Returns `Ok(())` without storing data. Tests won't detect save-path bugs.

### Minor

9. **Unused import `HashMap` in `usecases.rs`** line 7.

10. **14 clippy warnings** — collapsible `if`, manual `Option::map`, `&PathBuf` → `&Path`, same-type cast, iterator-on-map-values.

11. **Test `delete_is_idempotent_on_file` is misnamed** — Tests that second delete fails, but name implies idempotency (second call should succeed).

12. **`profile_not_found_logs_error` has vague assertion** — Accepts `idle` as valid status, passes even if processing never started.

13. **No test for pure profile-ref knot** — All tests create knots with profile refs that also supply inline `agent_config`.

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

- [x] Add `Arc<dyn AgentProfileRepository>` to `ProcessStrand` struct
- [x] Update `ProcessStrand::execute()`:
  - If knot has `agent_profile_ref`: load profile from repo
  - Merge profile fields into agent config (profile is the base)
  - Inline `agent-config` fields override profile fields
  - If no profile ref: use inline `agent-config` as-is (backward compat)
  - Build CLI args from resolved config + prompt template (existing logic)
- [x] Add `resolve_agent_config(knot: &Knot, profile_repo: &dyn AgentProfileRepository) -> Result<AgentConfig, PortError>` helper
- [x] Profile resolution errors: profile not found → log error, return `PortError::ProfileNotFound`
- [x] Add test: knot with profile ref → resolved config uses profile fields
- [x] Add test: knot with profile ref + inline model override → resolved config uses profile provider/tools but inline model
- [x] Add test: knot without profile ref → uses inline agent-config (backward compat)
- [x] Add test: knot with profile ref but profile doesn't exist → error
- [x] Update `usecases.rs` to wire `AgentProfileRepository` into composition (where `ProcessStrand` is constructed)
- [x] `cargo test` passes

### Phase 4: Inbound — HTTP Endpoints for Profiles

**Layer:** Inbound adapter (handlers + router)

Add REST API for profile CRUD and update knot handlers to support `agent_profile_ref`.

- [x] Add profile types to `adapters/inbound/types.rs`:
  - `ProfileRequest`: `provider`, `model`, `tools`, `system_prompt` (name derived from URL path)
  - `ProfileResponse`: `name`, `provider`, `model`, `tools`, `system_prompt`
- [x] Add profile handler functions in `adapters/inbound/loom.rs` (or new file `profiles.rs`):
  - `list_profiles` — `GET /profiles`
  - `get_profile` — `GET /profiles/{name}`
  - `create_profile` — `POST /profiles/{name}`
  - `delete_profile` — `DELETE /profiles/{name}`
- [x] Each handler uses `AgentProfileRepository` port via `AppContext`
- [x] `create_profile` validates mutual exclusivity of `system_prompt` (required, non-empty)
- [x] `delete_profile` returns 404 if profile not found
- [x] Update `AppContext` in `types.rs` to include `Arc<dyn AgentProfileRepository>`
- [x] Update router in `router.rs`:
  - Add profile routes: `/profiles`, `/profiles/{name}`
  - Add `AgentProfileRepository` to `AppContext` state
  - Register OpenAPI schema for `AgentProfile`
- [x] Update knot handlers (`create_knot`, `update_knot`) to accept optional `agent_profile_ref` field in `KnotRequest`
- [x] Update `generate_knot_file()` to write `agent-profile-ref` instead of `agent-config` when profile ref is present
- [x] Unit tests for profile handlers (mock `AgentProfileRepository`)
- [x] `cargo test` passes

### Phase 5: Composition Root + Integration Tests

**Layer:** Composition root + integration tests

Wire `AgentProfileRepository` into the application and write end-to-end tests.

- [x] Update composition root (`lib.rs` / `server.rs`) to instantiate `FileSystemAgentProfileRepository`
- [x] Pass it into `AppContext` and `ProcessStrand`
- [x] Write integration tests in `tests/shared_agent_profiles.rs`:
  - Test: create profile via `POST /profiles/{name}` → verify `GET /profiles/{name}` returns it
  - Test: list profiles via `GET /profiles` → returns all profiles
  - Test: delete profile via `DELETE /profiles/{name}` → verify 404 on subsequent get
  - Test: create knot with `agent_profile_ref` via `POST /looms/{id}/knots` → knot file has profile ref in frontmatter
  - Test: profile resolved at processing time — profile updated on disk, next strand uses new model
  - Test: profile not found → strand processing logs error, tie-off records failure
  - Test: backward compat — knot without profile ref still processes with inline config
- [x] Wire profile repository into `build_test_context()` for handler tests
- [x] Update all existing tests that create `AppContext` to include mock profile repo (no-op implementation)
- [x] Run full test suite — all passing
- [x] `cargo test` passes (including integration tests)

### Phase 6: Fix — `generate_knot_file` Mutual Exclusivity + `KnotRequest` Shape

**Layer:** Inbound adapter (types, handlers) — Domain (knot file format)

Fixes issues #1 and #2: make `agent_config` optional in `KnotRequest` and fix `generate_knot_file` to respect mutual exclusivity.

- [x] Make `KnotRequest.agent_config` optional: `agent_config: Option<AgentConfig>`
- [x] Update `KnotRequest` deserialization: `#[serde(default)]` on `agent_config`
- [x] Fix `generate_knot_file()` — when `agent_profile_ref` is set:
  - Write **only** `agent-profile-ref` in frontmatter (no `agent-config`)
  - When `agent_profile_ref` is absent, write `agent-config` as before
  - Knot file output must pass `KnotFile::parse()` — add a test that round-trips the generated file through the parser
- [x] Fix `register_loom`, `create_knot`, `update_knot` handlers:
  - Build `Knot` entity with `agent_config: body.agent_config.clone()` (Option, not Some)
  - When only `agent_profile_ref` is provided, `agent_config` is `None`
- [x] Add unit test: `generate_knot_file` with profile ref only → parses cleanly through `KnotFile::parse()`
- [x] Add unit test: `generate_knot_file` with agent config only → parses cleanly (backward compat)
- [x] Add integration test: `POST /looms/{id}/knots` with only `agent_profile_ref` (no `agent_config`) → 201, file is parseable
- [x] `cargo test` passes

### Phase 7: Fix — `resolve_agent_config` Uses Profile `system_prompt`

**Layer:** Application (use cases) — Domain (CLI args)

Fixes issue #3: the profile's `system_prompt` must flow into the agent CLI invocation.

- [x] Update `resolve_agent_config()` in `ProcessStrand`:
  - When profile ref resolves, merge profile's `system_prompt` into the execution context
  - The `system_prompt` from the profile should become the `--system-prompt` CLI argument
  - The knot's `prompt_template.instructions` can still provide additional instructions (concatenated or used as context)
  - Decision: profile `system_prompt` is the base; knot `prompt_template.instructions` appends as task-specific direction
- [x] Update `ProcessStrand::execute()`:
  - Pass the resolved system prompt through to `ExecutionContext` (new field or modified `cli_args`)
  - `build_cli_args` receives the merged system prompt
- [x] Decision on merge strategy: `--system-prompt "{profile_system_prompt}\n\n{knot_instructions}"` — profile system_prompt is the base, knot instructions appended
- [x] Add unit test: profile ref knot → CLI args contain profile's `system_prompt` as `--system-prompt`
- [x] Add unit test: profile ref knot → CLI args also include knot's `prompt_template.instructions`
- [x] Add integration test: profile with distinct system_prompt → processed strand output reflects profile's instructions (use mock agent that echoes `--system-prompt` value)
- [x] `cargo test` passes

### Phase 8: Fix — Profile Save Preserves Body + Frontmatter Extraction Cleanup

**Layer:** Outbound adapter — Domain (parser)

Fixes issues #4 and #5: preserve markdown body on save, eliminate duplicate frontmatter extraction.

- [x] Refactor `knot_file.rs`:
  - Extract shared `extract_frontmatter(content: &str) -> Result<(String, Option<String>), &str>` helper
  - Returns the YAML text and optional body (markdown after closing `---`)
  - Both `parse()` and `parse_agent_profile()` call the shared helper
- [x] Update `AgentProfileError` — add `InvalidFormat` variant for structural errors (no frontmatter, no closing delimiter)
- [x] Update `FileSystemAgentProfileRepository::save()`:
  - When overwriting, read the existing file first (if it exists)
  - Preserve the existing body (markdown after closing `---`)
  - Write: `---\n<new_yaml>---\n\n<preserved_body>`
  - On create (no existing file), use current minimal body
- [x] Add unit test: save profile that already has body → body preserved after round-trip
- [x] Add unit test: `parse_agent_profile` with no frontmatter → returns `InvalidFormat` error
- [x] `cargo test` passes

### Phase 9: Fix — Route Cleanup + Clippy + Test Polish

**Layer:** Inbound adapter (router) — All layers (lint)

Fixes issues #6, #7, #8, #9, #10, #11, #12, #13.

- [x] Fix `derive_tieoff_path` doc comment — remove merged duplicate, keep single clear description
- [x] Fix router: remove `POST /profiles` (no-name route). `POST /profiles/{name}` is the sole create endpoint
- [x] Fix `MockLoomRepository::save` — store in internal `HashMap` so `get`/`list` return saved data
- [x] Remove unused `HashMap` import from `usecases.rs`
- [x] Run `cargo clippy --fix` to resolve remaining warnings (9 auto-fixes + 3 manual fixes)
- [x] Rename test `delete_is_idempotent_on_file` → `delete_twice_returns_error`
- [x] Fix `profile_not_found_logs_error` integration test — assert `processing` or `failed` status, check `last_error` for profile name
- [x] Add test: pure profile-ref knot creation via HTTP (no inline `agent_config`) — `create_pure_profile_ref_knot`
- [x] `cargo test` passes

### Phase 10: Remove Inline `agent-config` — Profile Is The Only Source

**Layer:** Domain — Application — Inbound — All tests

Remove the ability to declare `agent-config` inline in a knot. Every knot must reference a shared agent profile via `agent-profile-ref`. The profile holds all agent configuration (`provider`, `model`, `tools`, `system-prompt`). The knot's `prompt-template.instructions` provides task-specific direction appended to the profile's system prompt.

**Rationale:** Inline config defeats the DRY principle that profiles solve. The in-memory vs. on-disk inconsistency (handler stored both fields, file wrote only profile ref) revealed that mutual exclusivity validation was fragile. Removing inline config entirely eliminates the ambiguity.

- [ ] **Domain — `KnotFile`:**
  - Remove `agent_config: Option<AgentConfig>` field from `KnotFile` struct
  - Change `agent_profile_ref: Option<String>` to `agent_profile_ref: String` (required)
  - Remove `agent-config` parsing from `RawFrontmatter` and `parse()`
  - Make `agent-profile-ref` required — parse error if absent
  - Remove `KnotFileError::BothProfileAndConfig` (no longer possible)
  - Remove `KnotFileError::MissingAgentConfigOrProfileRef` — replace with `KnotFileError::MissingProfileRef`
  - Remove `KnotFileError::EmptyGoal`, `EmptyProvider`, `EmptyModel` (profile-level concerns)
  - Update `derive_tieoff_path` doc comment — remove merged duplicate lines (issue #6)
  - Update unit tests: valid knot now requires `agent-profile-ref`, no agent-config tests
  - `cargo test` passes

- [ ] **Domain — `Knot` entity:**
  - Remove `agent_config: Option<AgentConfig>` field
  - Change `agent_profile_ref: Option<String>` to `agent_profile_ref: String` (required)
  - Update entity tests
  - `cargo test` passes

- [ ] **Application — `ProcessStrand`:**
  - Simplify `resolve_agent_config()` — single path: load profile, build `AgentConfig` from it
  - Remove `(None, Some(config))` branch (inline config)
  - Remove `(Some, Some(inline_config))` branch (both set)
  - Remove `(None, None)` branch (impossible with required field)
  - If profile not found → `PortError::ProfileNotFound`
  - System prompt merge strategy unchanged: profile `system_prompt` + knot `prompt_template.instructions`
  - Update unit tests for `resolve_agent_config`
  - `cargo test` passes

- [ ] **Application — `Knot` construction in use cases:**
  - `ConfigEventHandler::handle_knot_added` — `Knot` now has `agent_profile_ref: String`
  - `ConfigEventHandler::handle_knot_modified` — same
  - `ManageKnot` — same
  - `DiscoverLooms` / `RegisterLoom` — pass through from parsed `KnotFile`

- [ ] **Inbound — `KnotRequest`:**
  - Remove `agent_config: Option<AgentConfig>` field entirely
  - Change `agent_profile_ref: Option<String>` to `agent_profile_ref: String` (required)
  - Remove `#[serde(default)]` from `agent_profile_ref` — it is now mandatory
  - Update OpenAPI schema

- [ ] **Inbound — `generate_knot_file()`:**
  - Simplify: always writes `agent-profile-ref` only (no conditional on profile-ref vs inline)
  - Remove `agent-config` YAML generation path
  - Remove `quote_yaml_scalar` calls for agent config fields
  - Update unit test: `generate_knot_file` round-trips through `KnotFile::parse()`

- [ ] **Inbound — handlers:**
  - `register_loom` — build `Knot` with `agent_profile_ref` from request (no agent_config)
  - `create_knot` — same, remove `agent_config` from Knot construction
  - `update_knot` — same
  - Add HTTP validation: `agent_profile_ref` must be non-empty

- [ ] **Integration tests — `shared_agent_profiles.rs`:**
  - Update all knot creation payloads: remove `agent_config`, keep only `agent_profile_ref`
  - `create_pure_profile_ref_knot` — unchanged (already pure profile-ref)
  - `create_knot_with_agent_profile_ref` — remove agent_config from body, add assertion that file has NO `agent-config`
  - `profile_override_at_processing_time` — **rename to `profile_resolution_at_processing_time`**, remove override concept (no inline override possible). Test verifies profile is resolved and strand processed.
  - `dynamic_profile_update_at_processing_time` — update knot body to be profile-ref only
  - `profile_not_found_logs_error` — update knot body
  - `backward_compat_inline_config` — **remove entirely** (no backward compat path)
  - Add test: `POST /looms` with missing `agent_profile_ref` → 400 bad request
  - `cargo test` passes

- [ ] **Integration tests — other modules:**
  - `tests/auto_discovery_and_knot_crud.rs` — update knot creation payloads
  - `tests/loom_crud.rs` — update knot creation payloads
  - `tests/pipeline.rs` — update knot creation payloads
  - `tests/rig_lifecycle.rs` — update knot creation payloads
  - `tests/helpers.rs` — update `make_knot_content` to use `agent-profile-ref`
  - Any test helpers that create inline-config knots must be updated
  - `cargo test` passes (full suite)

### Phase 11: Clippy Cleanup + Remaining Fixes

**Layer:** All (lint)

Fix remaining clippy issues that survived Phase 9.

- [ ] Fix `src/application/debounce.rs:253` — `i % 1` (modulo_one deny). Change to literal `0` or `i % 1` with `#[allow(clippy::modulo_one)]` on the test. Prefer: use `0` directly since the intent is all-same-file.
- [ ] Fix `src/application/usecases.rs:1841` — redundant field name `knot: knot` → `knot`
- [ ] Fix `src/application/debounce.rs:310` — redundant closure `|e| event_path(e)` → `event_path`
- [ ] Fix `src/domain/knot_file.rs:991` — useless `format!(...)` → `.to_string()`
- [ ] Fix `src/application/ports.rs` — `MockAgentRunner::default()` → `MockAgentRunner` (unit struct), same for `MockEventSource`, `MockLoomLogPort`
- [ ] Fix `MockLoomRepository::save` in `ports.rs` test module — store in internal `HashMap` so `get`/`list` return saved data (issue #8, ports-level)
- [ ] Fix `tests/shared_agent_profiles.rs` — useless `format!` calls (lines 760, 1070)
- [ ] Fix `tests/helpers.rs` unused functions — add `#[allow(dead_code)]` on `mod helpers` or remove truly dead helpers
- [ ] Fix `tests/generic_task_management.rs:764` — `MutexGuard` held across await point (drop guard before await)
- [ ] Fix `tests/pipeline.rs` — `single_match` → `if let`, needless borrows
- [ ] Fix `tests/discovery.rs`, `tests/demo.rs`, `tests/loom_crud.rs` — unused variables, needless borrows
- [ ] Fix `tests/rig_lifecycle.rs:279` — unused `Result` from `tokio::time::timeout`
- [ ] `cargo clippy --all-targets` passes with zero warnings
- [ ] `cargo test` passes (full suite)

## Notes

- Profile resolution happens **at processing time** in `ProcessStrand::execute()`, not at discovery time. This ensures that edits to profile `.md` files are picked up on the next strand event without service restart.
- The `goal` field currently in `AgentConfig` is not used by the agent CLI — it's metadata only. Profiles will use `system_prompt` (which IS used as the `--system-prompt` CLI argument). This is a clean separation: `system_prompt` from the profile becomes the system prompt; `goal` from the knot (if present) is ignored in CLI args.
- `tools` defaults to empty Vec in both profiles and knots, matching current behavior.
- The profile repository scans the rig's `profiles/` directory on `list()` calls, and reads individual files on `get()` — both parse fresh from disk each time.
