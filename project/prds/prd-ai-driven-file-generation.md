# PRD: AI-Driven File Generation from Loom Events

## Problem

Developers work in a source workspace with raw input files (drafts, notes, data, code sketches) that need to be transformed into structured output files in a target directory. Currently, this transformation requires manual effort — the developer must read each input, understand the desired output, and manually trigger an LLM to produce it. When the same agent profile and prompt template must be applied across many files — for example, reviewing every PRD dropped into a folder, or generating docs from every spec file — the developer ends up repeating the same manual loop over and over. This becomes tedious and error-prone as the file count grows.

Knot requires a mechanism to **react to file system events in a watched workspace and automatically generate output files using AI**. The goal is to remove the need for the user to repeatedly control the agent: instead, the agent runs as a continuous workflow triggered by file events. Drop files into a watched folder and the configured output appears in a target directory — no manual invocation required.

## Goals

> **Glossary**
>
> - **Knot** — A configured artifact composed of two parts:
>   1. **Agent Profile** — The configuration that determines *which agent* runs: the LLM provider to use, the skills available to the agent (driving the prompt), the tools available to the agent (executing the prompt), and the agent's initial system prompt.
>   2. **Prompt Template** — The *how* of file processing: the goal description and the rules for how the input file(s) should be bundled into the prompt sent to the agent.
>   The knot is where it all comes together, ready for processing.
>
> - **Loom** — A directory containing one or more knot definition files. The directory structure inside the loom is up to the user (e.g. `prd-review-loom/goals-review-knot.md`, `prd-review-loom/non-goals-review-knot.md`). Each knot file defines a knot. A loom watches a configured source directory for **strands** — file events on strands trigger the loom's knots.
>
> - **Tie-off** — The final response or error produced by a knot at the end of its session. This is the single output that lands in the configured **tie-off point** (e.g. a summary, a list of documents, an error report). During processing the knot's agent may write files to any directories it has access to — the tie-off is what gets captured and filed away.
>
> - **Tie-off Point** — The directory location where tie-offs land. Configured per loom.
>
> - **Strand** — A file in the source directory that a loom watches for. When a strand is created, modified, or deleted, the loom's knots are triggered to process it.
>
> - **Loom-log** — A file that holds a loom's activity log. Records which knots are detected and registered, and high-level loom events. Users check this to confirm a loom is configured correctly.
>
> - **Knot-state** — A per-knot file that records processing events and status for that knot. Contains event type, strand path, tie-off path, and any errors. All knot-level HTTP status is sourced from this file.

- [ ] Users can configure one or more **looms**, each being a directory containing one or more knot definition files.
- [ ] When a file is **created, modified, or deleted** in a watched source directory, Knot triggers the relevant loom knot(s) with the file content — no manual user invocation required.
- [ ] The agent works in any directories it is configured to utilise, and at the end of its session produces a **tie-off** (final response or error) that is written to the configured target directory with the target name.
- [ ] Users can define, update, and remove looms and knots programmatically via Knot's HTTP interface **or manually via the file system** — without restarting the service.
- [ ] The file generation pipeline is observable via Knot's HTTP interface — users can see which events fired, what tie-offs were produced, and any errors.

## Non-Goals

- Real-time collaborative editing or live preview of generated files.
- Support for arbitrary CI/CD pipelines or cloud deployment targets.
- Fine-grained per-file AI model or prompt configuration (one knot per loom is sufficient for this feature).
- Conflict resolution when multiple events overlap on the same file.
- Support for non-file data sources (e.g., databases, APIs) as inputs.

## User Stories

### Story 1: Create a Knot and Confirm It Is Active

As a developer, I want to create a knot in a loom and confirm it is active, so that I know the loom is ready to process strands.

**Scenarios:**

1. Given I have created a knot file in a loom directory, when I check the loom-log or query the HTTP interface, then I can see the knot is registered and active.

### Story 2: Watch a Loom and Generate Tie-offs

As a developer, I want a loom to watch a source directory for strands, so that when a strand is created, modified, or deleted, the knot processes it and produces a tie-off.

**Scenarios:**

1. Given I have a loom `./docs-drafts` with a knot "convert markdown drafts into published API documentation" and a tie-off point at `./docs-final`, when I take a strand from the monitored source directory, then a tie-off is produced in the tie-off point.
2. Given I have configured a loom and tie-off point, when I modify a strand in the source directory, then the tie-off in the tie-off point is updated by the knot based on the changes.
3. Given I have configured a loom and tie-off point, when I delete a strand from the source directory, then the knot is still triggered — it produces a tie-off reporting what it changed or undone. The previous tie-off is overwritten, never deleted.

### Story 3: Configure Multiple Looms

As a developer, I want to configure multiple looms with different knots, so that I can run different AI transformations in parallel without interference.

**Scenarios:**

1. Given I have configured two looms — one for "convert specs to tests" and one for "convert notes to release notes" — when I drop strands into each loom's source directory independently, then each tie-off point receives the correct tie-off for its knot.

### Story 4: Configure Agent Runtime

As a developer, I want to specify an agent configuration in my knot, so that Knot knows which CLI tool to invoke and with which arguments when processing strands.

**Scenarios:**

1. Given I have configured `agent-config` in a knot file with CLI arguments (e.g. `--no-tools`), when a strand triggers the knot, then Knot constructs and calls the agent CLI with the agent profile and prompt template from the knot config.

### Story 5: Observe Loom and Knot Status

As a developer, I want to check the status of my loom and individual knots, so that I know which knots are active and whether strands are being processed successfully.

**Scenarios:**

1. Given I have a configured loom, when I check the loom via the HTTP interface, then I see which knots are detected and registered.
2. Given a strand event has triggered a knot, when I check the knot via the HTTP interface, then I can see the knot's processing events — event type, strand path, tie-off path, and current status — sourced from the **knot-state** file on disk.
3. Given a knot generation failed for a strand event, when I check the knot-state or query the HTTP interface, then I can see the error details so I can diagnose the issue.

## Success Criteria

- [ ] A user can start Knot with a loom configuration and **strands** in the watched source directory trigger the loom's knots on create/modify/delete.
- [ ] Strand events trigger the knot's agent, producing a **tie-off** in the configured tie-off point within a reasonable time (under 30 seconds for typical files).
- [ ] A **loom-log** file records loom-level activity (knots detected, loom events) and is queryable via the HTTP interface.
- [ ] Each knot maintains a **knot-state** file recording its processing events and status, queryable via the HTTP interface.
- [ ] All HTTP-exposed state is sourced from the filesystem — the HTTP interface reflects loom-log and knot-state files.
- [ ] Multiple configured looms operate independently without cross-interference.

## Dependencies & Constraints

- **Technical constraint:** Knot is a local-first Rust application — all file watching and AI processing runs on the local machine.
- **Technical constraint:** Knot uses axum for its HTTP server; any new endpoints follow the existing routing pattern.
- **Technical dependency:** Knot uses the `notify` crate for file system watching.
- **External dependency:** Knot calls an external agent CLI to execute knots. Initially this is **pi** (`pi.dev` CLI). The user configures `agent-config` in the knot with CLI arguments (e.g. `--no-tools`), and Knot parses the knot config to construct the full CLI invocation (provider, model, skills, tools, system prompt).
- **Configuration constraint:** Knot is started with respect to its workspace directory and discovers looms and knots by scanning downward from there. No separate top-level config file is required. The scanning rule may be constrained further in future.

## Implementation Status: 🔵 Open
