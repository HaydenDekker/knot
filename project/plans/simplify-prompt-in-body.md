# Plan: Simplify Prompts — Move Prompt Text to Markdown Body

## Problem

Both agent profiles and knot definition files embed the main prompt content inside YAML frontmatter block scalars (`profile-prompt: |` and `prompt-template.instructions: |`). The markdown body after the closing `---` is unused documentation — the parser extracts it into `AgentProfile.body` but never reads it at runtime.

This is backwards from normal markdown convention:
- The "meaty" content (prompts) is trapped in indentation-sensitive YAML
- The human-readable body area is wasted on duplicate summaries
- Refining a prompt means fighting YAML indentation and produces noisy diffs

## Target

All plain text after the frontmatter closing `---` becomes the prompt content directly:
- **Profiles**: body = `profile_prompt` (replaces `profile-prompt` frontmatter field)
- **Knots**: body = `prompt-template.instructions` (replaces `prompt-template.instructions` frontmatter field)

Frontmatter keeps only structural metadata:
- Profile: `name`, `provider`, `model`, `tools`, `timeout`
- Knot: `name`, `agent-profile-ref`, `strand-dir`, `git-versioned`

The `body` field on `AgentProfile` is removed — there's no distinction between "prompt" and "body" anymore.

Example profile after change:
```markdown
---
name: fast
provider: openai
model: gpt-4o
tools:
  - read
  - bash
---

You are a fast reviewer. Keep responses concise and direct.
```

Example knot after change:
```markdown
---
name: goals-review
agent-profile-ref: fast
strand-dir: "project/prds"
---

Review the goals section of this PRD. Check that:
- Each goal is specific and measurable
- Goals align with the problem statement
```

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `knot_file.rs` inline tests | Knot file parsing (18 tests), profile parsing (14 tests) | ✅ Green — all use frontmatter prompts |
| `value_objects.rs` inline tests | AgentProfile, PromptTemplate construction/serialization | ✅ Green |
| `profile_repo.rs` inline tests | Profile CRUD (16 tests) — fixtures use frontmatter `profile-prompt` | ✅ Green |
| `usecases.rs` inline tests | ProcessStrand, config handling — use `PromptTemplate` directly | ✅ Green |
| `loom_repository.rs` inline tests | Loom/knot scanning — fixtures use frontmatter instructions | ✅ Green |
| `event_source.rs` inline tests | Knot reload on modification — fixtures use frontmatter instructions | ✅ Green |
| `subprocess.rs` inline tests | Prompt building (profile_prompt + instructions + trigger) | ✅ Green — structural, format-agnostic |
| Integration tests (`tests/`) | End-to-end pipeline — create files, watch, process | ✅ Green — fixtures use frontmatter prompts |

## Test Gaps

- No test for empty body (missing prompt text) — new validation needed
- No test for whitespace-only body — new validation needed
- No test for body extraction edge cases (body with frontmatter-like `---` inside)

## Phases

### Phase 0: Domain — Parsing changes (`knot_file.rs`)

Core parsing logic. Both parsers must work before anything else can follow.

**Profile parsing (`parse_agent_profile`):**
- Remove `profile-prompt` from `RawProfileFrontmatter`
- Extract body text from the markdown content (reuse existing `extract_frontmatter` logic but capture the body portion)
- Use body text as `profile_prompt`
- Body is now required — empty/whitespace body returns `AgentProfileError::MissingProfilePrompt` (reuse existing error)
- Add `MissingBody` variant to `AgentProfileError` for the case where there's no body at all (no content after closing `---`)
- Remove `body` parameter from `AgentProfile::with_tools()` / `AgentProfile::new()` — it's gone

**Knot parsing (`parse`):**
- Remove `prompt-template` from `RawFrontmatter`
- Remove `RawPromptTemplate` struct entirely
- Extract body text from the markdown content
- Use body text as `PromptTemplate.instructions`
- Body is now required — empty/whitespace body returns `KnotFileError::MissingPromptTemplate` (reuse existing error)
- Add `MissingBody` variant to `KnotFileError` for the case where there's no body at all

**Shared body extraction:**
- Create a shared `extract_frontmatter_and_body(content) -> Result<(String, Option<String>), _>` helper that returns both the YAML text and the body text after the closing `---`
- Both parsers use this shared helper

- [x] Add `MissingBody` error variants to `KnotFileError` and `AgentProfileError`
- [x] Implement shared `extract_frontmatter_and_body()` helper
- [x] Refactor `parse()` to use body as instructions
- [x] Refactor `parse_agent_profile()` to use body as profile_prompt
- [x] Remove `RawPromptTemplate` struct
- [x] Remove `profile-prompt` from `RawProfileFrontmatter`
- [x] Update all inline test fixtures in `knot_file.rs`

### Phase 1: Domain — Value Object cleanup (`value_objects.rs`)

- [x] Remove `body: Option<String>` field from `AgentProfile`
- [x] Remove `AgentProfile::with_body()` method
- [x] Update `AgentProfile::new()` and `AgentProfile::with_tools()` — no body parameter
- [x] Update `AgentProfile` doc comments
- [x] Update inline tests for AgentProfile

### Phase 2: Outbound Adapters — Profile repository cleanup (`profile_repo.rs`, `ports.rs`)

`save` and `delete` are never called from production code (only in tests). The HTTP endpoints that used them were removed in plan #26 (file-first). Remove the dead write path entirely.

