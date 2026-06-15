# Plan: Agent Profile Skills — Explicit Skill Selection for Agent Invocations

## Related PRD

This plan contributes to [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md), Story 8: Shared Agent Profiles.

The PRD defines the agent profile as including "skills available to the agent (driving the prompt)" but the current implementation only covers provider, model, tools, system-prompt, and timeout. This plan adds skills as a first-class profile field.

## Problem

When Knot invokes `pi` to run an agent session, it passes `--model`, `--system-prompt`, and optionally `--tools`. It does **not** pass any skill-related flags. This means `pi` auto-discovers skills from all locations (`~/.agents/skills/`, `.agents/skills/`, settings entries, packages). Every available skill's description appears in the agent's system prompt, bloating context regardless of whether the skill is relevant to the task.

The user has no way to say "this knot's agent should only know about these three skills." The global skill set becomes the session's skill set by default.

## Target

When this plan is complete:

- `AgentProfile` has an optional `skills` field — a list of skill **names** (e.g. `["knot-inspect", "rust-project-init"]`)
- Profile `.md` files accept `skills:` in YAML frontmatter
- When a profile specifies skills, Knot passes `--no-skills` + `--skill <path>` for each skill to the `pi` CLI, making the skill set **exclusive** to the session
- When a profile has no `skills` field (or it is empty), Knot passes no skill flags — `pi` uses its normal auto-discovery (backwards compatible)
- Skills are resolved by name to filesystem paths — Knot searches known locations (`~/.agents/skills/`, project `.agents/skills/`, `~/.pi/agent/skills/`) for a directory matching the skill name containing a `SKILL.md`
- Resolution is validated at profile parse time — missing skills produce a parse error so the user knows before any agent session starts
- The HTTP `/profiles/{name}` endpoint includes `skills` in its response

### Profile File Format (after this plan)

```yaml
---
name: reviewer
provider: anthropic
model: claude-sonnet
system-prompt: |
  You are a code reviewer.
skills:
  - rust-review
  - project-planner
timeout: 300
---
```

### CLI Invocation (when skills are specified)

```
pi -p --model claude-sonnet --system-prompt "..." --no-skills --skill /home/user/.agents/skills/rust-review/SKILL.md --skill /home/user/.agents/skills/project-planner/SKILL.md
```

### CLI Invocation (when no skills specified)

```
pi -p --model claude-sonnet --system-prompt "..."
```

(no skill flags — pi uses its normal discovery)

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `knot_file.rs` — `parse_agent_profile` tests | Profile parsing: name, provider, model, system-prompt, tools, timeout | ✅ Green |
| `value_objects.rs` — `AgentConfig::build_cli_args` | CLI arg construction with tools, system-prompt | ✅ Green |
| `agent_profile.rs` (domain tests) | `AgentProfile::new`, `with_tools`, `with_timeout` | ✅ Green |

## Test Gaps

