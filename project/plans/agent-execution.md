# Plan: Agent Execution and Tie-off Generation

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan implements the core processing pipeline: receiving strand events, invoking the agent CLI (pi), capturing output, and writing tie-offs. It addresses Story 2 (generate tie-offs) and Story 4 (configure agent runtime).

## Problem

Knot can watch for strand events, but it cannot yet process them. There is no mechanism to take a strand event + knot configuration, invoke the agent CLI, and produce a tie-off. Without this, the entire file generation pipeline is incomplete — events are detected but nothing happens.

## Target

- A processing pipeline consumes `StrandEvent`s from the watcher channel.
- For each event, the pipeline constructs the agent CLI command using the workspace agent config and knot configuration.
- The CLI is invoked as a subprocess with the strand content and knot prompt template.
- The CLI output (final response) is captured and written as a tie-off to the configured tie-off point.
- Knot-state file is updated with processing status (processing → completed or processing → failed).
- Loom-log records the strand processing event.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |

## Test Gaps

- No tests for CLI command construction (args, working directory, stdin).
- No tests for subprocess invocation and output capture.
- No tests for tie-off writing to the target directory.
- No tests for knot-state status transitions during processing.
- No tests for error handling (CLI not found, CLI returns error, timeout).
- No integration test: strand event → CLI invoked → tie-off written.

## Phases

### Phase 0: CLI Command Builder
- [ ] Implement `CliCommandBuilder` that constructs the full CLI command from:
  - Workspace agent config (`cli_path`, `cli_args`)
  - Knot configuration (goal, prompt template instructions)
  - Strand path and content
- [ ] The builder assembles the command: `<cli_path> <cli_args> --prompt "<goal>\n<instructions>" <strand_path>`
- [ ] Unit tests: build command with defaults (pi), build command with custom args, build command with custom cli_path, verify strand path is included

### Phase 1: Subprocess Executor
- [ ] Implement `AgentExecutor` that runs the CLI command as a `tokio::process::Command`
- [ ] Capture stdout and stderr
- [ ] Set a timeout (e.g. 120 seconds) — if exceeded, treat as failure
- [ ] On success, return the captured stdout as the tie-off content
- [ ] On failure, return error details (exit code, stderr)
- [ ] Unit tests: execute a known command, capture output, handle timeout, handle command not found

### Phase 2: Tie-off Writing
- [ ] Implement `TieOffWriter` that writes tie-off content to the tie-off point directory
- [ ] Tie-off filename derived from strand filename (e.g. `strand.md` → `<tie-off-point>/strand.tie-off.md`)
- [ ] Handle all event types: Created → write tie-off, Modified → overwrite existing tie-off, Deleted → write tie-off reporting what was changed/undone (never delete previous tie-off)
- [ ] Unit tests: write tie-off for created strand, overwrite tie-off for modified strand, write error tie-off for deleted strand, tie-off filename mapping

### Phase 3: Processing Pipeline
- [ ] Implement `ProcessingPipeline` that ties it all together:
  1. Receive `StrandEvent` from channel
  2. Update knot-state to `processing`
  3. Read strand file content
  4. Build CLI command using `CliCommandBuilder`
  5. Execute via `AgentExecutor`
  6. Write tie-off via `TieOffWriter`
  7. Update knot-state to `completed` or `failed`
  8. Append to loom-log
- [ ] Handle errors at each stage — write error tie-off, update knot-state with error details
- [ ] Integration test: given a loom with a knot, a strand event triggers the full pipeline, knot-state transitions through processing → completed, tie-off file is created

## Notes
