# PRD: AI-Driven File Generation from Loom Events

## Problem

Developers work in a source rig with raw input files (drafts, notes, data, code sketches) that need to be transformed into structured output files in a target directory. Currently, this transformation requires manual effort — the developer must read each input, understand the desired output, and manually trigger an LLM to produce it. When the same agent profile and prompt template must be applied across many files — for example, reviewing every PRD dropped into a folder, or generating docs from every spec file — the developer ends up repeating the same manual loop over and over. This becomes tedious and error-prone as the file count grows.

Knot requires a mechanism to **react to file system events in a watched rig and automatically generate output files using AI**. The goal is to remove the need for the user to repeatedly control the agent: instead, the agent runs as a continuous workflow triggered by file events. Drop files into a watched folder and the configured output appears in a target directory — no manual invocation required.

## Goals

> **Glossary**
>
> - **Knot** — A configured artifact composed of three parts:
>   1. **Agent Profile** — The configuration that determines *which agent* runs: the LLM provider to use, the skills available to the agent (driving the prompt), the tools available to the agent (executing the prompt), and the agent's initial system prompt.
>   2. **Prompt Template** — The *how* of file processing: the goal description and the rules for how the input file(s) should be bundled into the prompt sent to the agent.
>   3. **Directories** — `strand_dir` (where strands to watch live, **required**). The tie-off output path is **statically derived** as `rig/output/{loom-id}/{knot-name}/output.md` — no `tie-off-dir` configuration is needed.
>   The knot is where it all comes together, ready for processing.
>
> - **Loom** — A directory inside the rig whose name ends with the `-loom` suffix (e.g. `rig/planning-loom/`). Contains one or more `.md` knot definition files at its first level. Knot auto-discovers looms by watching the rig directory: when a new `*-loom` directory is created it is immediately registered with file watchers and begins processing. Loom directories can also be created via the HTTP `POST /looms` endpoint, which writes the directory to disk and triggers the same registration flow. The loom directory is **static and derived from naming convention**; it is not user-configurable via the API beyond creation. The loom directory holds knot definitions — **not** strands. Strands live in each knot's `strand_dir`.
>
> - **Strand Directory** — The directory a knot watches for strand file events. Configured per-knot as `strand_dir` in the knot's YAML frontmatter. This is where raw input files (**strands**) live. Distinct from the loom directory.
>
> - **Tie-off** — The final response or error produced by a knot at the end of its session. Each processing event is appended to a single `output.md` file at `rig/output/{loom-id}/{knot-name}/output.md`, so the document grows over time and tells the complete story of the knot's work. The event metadata in each section identifies which strand was processed. During processing the knot's agent may write files to any directories it has access to — the tie-off is what gets captured and filed away.
>
> - **Tie-off Directory** — Statically derived path under `rig/output/{loom-id}/{knot-name}/`. No longer configurable per-knot (the `tie-off-dir` YAML field has been removed).
>
> - **Strand** — Any text file in a knot's strand directory. Strands can be source code (`.py`, `.rs`, `.js`), config files (`.json`, `.yaml`), plain text (`.txt`), or markdown (`.md`) — any text-based file. When a strand is created, modified, or deleted, the knot that watches that directory is triggered to process it. The strand is the raw input fed into a knot. Binary formats (images, PDFs, archives) are not expected inputs — if a binary file appears in a strand directory it is skipped and a warning is logged.
>
> - **Loom-log** — A file that holds a loom's activity log. Lives at `<rig>/output/<loom-id>/.loom-log` — outside the loom directory itself, keeping the rig clean of non-loom directories. Records which knots are detected and registered, and all loom and knot events for that loom. Users check this to confirm a loom is configured correctly and to trace processing history.
>
> - **Knot-state** — A per-knot file that records processing events and status for that knot. Contains event type, strand path, tie-off path, and any errors. All knot-level HTTP status is sourced from this file.

