# Plan: Knot Domain Models

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan establishes the domain layer — entities, value objects, domain events, and validation logic. Zero IO, zero framework imports. Pure Rust.

## Problem

Knot has no domain model for its core concepts. There are no types representing knots, looms, strands, or tie-offs. Knot definition files have no specified format and no parser to read them. Without these, no loom discovery, file watching, or agent execution can be implemented.

## Target

- Domain entities: `Knot`, `Loom`, `Strand`, `TieOff`
- Value objects: `AgentConfig`, `PromptTemplate`, `WorkspaceAgentConfig`, `KnotId`, `LoomId`, `StrandPath`, `TieOffPath`
- Domain events: `KnotRegistered`, `StrandEvent`, `TieOffProduced`, `ProcessingFailed`, `LoomEvent`
- Knot file format defined and validated (parser lives in adapters — not this plan)
- Domain validation logic (invariants, not IO)
- All domain types in `src/domain/`, no dependencies on framework crates

## Implementation Status: ⬜ Draft

## Hex Layer: Domain

No ports, no adapters, no IO. This layer depends on nothing except `std` and `serde` (for serialization).

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |

## Test Gaps

- No domain entity tests (construction, validation, invariants).
- No value object tests (parsing, defaults, validation).
- No domain event tests (construction, serialization).
- No knot file format validation tests.

## Proposed Knot File Format

A knot definition is a markdown file with YAML frontmatter:

```markdown
---
name: prd-goals-review
agent-config:
  goal: "Review PRD goals for clarity, completeness, and alignment"
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review the goals section of this PRD. Check that:
    - Each goal is specific and measurable
    - Goals align with the problem statement
    - Non-goals are not accidentally included as goals
---

# PRD Goals Review Knot

This knot reviews the goals section of PRD documents.
```

Agent config (`cli_path`, `cli_args`) is workspace-level — one config for the whole workspace. The knot file carries only the goal and prompt template.

## Phases

### Phase 0: Domain Entities
**Failing tests created:** `domain::entities::tests::knot_construction`, `domain::entities::tests::loom_construction`

- [x] Failing test: `domain::entities::tests::knot_construction` — construct a `Knot` with a `KnotId`, `AgentConfig`, and `PromptTemplate`; verify fields
- [x] Failing test: `domain::entities::tests::loom_construction` — construct a `Loom` with `LoomId`, source dir, tie-off point, and `Vec<Knot>`; verify fields
- [x] Failing test: `domain::entities::tests::strand_construction` — construct a `Strand` from a path; verify path is stored
- [x] Failing test: `domain::entities::tests::tieoff_construction` — construct a `TieOff` with content, path, and status; verify fields
- [x] Implement `Knot`, `Loom`, `Strand`, `TieOff` structs in `src/domain/entities.rs`
- [x] Implement `KnotId`, `LoomId`, `StrandPath`, `TieOffPath` as newtype wrappers
- [ ] Add `serde::Serialize + Deserialize` derives

### Phase 1: Value Objects
**Failing tests created:** `domain::value_objects::tests::agent_config_defaults`, `domain::value_objects::tests::prompt_template_fields`, `domain::value_objects::tests::workspace_agent_config_defaults`

- [x] Failing test: `domain::value_objects::tests::agent_config_defaults` — `AgentConfig` with goal string; verify non-empty goal required
- [x] Failing test: `domain::value_objects::tests::prompt_template_fields` — `PromptTemplate` with `input_bundling` and `instructions`; verify both required
- [x] Failing test: `domain::value_objects::tests::workspace_agent_config_defaults` — `WorkspaceAgentConfig` defaults to `cli_path = "pi"`, `cli_args = []`; verify custom path and args accepted
- [x] Implement `AgentConfig` (goal), `PromptTemplate` (input_bundling, instructions), `WorkspaceAgentConfig` (cli_path, cli_args) in `src/domain/value_objects.rs`
- [x] Implement `try_from` or constructor with validation — empty goal returns error

### Phase 2: Domain Events
**Failing tests created:** `domain::events::tests::strand_event_types`, `domain::events::tests::tieoff_produced_event`, `domain::events::tests::processing_failed_event`, `domain::events::tests::loom_event_types`

- [x] Failing test: `domain::events::tests::strand_event_types` — construct `StrandEvent::Created`, `Modified`, `Deleted` with loom ID, knot ID, strand path; verify all variants
- [x] Failing test: `domain::events::tests::tieoff_produced_event` — `TieOffProduced` with knot ID, strand path, tie-off path; verify serialisation
- [x] Failing test: `domain::events::tests::processing_failed_event` — `ProcessingFailed` with knot ID, strand path, error message; verify error details preserved
- [x] Failing test: `domain::events::tests::loom_event_types` — `LoomEvent::KnotRegistered`, `LoomStarted`, `LoomStopped`, `StrandProcessed`; verify all variants
- [x] Failing test: `domain::events::tests::knot_registered_event` — `KnotRegistered` with loom ID and knot ID; verify construction
- [x] Implement domain event enums in `src/domain/events.rs`
- [ ] Events are serialisable (for writing to loom-log / knot-state later)

### Phase 3: Knot File Format Validation
**Failing tests created:** `domain::knot_file::tests::valid_knot_file_parse`, `domain::knot_file::tests::missing_name_returns_error`, `domain::knot_file::tests::empty_goal_returns_error`, `domain::knot_file::tests::missing_prompt_template_returns_error`

- [ ] Failing test: `domain::knot_file::tests::valid_knot_file_parse` — given a well-formed frontmatter string, produce a `KnotFile` with name, agent_config, prompt_template
- [ ] Failing test: `domain::knot_file::tests::missing_name_returns_error` — frontmatter without `name` field returns `KnotFileError::MissingName`
- [ ] Failing test: `domain::knot_file::tests::empty_goal_returns_error` — frontmatter with empty goal returns `KnotFileError::EmptyGoal`
- [ ] Failing test: `domain::knot_file::tests::missing_prompt_template_returns_error` — frontmatter without prompt-template returns `KnotFileError::MissingPromptTemplate`
- [ ] Failing test: `domain::knot_file::tests::malformed_yaml_returns_error` — invalid YAML in frontmatter returns `KnotFileError::InvalidFormat`
- [ ] Implement `KnotFile` struct and `KnotFileError` in `src/domain/knot_file.rs`
- [ ] Implement frontmatter parser (split on `---`, parse YAML portion into `serde_json` or `serde_yaml`)
- [ ] The parser takes a `String` (file content) — it does not read from the filesystem. Reading is an adapter concern.

## Notes
