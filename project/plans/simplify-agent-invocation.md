# Plan: Simplify Agent Invocation — Remove --system-prompt, Merge Profile Prompt into Stdin

## Problem

The agent profile's `system_prompt` is currently injected into `pi` via `--system-prompt` CLI flag. This creates two problems:

1. **Duplication**: knot instructions appear in both the system prompt (merged via `resolve_agent_config`) and the user message (stdin), wasting context tokens.
2. **Invisible in session**: pi does not persist `--system-prompt` into the session `.jsonl` file — the profile's persona instructions are sent to the LLM but never recorded, making sessions opaque when inspected.

The current flow splits the prompt across two channels:
- `--system-prompt`: `<profile.system_prompt>\n\n<knot.instructions>`  (merged in `resolve_agent_config`)
- stdin (user message): `<trigger line>\n\n<knot.instructions>\n\n<@file>` 

The knot instructions are duplicated. The profile's system prompt is invisible in session storage.

## Target

Simplify to a single stdin-only prompt. Remove `--system-prompt` entirely. The stdin becomes a single coherent user message:

```
<profile-prompt>

<knot instructions>

**<knot-name>** triggered by **<event-type>** on **<strand-path>**

<@strand-file>
```

Rename `system_prompt` → `profile_prompt` in the `AgentProfile` entity and YAML frontmatter (key becomes `profile-prompt`), making it clear this is a profile-level prompt segment, not a system role message.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `domain::value_objects::tests::agent_config_build_cli_args_*` | `AgentConfig::build_cli_args` produces correct CLI args including `--system-prompt` | ✅ Green — 5 tests |
| `application::usecases::tests::*resolve_agent_config*` | Profile resolution, system prompt merging, timeout extraction | ✅ Green — 4 tests |
| `adapters::subprocess::tests::runner_passes_*` | Stdin prompt delivery, event metadata prepending | ✅ Green — 3 tests |
| `adapters::outbound::profile_repo::tests::*` | Profile parsing, save/load roundtrip with `system-prompt` YAML key | ✅ Green — 12 tests |
| `domain::knot_file::tests::*parse_agent_profile*` | Frontmatter parsing of `system-prompt` field | ✅ Green — 10 tests |
| `domain::value_objects::tests::agent_profile_*` | AgentProfile creation, validation, serialization | ✅ Green — 14 tests |
| `tests/profile_timeout.rs` (integration) | Profile timeout in ProcessStrand execution | ✅ Green — 3 tests |

## Test Gaps

- No integration test that validates the exact CLI args produced by `ProcessStrand` end-to-end (args are built in usecases.rs but never asserted against in integration tests)
- No test that verifies the full stdin content delivered to the agent runner

## Phases

### Phase 0: Domain — Rename `system_prompt` → `profile_prompt` in `AgentProfile`

- [x] Rename `AgentProfile.system_prompt` field to `profile_prompt` in `src/domain/value_objects.rs`
- [x] Update `serde(rename = "system-prompt")` → `serde(rename = "profile-prompt")`
- [x] Update `AgentProfile::new()` and `AgentProfile::with_tools()` parameter name
- [x] Update `AgentProfileError::MissingSystemPrompt` → `MissingProfilePrompt`
- [x] Update all unit tests in `value_objects.rs` (agent_profile_* tests)
- [x] Update `parse_agent_profile()` in `knot_file.rs`: rename `system_prompt` field in `RawProfileFrontmatter`, update `serde(rename)`, update validation error mapping
- [x] Update all unit tests in `knot_file.rs` (profile parsing tests)
- [x] Update `FileSystemAgentProfileRepository` tests that reference `system_prompt`
- [x] Compile check: `cargo build`

**Additionally completed**: Updated `ProfileResponse` in inbound types, all handler mappings in `loom.rs`, `usecases.rs` references, and all integration test profile fixtures across 10 test files.

**Rationale**: Renaming first makes the intent clear — this is a profile-level prompt segment, not an HTTP/API system role. The serde rename keeps the YAML key distinct (`profile-prompt`), which will require updating any existing profile files.

### Phase 1: Application — Remove `--system-prompt` from CLI args, merge into stdin

- [ ] In `AgentConfig::build_cli_args()` (`value_objects.rs`): remove `system_prompt` parameter and `--system-prompt` from returned args vector. Signature becomes `build_cli_args(&self, template: &PromptTemplate) -> Vec<String>`
- [ ] In `ProcessStrand::resolve_agent_config()` (`usecases.rs`): remove system prompt merging logic. Return `(AgentConfig, Option<Duration>)` instead of `(AgentConfig, String, Option<Duration>)` — the profile_prompt is no longer part of the return tuple
- [ ] In `ProcessStrand::execute()` (`usecases.rs`): call `build_cli_args()` without the system_prompt argument. Build the full stdin prompt as: `<profile.profile_prompt>\n\n<knot.instructions>\n\n<trigger line>`
- [ ] In `SubprocessAgentRunner::build_prompt_with_context()` (`subprocess.rs`): accept `profile_prompt: &str` parameter, prepend it to the prompt before trigger line + original prompt. Signature becomes `build_prompt_with_context(ctx: &ExecutionContext, profile_prompt: &str) -> String`
- [ ] In `ExecutionContext` (`ports.rs`): add `profile_prompt: String` field
- [ ] Update `ProcessStrand::execute()` to populate `ctx.profile_prompt` from the resolved profile
- [ ] Update all usecase unit tests that call `build_cli_args()` or `resolve_agent_config()`
- [ ] Update subprocess unit tests that construct `ExecutionContext`
- [ ] Compile check: `cargo build`, then `cargo test`

**Rationale**: The profile prompt moves from CLI args into the stdin prompt chain. `ExecutionContext` gains a `profile_prompt` field so the runner can prepend it. The stdin ordering is: profile prompt (persona) → knot instructions (task) → trigger line (event context) → @file reference (input data).

### Phase 2: Integration tests and existing profile file compatibility

- [ ] Update integration tests in `tests/profile_timeout.rs` if they reference `system_prompt` or `--system-prompt`
- [ ] Update integration tests in `tests/agent_profile_crud.rs` if they reference `system-prompt` YAML key
- [ ] Add a migration note: existing profile files use `system-prompt:` YAML key — Knot will fail to parse them. Document the rename (`system-prompt` → `profile-prompt`) in the plan notes.
- [ ] Run full test suite: `cargo test`
- [ ] Run clippy: `cargo clippy -- -D warnings`
- [ ] Update domain glossary if `system-prompt` / `system prompt` is referenced

**Rationale**: Profile files are user-facing config. The YAML key change is a breaking change for existing rigs. The migration path is: edit each `rig/profiles/*.md` file, change `system-prompt:` to `profile-prompt:`.

## Notes

- This is a breaking change for existing profile files (`system-prompt` → `profile-prompt` in YAML frontmatter). Users must update their `rig/profiles/*.md` files.
- The `--system-prompt` flag is a pi CLI feature — removing it from Knot means Knot no longer relies on pi's system prompt mechanism. This is a simplification, not a loss of capability.
- The profile prompt in stdin is effectively the same as a system prompt for the LLM — it's the first text in the conversation. The difference is it's now visible in the session file.