**Port trait (`ports.rs`):**
- Remove `save()` and `delete()` from `AgentProfileRepository` trait
- Remove `PortError::ProfileSaveFailed` variant (no longer reachable)
- Update `MockAgentProfileRepository` to remove `save`/`delete`
- Remove port contract test for save/delete

**`FileSystemAgentProfileRepository` (`profile_repo.rs`):**
- Remove `save()` implementation and its `ensure_dir()` helper
- Remove `delete()` implementation
- Remove `extract_body()` helper — body IS the prompt, parsed by `parse_agent_profile`
- **`get()` simplifies**: remove `.with_body()` call (body field is gone from AgentProfile)
- Keep `list()` and `get()` — these are used in production (StateWriter, ProcessStrand)

- [x] Remove `save()` and `delete()` from `AgentProfileRepository` trait
- [x] Remove `PortError::ProfileSaveFailed` variant
- [x] Remove `MockAgentProfileRepository::save`/`delete`
- [x] Remove `FileSystemAgentProfileRepository::save`, `::delete`, `::ensure_dir`, `::profile_path`
- [x] Update `get()` to remove `.with_body()` call
- [x] Remove `extract_body()` helper
- [x] Remove/update save and delete test cases from `profile_repo.rs`
- [x] Update port contract tests in `ports.rs`
- [x] Update usecase test fixtures that call `save` on mock profile repo
- [x] Fix `create_profile_file()` test helper to not include `profile-prompt` in frontmatter

### Phase 3: Outbound Adapters — Loom repository + Event source

These use `KnotFile::parse()` so they get the new format automatically, but their **test fixtures** need updating.

- [ ] Update test fixtures in `loom_repository.rs` (remove `prompt-template.instructions` from frontmatter, move to body)
- [ ] Update test fixtures in `event_source.rs` (same pattern)
- [ ] Verify `event_source.rs` knot reload logic works (uses `KnotFile::parse()` directly)
- [ ] Verify `loom_repository.rs` scanning works (uses `KnotFile::parse()` directly)

### Phase 4: Application Layer — Use cases

`ProcessStrand` uses `knot.prompt_template.instructions` and `profile.profile_prompt` — these still exist as fields, just populated from a different source. No structural change needed.

- [ ] Update inline test fixtures in `usecases.rs` that construct `Knot`/`AgentProfile` directly (no change needed — constructors are unchanged)
- [ ] Update any test fixtures that create raw knot file content strings

### Phase 5: Integration Tests

Integration tests create real `.md` files and verify end-to-end processing. All knot file and profile file fixtures need updating.

- [ ] Audit all integration test files under `tests/` for knot/profile file fixtures
- [ ] Update fixtures: move prompt text from frontmatter to body
- [ ] Run full integration test suite

### Phase 6: Update Skills and Documentation

Update the agent skill and all docs to reflect the new format.

- [ ] Update `.agents/skills/knot-create/SKILL.md` — profile and knot file format sections
- [ ] Update `project/domain-glossary.md` — Agent Profile and Knot definitions
- [ ] Update `docs/configuration/profiles.md`
- [ ] Update `docs/configuration/knots.md`
- [ ] Update `docs/getting-started.md` (any format examples)
- [ ] Update `docs/workflows/*.md` (any format examples)

### Phase 7: Create `knot-update` skill

Create a new agent skill at `.agents/skills/knot-update/SKILL.md` that records format changes between Knot versions. When a project updates its Knot binary, the skill tells the agent what changed in project documents and how to fix them.

This is the first use of the skill — the prompt-in-body change is a version-breaking format migration that needs documentation.

The skill file itself contains:
- A `## Changelog` section with versioned entries
- Migration instructions for each breaking change (what to search for, what to replace)
- A brief description of how the skill is used by other projects

The first changelog entry documents this change:

```
## 0.18.0 — Prompt text moved to markdown body (2026-06-24)

**Profiles** (`rig/profiles/*.md`):
- Remove `profile-prompt: |` from frontmatter
- Move the prompt text to the body (after closing `---`)
- Remove duplicate heading/body if present

**Knots** (`rig/*-loom/*.md`):
- Remove `prompt-template:\n  instructions: |` from frontmatter
- Move the instruction text to the body (after closing `---`)
- Remove duplicate heading/body if present
```

After creating the skill, publish it globally:
```bash
cp -r .agents/skills/knot-update ~/.agents/skills/knot-update
```

- [ ] Create `.agents/skills/knot-update/SKILL.md` with format and first changelog entry
- [ ] Publish skill globally (`cp -r` to `~/.agents/skills/`)

### Phase 8: Demo Rig Files and Version

- [ ] Update `rig/workflow-loom/review-knot.md`
- [ ] Update `rig/new-loom/review-knot.md`
- [ ] Bump `Cargo.toml` version (SemVer MINOR — breaking change to file format)
- [ ] Run full test suite, verify clippy clean
- [ ] Update `docs/release-notes.md` with format change note

## Notes

- This is a **breaking change** to the file format. Existing knot and profile files with frontmatter prompts will fail to parse. The error messages should be clear: "missing body (prompt text must be in the markdown body, not frontmatter)".
- Consider whether to add a **migration warning**: if the parser detects `profile-prompt` or `prompt-template.instructions` in frontmatter AND an empty body, emit a helpful error suggesting the migration.
- `PromptTemplate` struct remains (single `instructions: String` field) — it's a useful domain wrapper even though it wraps one string.
- The runtime prompt assembly (`profile_prompt` → `prompt_template.instructions` → trigger line) is unchanged. Only the data source changes.
