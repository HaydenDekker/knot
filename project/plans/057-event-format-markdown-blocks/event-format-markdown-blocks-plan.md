# Plan: Agent Event Format — Markdown Code Blocks with Frontmatter

## Problem

The current agent event parsing in tie-off files is fragile. Events are emitted as indented key-value blocks or ```-delimited code blocks, and the parser relies on whitespace detection and a global code-block toggle state. Key weaknesses:

- **Indentation is the only structural signal** — if the agent forgets to indent, events are silently dropped
- **Block boundaries are fragile** — blank lines terminate blocks, so interjected prose breaks event parsing
- **Non-indented `event:` lines are dropped** — agents that deviate from the format lose events with no feedback
- **No event body** — the current model captures only `event_id` + `HashMap<String, String>` payload. Agents have no way to attach freeform context to an event
- **``` language tags break code block mode** — `trimmed == "```"` is an exact match, so ```yaml or ```json are silently rejected

The current model works well enough for key-value data, but it cannot carry narrative context and it breaks on common formatting variations.

## Target

Events are emitted as individual ```markdown code blocks, each containing YAML-style frontmatter (the structured event data) followed by a plain-text body (freeform event context):

```markdown
```markdown
---
event: PlanCreated
plan: PLAN-001
description: Implementation plan for event routing
timestamp: "2026-07-11T14:30:00Z"
---

The plan covers three phases: planning, review, and approval.
Each phase is tracked in the project plans directory.
```

Multiple events are emitted as separate code blocks in the tie-off:

```markdown
```markdown
---
event: PlanCreated
plan: PLAN-001
description: Implementation plan created
---

Plan created with initial scope.
```

```markdown
---
event: ScopeChanged
plan: PLAN-001
description: Phase 2 removed from scope
---

Phase 2 was removed after stakeholder review.
```
```

Changes required:

1. **Parser (`tieoff_parser.rs`)** — replace the indented-block parser with a code-block parser that extracts ```markdown blocks, splits frontmatter from body, and returns `AgentEvent` structs with a new `body` field. The old indented-block and plain-```-block formats are removed.
2. **`AgentEvent` struct (`events.rs`)** — add an optional `body: Option<String>` field to carry the freeform event context from the body.
3. **`build_listener_context` (`events.rs`)** — update the injected prompt to instruct agents to use the new format.
4. **`EventDispatcherPort` / `FileSystemEventDispatcher`** — update the dispatched event file to include the event body (the file already has frontmatter + body structure, so this is wiring the `body` through).

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `tieoff_parser.rs::extract_agent_events_*` (20+ tests) | Indented block parsing, code block parsing, `event: None`, multiple events, edge cases | ✅ Green — tests the old format |
| `events.rs::build_listener_context_*` (10+ tests) | Context injection prompt generation, deduplication, event descriptions | ✅ Green — tests the prompt injected into producers |
| `event_dispatcher.rs::tests` (7 tests) | File dispatch, path creation, file content structure, fan-out | ✅ Green — tests the consumer-side file creation |
| `events.rs::AgentEvent` tests (5 tests) | Construction, serialisation, empty payload | ✅ Green — tests the event struct |

All existing tests validate the **current** (indented/``` code block) format. They will need to be rewritten or replaced for the new format.

## Test Gaps

- No test for the event body being preserved through dispatch
- No test for the frontmatter/body split inside code blocks
- No test for multiple ```markdown blocks in the same tie-off
- No test for malformed frontmatter (e.g. `---` without closing `---`)
- No test for the new prompt format in `build_listener_context`

## Phases

### Phase 0: Add `body` field to `AgentEvent` and update serialisation

Add `body: Option<String>` to `AgentEvent` in `events.rs`. Update the struct's serialisation tests. This is a pure domain-layer change with no parser or prompt changes.

**Tests:**
- Existing `AgentEvent` serialisation tests (update to include `body`)
- New test: `AgentEvent` with body round-trips through JSON serialisation
- New test: `AgentEvent` with `None` body survives serialisation

### Phase 1: Write failing tests for the new parser format

Write the parser tests **before** implementing the parser. These tests define the target format:

- Parse ```markdown blocks with frontmatter + body
- Parse multiple events in separate ```markdown blocks
- Reject blocks that are not ```markdown (other language tags ignored)
- Handle `event: None` inside a ```markdown block
- Handle missing closing `---` in frontmatter gracefully
- Handle empty body
- Handle body with blank lines, headers, lists

**Tests:** All new tests in `tieoff_parser.rs` — the old tests for indented/``` blocks will be replaced.

### Phase 2: Implement the new parser

Replace `extract_agent_events` with a parser that:

1. Scans for ```markdown opening fences
2. Reads lines until the closing ``` fence
3. Splits the block content on `---` to separate frontmatter from body
4. Parses frontmatter as key-value pairs (same `parse_kv_line` helper)
5. The first key must be `event:` — if absent, the block is skipped
6. All other frontmatter keys go into the payload
7. Everything after the closing `---` of frontmatter becomes the `body`
8. `event: None` produces no event (same as current)

The old indented-block and plain-```-block paths are removed.

**Tests:** All tests from Phase 1 should pass. Existing indented/``` block tests are deleted.

### Phase 3: Update `build_listener_context` prompt

Update the injected context in `build_listener_context` (`events.rs`) to instruct agents to emit ```markdown blocks with frontmatter + body instead of indented key-value pairs. The prompt example should show the new format with multiple events.

**Tests:**
- Existing `build_listener_context` tests updated for the new format
- New test: prompt contains ```markdown fence example
- New test: prompt shows frontmatter + body structure
- New test: prompt shows `event: None` in the new format

### Phase 4: Wire event body through the dispatcher

Update `FileSystemEventDispatcher` to include the event body when writing the dispatched event file. The file format already has a frontmatter + body structure (`build_event_file_content`), so this is wiring the `body` field into the output. If an event has no body, the existing "No payload data." or bullet-list body is used.

**Tests:**
- Existing dispatcher tests updated (the file content structure test checks the body)
- New test: dispatched file includes event body in markdown content
- New test: dispatched file without event body uses fallback content

## Notes

- The ```markdown language tag is chosen because it signals to markdown renderers that the block contains markdown content. This also avoids ambiguity with the current plain-``` format.
- The frontmatter uses the same `key: value` line format as the current parser — `parse_kv_line` is reused.
- The `---` delimiter for frontmatter is standard Markdown/YAML frontmatter convention.
- Backward compatibility: existing tie-off files with the old format will have their events silently ignored. This is acceptable because events are consumed immediately at dispatch time — old tie-offs are never re-parsed.
- The `event: None` signal remains the same conceptually (no events emitted) but changes format to live inside a ```markdown block with frontmatter.