- [x] Users can configure one or more **looms**, each a `*-loom` directory inside the rig containing one or more knot definition files at its first level. (*Plans 1–5*)
- [x] When a file is **created, modified, or deleted** in a knot's strand directory, Knot triggers the relevant knot(s) with the file content — no manual user invocation required. (*Plans 2, 3, 5*)
- [x] The agent works in any directories it is configured to utilise, and at the end of its session produces a **tie-off** (final response or error) that is written to the configured target directory with the target name. (*Plans 3, 5, 6, 7*)
- [ ] When a strand is processed (create/modify/delete), the tie-off file records the full event history — each agent response is appended as a new section with metadata (event type, strand path, timestamp) separated by `---` delimiters, so the output document tells the complete story of what has happened. (*Plan 12*)
- [ ] Users can define, update, and remove looms and knots programmatically via Knot's HTTP interface **or manually via the file system** — without restarting the service. Looms created as directories on disk (or via `POST /looms`) are auto-discovered at runtime: a rig directory watcher detects new `*-loom` directories and registers them with file watchers immediately. Knots defined as `.md` files inside a loom directory are auto-discovered: new `.md` files are parsed and the knot is registered for processing, edited `.md` files update the in-memory knot config, and deleted `.md` files deregister the knot. The HTTP interface (`GET /looms`, `GET /looms/{id}/knots`) reflects the current in-memory state, which is always in sync with the filesystem. (*Planned*)
- [x] The file generation pipeline is observable via Knot's HTTP interface — users can see which events fired, what tie-offs were produced, and any errors. (*Plans 2, 4*)
- [ ] When a knot's `strand_dir` does not exist at registration time, Knot creates the directory automatically so the knot can begin watching immediately. The creation is recorded in the loom-log so the user knows the directory was provisioned by Knot rather than pre-existing. This allows users to define knots pointing to directories they intend to populate later — no manual directory creation is required.
- [ ] Users can watch a parent directory and invoke a knot for every child event within that directory, so that the knot can make a judgement call with respect to the whole (e.g. a new plan subdirectory triggers processing for each file it contains).
- [ ] Users can define agent profiles as shared, named entities outside of any single knot, so that multiple knots reference the same profile and the LLM target can be changed dynamically from one place.
- [ ] Users can switch between multiple rigs on the same project, so that they can stop one rig and start another without losing loom definitions or needing to reconfigure.
- [ ] Users can share a rig with colleagues by packaging its looms — the rig's portable unit is its loom definitions; tie-offs and logs are derived state that regenerate on the recipient's machine.

## Non-Goals

- Real-time collaborative editing or live preview of generated files.
- Support for arbitrary CI/CD pipelines or cloud deployment targets.
- Conflict resolution when multiple events overlap on the same file.
- Support for non-file data sources (e.g., databases, APIs) as inputs.
- Rig sharing as a sync/merge service — sharing is a one-way handoff of loom definitions; the recipient owns their own state.
- Binary file processing — Knot accepts text files only (source code, config, plain text, markdown). Binary formats (images, PDFs, archives) are skipped with a warning logged to the loom-log.

## User Stories

### Story 1: Create a Knot and Confirm It Is Active

As a developer, I want to create a knot in a loom and confirm it is active, so that I know the loom is ready to process strands.

**Scenarios:**

1. Given I have created a knot file in a loom directory, when I check the loom-log or query the HTTP interface, then I can see the knot is registered and active.
2. Given I have created a knot file with a `strand_dir` that does not yet exist on disk, when the knot is registered, then Knot creates the directory automatically and I can see in the loom-log that the directory was created by Knot, and the knot is registered and active.

### Story 2: Watch a Loom and Generate Tie-offs

As a developer, I want a loom to watch a source directory for strands, so that when a strand is created, modified, or deleted, the knot processes it and produces a tie-off.

**Scenarios:**

1. Given I have a loom `./docs-drafts` with a knot "convert markdown drafts into published API documentation" and a tie-off point at `./docs-final`, when I take a strand from the monitored source directory, then a tie-off is produced in the tie-off point.
2. Given I have configured a loom and tie-off point, when I modify a strand in the source directory, then the tie-off in the tie-off point is updated by the knot based on the changes.
3. Given I have configured a loom and tie-off point, when I delete a strand from the source directory, then the knot is still triggered — the agent is invoked with the event context (event type, strand path, previous content if available) and its response is appended to the existing tie-off as a new section, separated by `---`. The tie-off file grows over time to tell the complete story of the strand's lifecycle.

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

### Story 6: Understand Processing History from the Tie-off

As a user, I want the tie-off document to tell the complete story of what has happened to a strand — what the agent has done, what changes were made, and when files were deleted — so that I can understand the full processing history by reading the output file.

**Scenarios:**

1. Given a strand has been processed multiple times (created, modified, deleted, etc.), when I open the tie-off document, then I see a chronological record of each event — each entry has a header with event type, strand path, and timestamp, followed by the agent's response for that event, separated by `---` delimiters.
2. Given a strand was deleted, when I read the tie-off document, then I see the deletion event appended as a new section with the agent's assessment of what was removed and what remains.
3. Given I am reviewing a tie-off document, when I see the `---` section separators, then I can quickly scan the history of what has happened to the strand without losing context from earlier events.

