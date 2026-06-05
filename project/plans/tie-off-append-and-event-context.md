# Plan: Tie-Off Append and Event Context

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan fulfils the PRD goals:
- *"When a strand is processed (create/modify/delete), the tie-off file records the full event history — each agent response is appended as a new section with metadata (event type, strand path, timestamp) separated by `---` delimiters."*
- Story 6: *"As a user, I want the tie-off document to tell the complete story of what has happened to a strand."*

## Problem

The current tie-off model overwrites the output file on every strand event. This means:

1. **Delete events skip the agent** — `ProcessStrand` detects a delete event and writes a static tombstone string (`"Strand deleted: <path>"`) instead of invoking the agent. The PRD says the agent should be triggered and produce a response reporting what changed/undone.
2. **Tie-off history is lost** — Each event overwrites the previous tie-off. A user reading the output file sees only the latest response, with no record of what happened before.
3. **Agent has no event context** — The agent receives strand content and prompt template but no information about *what kind of event* triggered the processing (create/modify/delete). It cannot make different decisions based on event type.

The root cause: `TieOffSink::write()` uses `fs::write()` (overwrite), `ProcessStrand` short-circuits delete events, and the agent prompt has no event metadata.

## Target

- **Tie-off append mode** — `TieOffSink` appends to existing tie-off files with `---` section delimiters and metadata headers (`event`, `strand`, `timestamp`).
- **Event context in agent prompt** — `ProcessStrand` passes event type, strand path, and previous tie-off content to the agent via the prompt context.
- **Delete events trigger the agent** — `ProcessStrand` no longer short-circuits delete events. The agent receives context about the deletion and produces a response that is appended to the tie-off.
- **Tie-off document tells the story** — Reading the output file shows the complete chronological history of all events and agent responses.

## Phases

### Phase 0: Tie-Off Append Mode with Section Separators

**Failing tests created:** `adapters::outbound::tie_off_sink::tests::append_mode_creates_file`, `adapters::outbound::tie_off_sink::tests::append_mode_adds_section`, `adapters::outbound::tie_off_sink::tests::append_mode_preserves_history`

- [ ] Failing test: `adapters::outbound::tie_off_sink::tests::append_mode_creates_file` — first append creates the file with header section
- [ ] Failing test: `adapters::outbound::tie_off_sink::tests::append_mode_adds_section` — second append adds `---` delimiter and new section
- [ ] Failing test: `adapters::outbound::tie_off_sink::tests::append_mode_preserves_history` — three appends produce three sections, all readable
- [ ] Add `append_mode: bool` to `TieOff` or `TieOffSink::append()` method
- [ ] Implement `append()` in `FileSystemTieOffSink`: read existing content, add `---` delimiter, write metadata header + new content
- [ ] Metadata header format:
  ```markdown
  ---
  ## Event: Created | Modified | Deleted
  ## Strand: /path/to/strand.md
  ## Timestamp: 2026-06-05T14:00:00Z
  ---
  ```
- [ ] If file does not exist, create with header section (no leading `---`)
- [ ] Update `ProcessStrand` to call `append()` instead of `write()`
- [ ] Update existing tests that construct `TieOff` with `write()` calls

### Phase 1: Event Context in Agent Prompt

**Failing tests created:** `application::usecases::tests::process_strand_passes_event_context`, `adapters::subprocess::tests::runner_passes_event_metadata`

- [ ] Failing test: `application::usecases::tests::process_strand_passes_event_context` — `ProcessStrand` builds agent context with event type, strand path, previous tie-off content
- [ ] Failing test: `adapters::subprocess::tests::runner_passes_event_metadata` — subprocess runner receives and forwards event metadata to agent CLI
- [ ] Add event context fields to agent input: `event_type`, `strand_path`, `previous_tie_off` (if exists)
- [ ] `ProcessStrand::execute()` reads existing tie-off content before calling agent (if file exists)
- [ ] Agent prompt includes event context section (e.g. in system prompt or as context block)
- [ ] For delete events: previous strand content is not available, so pass strand path and previous tie-off content
- [ ] Update agent runner interface to accept event context
- [ ] Update existing tests that construct agent execution context

### Phase 2: Delete Events Trigger the Agent

**Failing tests created:** `application::usecases::tests::process_strand_delete_triggers_agent`, `integration::tests::delete_strand_agent_produces_tie_off`

- [ ] Failing test: `application::usecases::tests::process_strand_delete_triggers_agent` — delete event triggers agent (mock runner called), response appended to tie-off
- [ ] Failing test: `integration::tests::delete_strand_agent_produces_tie_off` — delete strand file → tie-off has new section with agent response about deletion
- [ ] Remove short-circuit in `ProcessStrand::execute()` for delete events
- [ ] Delete events still pass agent context (event type, strand path, previous tie-off)
- [ ] Agent response appended to tie-off (not overwriting)
- [ ] Loom-log still records `KnotProcessing`, `KnotCompleted`/`KnotFailed`, `StrandProcessed` for delete events
- [ ] Update existing tests that expect delete events to skip agent

### Phase 3: Integration Test — Full Lifecycle

**Failing tests created:** `integration::tests::full_tie_off_history`, `integration::tests::tie_off_sections_readable`

- [ ] Failing test: `integration::tests::full_tie_off_history` — create strand → modify strand → delete strand → tie-off has 3 sections with correct headers
- [ ] Failing test: `integration::tests::tie_off_sections_readable` — parse tie-off markdown sections, verify each has event type, strand path, timestamp
- [ ] Tests use mock agent CLI that returns different content per event type
- [ ] Verify tie-off file is valid markdown with `---` delimiters
- [ ] Verify sections are in chronological order
- [ ] Compile and verify no errors

## Notes

- **Markdown section format** — The `---` delimiter is standard markdown horizontal rule. Combined with `## Event:` headers, the tie-off file becomes a readable document that tools (or users) can parse.
- **Previous tie-off content** — For create events, there is no previous tie-off. For modify/delete events, the previous tie-off content is read and passed to the agent as context. This allows the agent to reference earlier decisions.
- **Delete event content** — When a strand is deleted, its content is not available (the file is gone). The agent receives the strand path, event type, and previous tie-off content. The agent can assess what was removed and what remains.
- **Content diff** — How the agent sees the *difference* between the old and new strand content is deferred to a future plan. For now, the agent receives the current content (or previous tie-off for delete) and makes its best assessment.
- **Performance** — Reading the entire tie-off file on each event could be slow for very large files. This is acceptable for now — tie-off files are agent responses, not megabyte-scale artifacts.