- No test for `skills` field on `AgentProfile` (field doesn't exist yet)
- No test for skill name → path resolution
- No test for missing skill resolution error
- No test for `--no-skills` + `--skill` in CLI args
- No test for empty skills list (should produce no skill flags)

## Phases

### Phase 0: Domain — Add `skills` field to `AgentProfile` and resolution logic

**Hex Layer:** Domain

Add `skills: Vec<String>` to `AgentProfile`. Add `AgentProfile::with_skills()` builder. Add a standalone `resolve_skill_paths(skills: &[String], project_root: &Path) -> Result<Vec<PathBuf>, SkillResolveError>` function in the domain layer that:

1. Accepts a list of skill names
2. Searches `~/.agents/skills/<name>/SKILL.md`, `~/.pi/agent/skills/<name>/SKILL.md`, `<project_root>/.agents/skills/<name>/SKILL.md` in order
3. Returns the list of resolved absolute paths
4. Returns `SkillResolveError::SkillNotFound(name)` if any skill cannot be found

- [ ] Add `skills` field and `with_skills()` to `AgentProfile`
- [ ] Define `SkillResolveError` domain error type
- [ ] Implement `resolve_skill_paths()` — pure function, no IO (accepts candidate base paths)
- [ ] Domain unit tests: `with_skills()`, `resolve_skill_paths()` with found/not-found, empty list

### Phase 1: Knot File Parser — Accept `skills` in profile frontmatter

**Hex Layer:** Domain (parsing)

Update `RawProfileFrontmatter` to include `skills: Option<Vec<String>>`. Update `parse_agent_profile()` to pass skills through to `AgentProfile::with_skills()`.

- [ ] Add `skills` to `RawProfileFrontmatter`
- [ ] Wire skills through `parse_agent_profile()` → `AgentProfile::with_skills()`
- [ ] Domain unit tests: valid skills, empty skills list, missing skills field, profile with all fields

### Phase 2: CLI Args — Pass `--no-skills` + `--skill` to `pi`

**Hex Layer:** Domain (CLI construction)

Update `AgentConfig::build_cli_args()` to accept an optional skills parameter. When skills are provided, append `--no-skills` followed by `--skill <path>` for each resolved path. When no skills are provided, produce no skill flags.

- [ ] Add `skills: Option<&[PathBuf]>` parameter to `build_cli_args()`
- [ ] When skills present: append `--no-skills`, then `--skill <path>` per skill
- [ ] When skills absent or empty: produce no skill flags
- [ ] Domain unit tests: with skills, without skills, empty skills
- [ ] Update callers of `build_cli_args()` to pass the skills list (trace through `ExecutionContext` → `ProcessStrand` → `SubprocessAgentRunner`)

### Phase 3: Skill Resolution Wiring — Resolve skill names at processing time

**Hex Layer:** Application → Outbound Adapters

The skill resolution function in Phase 0 is pure (accepts candidate paths). Wire it so that:

1. `ProcessStrand` resolves skill names to paths using the rig directory and home directory as candidate bases
2. Resolution happens at processing time (same as profile resolution) — not at startup
3. Resolution failures produce a `KnotFailed` in the loom-log (non-fatal, preserves existing tie-off)

- [ ] Add skill resolution step in `ProcessStrand::execute()` after profile resolution
- [ ] Pass resolved skill paths into `build_cli_args()`
- [ ] Handle resolution errors — write `KnotFailed` to loom-log, preserve tie-off
- [ ] Integration test: profile with skills resolves paths and agent is invoked with correct flags

### Phase 4: HTTP — Include `skills` in profile endpoint response

**Hex Layer:** Inbound Adapter

Ensure `GET /profiles` and `GET /profiles/{name}` include the `skills` field in JSON responses (automatic via `utoipa::ToSchema` derive on `AgentProfile`).

- [ ] Verify `AgentProfile` derives `utoipa::ToSchema` (already does)
- [ ] Integration test: `GET /profiles/{name}` returns `skills` array

### Phase 5: Integration Test — End-to-end skill invocation

**Hex Layer:** Integration

End-to-end test using a stub `pi` script that captures its arguments. Verify:

- Profile with skills → stub receives `--no-skills` + `--skill <path>` for each
- Profile without skills → stub receives no skill flags
- Missing skill → `KnotFailed` in loom-log, no agent invocation

- [ ] Create stub `pi` that writes its args to a temp file
- [ ] Test: profile with 2 skills → correct CLI args
- [ ] Test: profile with no skills → no skill flags
- [ ] Test: profile with missing skill → knot fails, tie-off preserved

## Notes

- `pi`'s `--skill <path>` is repeatable and additive even with `--no-skills`. Using `--no-skills` + explicit `--skill` paths makes the skill set **exclusive** to the session — pi won't also load skills from discovery locations. This keeps agent context concise.
- Skill resolution is kept simple: name → directory lookup in known locations. No globbing, no pattern matching, no relative paths from the profile. If the user needs more control, they can use absolute `--skill` paths directly in the profile later.
- The PRD defines skills as part of the agent profile. This plan keeps skills at the profile level only — no knot-level override. If knot-level skill customization is needed later, it can be a separate plan that extends the profile's skills list at processing time.
- This is a draft — the feature is small and focused. Activation can happen when the user decides to implement it.
