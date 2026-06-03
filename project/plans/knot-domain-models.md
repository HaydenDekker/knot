# Plan: Knot Domain Models

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan establishes the core domain types — `Knot`, `AgentProfile`, `PromptTemplate`, `Loom` — and the knot file format and parser. It is the foundation all other plans build on.

## Problem

Knot has no domain model for its core concepts. There are no types representing knots, looms, agent profiles, or prompt templates. Knot definition files have no specified format and no parser to read them. Without these, no loom discovery, file watching, or agent execution can be implemented.

## Target

- Domain types (`Knot`, `AgentProfile`, `PromptTemplate`, `Loom`, `KnotFile`) exist in the domain layer with clear relationships.
- A knot definition file format is proposed and a parser can read it into a `Knot` struct.
- Agent config is workspace-level (one config for the whole workspace), not per-knot. The CLI path defaults to `pi` but is configurable.
- Prompt template captures goal description and input bundling rules.
- All types are serialisable and validated.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |

## Test Gaps

- No tests for knot file parsing (format, valid/invalid files, missing fields).
- No tests for loom model construction (valid/invalid configurations).
- No tests for agent profile validation.
- No tests for prompt template structure.

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

The `agent-config` block captures the *what* (goal description) and the `prompt-template` captures the *how* (input bundling + instructions). The CLI args (`--no-tools`, etc.) are workspace-level, not in the knot file.

## Phases

### Phase 0: Domain Types
- [ ] Define `Knot` struct with `name`, `agent_config`, `prompt_template` fields
- [ ] Define `AgentConfig` struct with `goal` field
- [ ] Define `PromptTemplate` struct with `input_bundling` and `instructions` fields
- [ ] Define `WorkspaceAgentConfig` (top-level) with `cli_path` (default: `"pi"`), `cli_args` (Vec<String>)
- [ ] Define `Loom` struct with `id`, `source_dir`, `tie_off_point`, `knots`
- [ ] Add `serde` derive for serialisation
- [ ] Unit tests: construct valid instances, verify field defaults

### Phase 1: Knot File Format and Parser
- [ ] Propose frontmatter format (see above) — codify in parser
- [ ] Implement `KnotFileParser` that reads a `.md` file with YAML frontmatter
- [ ] Parse frontmatter into `Knot` struct
- [ ] Handle parse errors with descriptive error types
- [ ] Unit tests: parse valid knot file, parse file with missing fields returns error, parse file with malformed YAML returns error

### Phase 2: Workspace Agent Config
- [ ] Define workspace-level agent config struct (CLI path + args)
- [ ] Default CLI path to `"pi"`, default args to empty
- [ ] Unit tests: default values, custom CLI path, custom args

### Phase 3: Loom Model
- [ ] `Loom` struct with source directory, tie-off point, list of knots
- [ ] Validation: source dir and tie-off point are non-empty paths
- [ ] Unit tests: valid loom, loom with missing tie-off point fails validation

## Notes