### Story 7: Parent Directory Strand — Child Event Fan-Out

As a user, I want to take a strand of a parent directory to invoke a knot for every child event, so that the knot can make a judgement call with respect to the whole. For example, I want to listen for a new plan in a subdirectory where the plan title (and therefore subdirectory folder) may not have been created yet. This allows breaking down a plan into further smaller chunks.

**Scenarios:**

1. Given I have a knot watching a parent directory (e.g. `plans/`), when a new subdirectory is created (e.g. `plans/new-feature-plan/`), then the knot is triggered once with the directory as the strand context, and the agent can inspect all files within that directory to make a holistic judgement
2. Given a subdirectory contains multiple `.md` files, when the parent strand event fires, then each child file is available to the agent as part of the strand context so the agent can consider them together
3. Given the subdirectory name is not known in advance (e.g. derived from the plan title), when the directory is created, then the knot still triggers on it — the strand is the directory itself, not a pre-known file
4. Given the agent produces output that creates further subdirectories, when those child directories are created, then the knot can recursively process them if configured (respecting k2k iteration limits from the System Reliability PRD)

### Story 8: Shared Agent Profiles

As a user, I want to swap out a model to re-run a phase or undertake some work on future changes. I need to create an agent profile and then use the agent profile in the knot. That way multiple knots could use the same agent profile and I could set the profile LLM targets dynamically.

**Scenarios:**

1. Given I have defined an agent profile (provider, model, skills, tools, system prompt) as a shared resource, when I reference that profile from multiple knot definitions, then each knot uses the profile's configuration at processing time
2. Given a shared agent profile is used by three knots, when I update the profile's model (e.g. from `gpt-4o` to `claude-sonnet`), then the next strand event processed by any of the three knots uses the updated model
3. Given a strand was processed with model A and produced an unsatisfactory tie-off, when I update the shared agent profile to model B and replay the event, then the replay uses model B — I don't need to edit the knot definition itself
4. Given I have two shared profiles — `fast-profile` (gpt-4o, 60s timeout) and `thorough-profile` (claude-sonnet, 600s timeout) — when I switch a knot's profile reference from one to the other, then future processing uses the new profile's provider, model, and timeout

### Story 9: Rig Switching and Sharing

As a user, I want to switch between multiple rigs on the same project, so that I can stop one rig and start another — for example, switching from a development rig to a review rig, or from one team's configuration to another's. As a rig matures, I want to share it with colleagues so they can benefit from the loom definitions we've built.

The rig's portable unit is its loom definitions. Tie-offs and logs are derived state — they are removed when sharing, and regenerate when the recipient starts the rig. This means a rig is essentially its looms and profiles.

**Scenarios:**

1. Given I have two rigs — `rig-dev` and `rig-review` — when I stop Knot and restart it pointing at the other rig, then Knot loads the new rig's looms and begins processing against its configurations
2. Given a rig has been running and has accumulated tie-offs and loom-logs, when I share the rig with a colleague, then only the loom definitions and profiles are packaged — tie-offs and logs are excluded as derived state
3. Given I receive a shared rig from a colleague, when I start Knot with that rig and drop strands into its watched directories, then tie-offs and logs regenerate on my machine using my own agent profiles and LLM providers
4. Given I have a mature rig I want to share, when I request the rig be packaged, then Knot produces a distributable artifact containing the rig's looms, knot definitions, and profiles — nothing else

## Non-Goals

## Success Criteria

- [x] A user can start Knot with a loom configuration and **strands** in the watched source directory trigger the loom's knots on create/modify/delete.
- [x] Strand events trigger the knot's agent, producing a **tie-off** in the configured tie-off point within a reasonable time (under 30 seconds for typical files).
- [ ] Strands can be any text file (`.py`, `.rs`, `.js`, `.json`, `.yaml`, `.txt`, `.md`, etc.) — not limited to `.md`. Binary files are skipped with a warning logged to the loom-log and stderr.
- [x] A **loom-log** file records loom-level activity (knots detected, loom events) and is queryable via the HTTP interface.
- [x] Each knot maintains a **knot-state** file recording its processing events and status, queryable via the HTTP interface.
- [x] All HTTP-exposed state is sourced from the filesystem — the HTTP interface reflects loom-log and knot-state files.
- [ ] Knot discovers the rig directory automatically: `./rig/` in the current working directory. If it doesn't exist, Knot creates it on first run.
- [x] Multiple configured looms operate independently without cross-interference.
- [ ] Tie-off files append new agent responses as `---`-separated sections with event metadata headers, preserving the full processing history (*Plan 12*).
- [ ] A knot can watch a parent directory and be triggered by subdirectory creation, receiving child file context for holistic processing
- [ ] Agent profiles are shareable, named entities that multiple knots can reference
- [ ] Updating a shared agent profile's LLM target is reflected in all knots that reference it on their next invocation
- [ ] A user can start Knot with a named rig (not just the default `rig/` directory), enabling switching between rigs on the same project
- [ ] A rig can be packaged as a distributable artifact containing looms and profiles, excluding tie-offs and logs

