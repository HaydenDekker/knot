# Plan: Tie-Off Context Extraction for Agent Processing

## Related PRD

This plan contributes to [AI-Driven File Generation](../prds/prd-ai-driven-file-generation.md).

The PRD specifies that delete events should invoke the agent with "event context (event type, strand path, previous content if available)". The design document for Plan 12 (Tie-Off Append and Event Context) defined `previous_tie_off` in `ExecutionContext`, but Plan 30 (Context Management) removed it because full tie-off files can be large. This plan re-introduces a scoped version: extract only the last N entries relevant to the specific strand being processed, keeping context bounded.

## Problem

When a strand is deleted, `ProcessStrand::execute()` passes `@{strand_path}` as a CLI arg to `pi`, which tries to read a file that no longer exists. The agent receives no information about what the file contained or what previous processing has done with it.

The PRD envisions "previous content if available" for deletion events, but the full tie-off file can contain entries for many strands and grow large — injecting it wholesale blows up the prompt context.

## Target

A bounded context extraction mechanism that:
1. Parses the tie-off file into individual sections
2. Filters sections by strand path
3. Returns the last N entries (default: 5) for the specific strand
4. Injects this scoped history into the agent prompt for deleted events
5. Skips the `@file` reference for deleted events (file is gone)
6. Injects a deletion notice so the agent knows the file was removed

When a strand is deleted, the agent receives: a deletion notice, the strand path, and the last 5 processing entries for that strand from the tie-off file. This gives the agent enough context to reason about downstream references without context overflow.

## Implementation Status: ✅ Complete (2026-06-22)

## Notes
- Phase 0: Created `src/domain/tieoff_parser.rs` with `TieOffSection`, `parse_sections()`, `extract_last_n()` + 9 unit tests
- Phase 1: Integrated parser into `ProcessStrand::execute()` — Deleted events get scoped history (last 5 entries), deletion notice, `@file` skipped. 5 new unit tests
- Phase 2: 3 e2e integration tests in `tests/pipeline.rs`. Fixed path-mismatch bug discovered during testing
- Phase 3: Already covered by Phase 1's implementation — verified with existing tests
- Full test suite passes (366 tests, 0 failures)
- Version bumped to 0.16.0

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `tieoff_sink::tests::append_mode_*` | Append mode creates files, adds sections, preserves history with `---` delimiters | ✅ Unit — verifies write format |
| `pipeline::tests::pipeline_handles_strand_delete` | Delete events trigger processing, StrandProcessed appears in loom-log | ✅ Integration — no context extraction tested |
| `subprocess::tests::runner_passes_event_metadata` | Event metadata (trigger line) passed via stdin | ✅ Unit |

## Test Gaps

- No tie-off section parser exists (read-only access to tie-off files)
- No test for extracting per-strand history from a multi-strand tie-off file
- No test for deletion events receiving context instead of `@file`
- No test verifying the agent prompt contains deletion notice + scoped history

## Phases

### Phase 0: Tie-Off Section Parser (Domain Layer)

Create a pure function that parses a tie-off file into structured sections.

**File:** `src/domain/tieoff_parser.rs` (new)

**Design:**
- The tie-off format uses `---` as section delimiters
- Each section has a header line: `## {knot_name} triggered by {event_type} {strand_path}`
- Followed by: `Timestamp: {iso8601}`
- Then: `---`
- Then: the agent's response body

- Define `TieOffSection` struct: `{ knot_name: String, event_type: String, strand_path: String, timestamp: String, body: String }`
- `fn parse_sections(content: &str) -> Vec<TieOffSection>` — splits on `---`, parses headers, extracts body
- `fn extract_last_n(content: &str, strand_path: &str, n: usize) -> Vec<TieOffSection>` — filters by strand path, returns last N

**Tests (in same file):**
- `parse_sections_empty_input` — empty string returns empty vec
- `parse_sections_single_section` — one section parses correctly
- `parse_sections_multiple_sections` — three sections all parse
- `parse_sections_preserves_body_newlines` — body content with newlines is preserved
- `extract_last_n_filters_by_strand` — mixed strands, only matching strand returned
- `extract_last_n_limits_to_n` — more than N matching entries, only last N returned
- `extract_last_n_less_than_n` — fewer than N matching entries, returns all
- `extract_last_n_no_matches` — strand not found, returns empty vec
- `parse_sections_malformed_header` — sections without valid header line are skipped gracefully

**Hex layer:** Domain (pure parsing, no IO)

### Phase 1: Context Extraction in ProcessStrand (Application Layer)

