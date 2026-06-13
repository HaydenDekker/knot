# Plan: Profile Body Storage and Init Default Profile

## Related PRD

This plan contributes to [Knot Skills — AI-Driven Configuration via Skills](../prds/prd-knot-skills.md).

The PRD's Story 1 (Initialise a Knot Rig) covers rig init and loom checking but does not mention profiles. This plan closes that gap: init now checks for profiles, discovers available Pi models, and drafts a first default profile with comment annotations showing alternatives.

## Problem

When a new user runs `knot init`, the rig is initialised but no agent profiles exist. Since every knot requires an `agent-profile-ref`, the user hits a silent wall when trying to create their first loom — there are no profiles to reference. The `knot-init` skill reports "no looms" but never surfaces the profile prerequisite.

Additionally, profile `.md` files are plain YAML frontmatter with no way to attach documentation or notes. When a profile is created with `POST /profiles/{name}`, any markdown body is lost on subsequent saves (the repo preserves the body for existing files, but there's no way to set one on initial creation).

## Target

1. `POST /profiles/{name}` accepts an optional `body` field that writes a custom markdown body to the profile `.md` file
2. `GET /profiles/{name}` returns the `body` field in responses
3. `knot-init` skill discovers models from `~/.pi/agent/models.json`, checks for existing profiles, and if none exist, drafts a first default profile with a comment body documenting available alternatives
4. All tests pass (existing + new)

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `profile_repo.rs` tests (16 tests) | CRUD, body preservation on overwrite, roundtrip, full lifecycle | ✅ Green |
| `shared_agent_profiles.rs` integration (9 tests) | Profile CRUD via HTTP, knot creation with profile ref, resolution | ✅ Green |
| `knot_file.rs` — `parse_agent_profile` (11 tests) | Valid/invalid profiles, missing fields, malformed YAML | ✅ Green |
| `skill_integration.rs` | Skill file existence, endpoint refs, workflow simulation | ✅ Green |
| `loom.rs` inbound handler tests | Profile create/list/get/delete handlers | ✅ Green |

## Test Gaps

- No test for `POST /profiles/{name}` with a `body` field — body must appear in the written file and be readable back
- No test that body survives subsequent saves (partial body preservation already tested, but explicit body on creation is not)
- No test for `GET /profiles/{name}` returning body in response
- No skill-level test that `knot-init` references profile endpoints
- No integration test for the full init → draft profile → create loom flow

## Phases

### Phase 0: API — Profile body in request/response types + trait
- [ ] Add `body: Option<String>` to `ProfileRequest` (done, uncommitted)
- [ ] Add `body: Option<String>` to `ProfileResponse` (done, uncommitted)
- [ ] Change `AgentProfileRepository::save(profile)` → `save(profile, body: Option<String>)` (done, uncommitted)
- [ ] Update all trait implementations: `FileSystemAgentProfileRepository`, mock repos in `ports.rs`, `loom.rs`, `usecases.rs`, and integration test files (`swagger_ui.rs`, `skill_integration.rs`) (done, uncommitted)
- [ ] Update `create_profile` handler to pass `body.body` to `save()` (done, uncommitted)
- [ ] Update `list_profiles` and `get_profile` handlers to include `body` in response (done, uncommitted — currently `None` since body isn't stored in domain entity)
- [ ] Add test: `POST /profiles/{name}` with body writes custom markdown body to file
- [ ] Add test: `GET /profiles/{name}` returns body field
- [ ] Add test: explicit body on first save is preserved on subsequent save with `None`
- [ ] Verify `cargo test` passes

### Phase 1: Inbound — Read body back from disk in GET/list
- [ ] Add `get_body(name: &str) -> Option<String>` to `AgentProfileRepository` trait (reads markdown body from disk)
- [ ] Implement `get_body` in `FileSystemAgentProfileRepository` using `extract_body()` on the profile file
- [ ] Implement `get_body` in mock repos (returns `None`)
- [ ] Update `get_profile` handler to include body from `get_body()`
- [ ] Update `list_profiles` handler to include body from `get_body()`
- [ ] Update `ProfileResponse` usage in tests
- [ ] Verify `cargo test` passes

### Phase 2: Skill — knot-init discovers models and drafts default profile
- [ ] Add `GET /profiles` check to `knot-init` skill workflow (after rig config check)
- [ ] If profiles list is empty, instruct the agent to:
  - Read `~/.pi/agent/models.json` to discover available providers and models
  - Pick a reasonable default (first model from first provider)
  - Draft a `POST /profiles/default` with:
    - `provider`: first provider key from models.json (e.g. `llama-workhorse`)
    - `model`: first model id from that provider (e.g. `qwen3-27b`)
    - `tools`: `["read", "write", "ls", "bash"]` (Pi's default tool set)
    - `system_prompt`: a sensible default for a Knot agent
    - `body`: markdown body with `#`-commented sections documenting:
      - Other available models from models.json (grouped by provider)
      - Other available tools from Pi's built-in tool set
      - How to change provider/model via `POST /profiles/default`
      - How to add more profiles
- [ ] Add `GET /profiles/{name}` verification after creation
- [ ] Update skill's "Cross-Reference" section to mention profile creation
- [ ] Update skill's endpoint table to include `/profiles` and `/profiles/{name}`

### Phase 3: Integration — End-to-end init-to-loom flow
- [ ] Add integration test in `tests/skill_integration.rs` (or new test file) that:
  - Starts mock server with empty profiles
  - Simulates `knot-init` reading empty profile list
  - Simulates creating a default profile with body
  - Verifies profile exists and body is readable
- [ ] Verify full `cargo test` passes

## Notes

- The `AgentProfile` domain entity does **not** store body — body is file-level metadata. The `AgentProfileRepository` port handles the roundtrip. This keeps the domain clean and the body as documentation only.
- `extract_body()` already exists in `profile_repo.rs` and is used for preservation during overwrite. Phase 1 exposes this read capability through the trait so the HTTP layer can return it.
- The skill reads `~/.pi/agent/models.json` at agent time (not server time). This is the Pi models config file — it exists on the developer's machine and is where the init skill discovers available models. The skill instructs the agent to read it, not the server.
- Comment lines in the markdown body (`# other models: ...`) are pure documentation — they survive YAML frontmatter parsing since the body is everything after the closing `---` delimiter.