## Dependencies & Constraints

- **Technical constraint:** Knot is a local-first Rust application — all file watching and AI processing runs on the local machine.
- **Technical constraint:** Knot uses axum for its HTTP server; any new endpoints follow the existing routing pattern.
- **Technical dependency:** Knot uses the `notify` crate for file system watching.
- **External dependency:** Knot calls an external agent CLI to execute knots. Initially this is **pi** (`pi.dev` CLI). The user configures `agent-config` in the knot with CLI arguments (e.g. `--no-tools`), and Knot parses the knot config to construct the full CLI invocation (provider, model, skills, tools, system prompt).
- **Configuration constraint:** Knot is started with respect to its rig directory. Looms and knots are discovered by watching the rig and loom directories for filesystem events (directory creation, `.md` file creation/modification/deletion), not by periodic scanning. No separate top-level config file is required.
- **Technical constraint:** Parent directory strand processing requires directory creation events to trigger knots (currently filtered out). The `map_strand_event` method in `NotifyEventSource` skips directory events — a new event type or watch mode is needed.
- **Technical constraint:** Shared agent profiles require a new domain entity and a reference mechanism in the knot definition (e.g. `agent-profile-ref` instead of inline `agent-config`). Profile resolution at processing time replaces the current inline config lookup.
- **Design decision:** A rig is portable — its durable state is loom definitions and agent profiles. Tie-offs and logs are derived state that regenerate on any machine. Sharing a rig means sharing only its looms and profiles.
- **Design decision:** Rig sharing is a one-way handoff. No sync, merge, or conflict resolution between shared rigs and recipient rigs is required.

## Implementation Status: ✅ Complete (2026-06-04)

All 7 plans contributing to this PRD are complete.

### Status Note — 2026-06-04

All goals were achieved except rig directory scoping (see Plan 11). Seven plans were executed across the domain, application, outbound adapter, inbound adapter, and composition root layers:

1. **Knot Domain Models** (Plan 1) — Entities, value objects, events, knot file parser.
2. **Application Layer — Ports and Use Cases** (Plan 2) — Port traits, use cases, debounce engine, processing state machine.
3. **Outbound Adapters** (Plan 3) — Filesystem IO, `notify` watching, subprocess execution, tie-off writing.
4. **Loom HTTP Interface** (Plan 4) — Axum handlers and routes for all loom/knot operations.
5. **System Integration and Wiring** (Plan 5) — Composition root, event pipeline, end-to-end integration tests, graceful shutdown.
6. **Loom Config, Path Resolution and Agent Error Logging** (Plan 6) — `.loom-config.yaml` for external source/tie-off directories, canonical path resolution, agent error visibility in knot-state and loom-log.
7. **pi Agent Integration** (Plan 7) — `AgentConfig` extended with provider/model/tools; `pi` CLI invocation constructed from knot config; strand content passed to agent.

**Exceptions:**

- **Real `pi` integration test skipped** — Plan 7 Phase 3 used a stub script (`stub-pi.sh`) instead of calling the real `pi` CLI, as the integration test does not require an API key. A real `pi` call would need provider credentials and is not automated in CI. This is noted in Plan 7 as Option A (unselected).
- **No real LLM calls in test suite** — All integration tests use mock or stub agents. The tie-off content reflects whatever the stub returns, not actual LLM output. End-to-end verification with a real LLM is manual.
- **Debounce window fixed at 100ms** — The `DebounceEngine` uses a hardcoded 100ms window. Not yet configurable via loom config.
- **`RigAgentConfig` loaded from defaults only** — The PRD envisions rig-level config discovery; currently `main.rs` loads defaults (`cli_path = "pi"`, `cli_args = []`). Config file loading is not yet implemented.