Integrate the parser into `ProcessStrand::execute()`.

**Changes:**
- In `ProcessStrand::execute()`, before building `ExecutionContext`:
  - Read the existing tie-off file via `tie_off_sink.read_content(&tie_off_path)`
  - For `Deleted` events: call `extract_last_n(tie_off_content, &strand_path, 5)` to get scoped history
  - Build a context block string from the extracted sections
- For `Deleted` events specifically:
  - **Skip** the `@{strand_path}` CLI arg (file is gone)
  - **Inject** a deletion notice into the prompt: `"This file was deleted. There may be git history to help understand the file scope if you need to rectify downstream references due to this deletion."`
  - **Append** the scoped strand history (last 5 entries) to the prompt context
- For `Created`/`Modified` events: no change — continue using `@{strand_path}` as before

**Prompt structure for deleted events (written to stdin):**
```
{profile_prompt}

{knot_instructions}

This file was deleted. There may be git history to help understand the file scope if you need to rectify downstream references due to this deletion.

Strand: {strand_path}
Previous processing history (last 5 entries):

## review triggered by Created strand.md
Timestamp: 2026-06-05T10:00:00Z
{body of entry 1}

## review triggered by Modified strand.md
Timestamp: 2026-06-05T11:00:00Z
{body of entry 2}

---

**review** triggered by **Deleted** on **strand.md**
```

**Tests (in `usecases.rs`):**
- `process_strand_deleted_skips_at_file_arg` — verify `cli_args` does not contain `@` reference for Deleted events
- `process_strand_deleted_injects_deletion_notice` — verify prompt contains deletion notice
- `process_strand_deleted_includes_strand_history` — verify prompt contains extracted sections
- `process_strand_created_still_uses_at_file` — regression guard: Created events still use `@file`
- `process_strand_deleted_no_history_injects_notice_only` — when no previous entries exist, only deletion notice is injected

**Hex layer:** Application (use case)

### Phase 2: Integration Tests (End-to-End)

Full pipeline tests verifying deletion context works through the real file system and mock agent.

**File:** `tests/tie_off.rs` (extend existing tests) or new section in `tests/pipeline.rs`

**Tests:**
- `delete_event_agent_receives_context` — write a strand, let it process, delete it, verify the agent's received prompt contains deletion notice + previous entries
- `delete_event_agent_skips_missing_file` — verify no error about missing file in agent execution
- `delete_event_large_tieoff_bounded_context` — create many entries for multiple strands, delete one strand, verify only last N entries for that strand appear in prompt (not all entries)

**Hex layer:** Integration (end-to-end with real filesystem)

### Phase 3: Short-Term Bug Fix — Skip @file for Deleted Events

This is the minimal fix that addresses the immediate bug. Can be implemented independently of the context extraction.

**Change:** In `ProcessStrand::execute()`, wrap the `@{strand_path}` push in a conditional:

```rust
if !matches!(event, StrandEvent::Deleted { .. }) {
    cli_args.push(format!("@{}", strand_path.0.display()));
}
```

This prevents the `pi` warning about reading a deleted file. Without the context extraction (Phase 1), the agent still gets the trigger line (`**knot** triggered by **Deleted** on **file.md**`) which tells it the event type and strand path.

**Tests:**
- `process_strand_deleted_no_at_file_arg` — regression guard

**Hex layer:** Application (one-line change)

**Note:** This can be done as a standalone fix before the full context extraction plan. If we want to get the immediate bug fix out quickly, this phase can be extracted into its own plan or merged with Phase 1.

## Notes

### Why Last 5 Entries?

Five is a reasonable default that gives the agent enough history to understand the strand's processing lifecycle (e.g., created → modified → modified → review comments) without context overflow. A strand typically won't have more than a few dozen processing events before being resolved. If 5 proves insufficient, this can be made configurable in a future plan.

### Why Not Add It Back as `previous_tie_off`?

Plan 30 removed `previous_tie_off` because full tie-off files can be large. Rather than re-adding an unbounded field, we scope the context to the specific strand and limit to N entries. This is injected directly into the prompt string (not stored in `ExecutionContext`) to avoid schema churn.

### Deletion Notice Text

The notice text ("This file was deleted...") is a static string injected for all deleted events. It gives the agent a hint that the file is gone and suggests git history as a fallback. The agent's prompt template and profile instructions can further guide how it should respond to deletions.

### Scope: Deleted Events Only

This plan scopes to `Deleted` events. For `Created`/`Modified` events, the `@file` reference works fine and the agent reads the current file content directly. Adding scoped history for those events could be a future plan if needed.
