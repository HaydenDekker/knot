# Plan: Context Management — Slim Agent Prompt and Tie-Off Headers

## Problem

Each agent call was prepending the **entire knot tie-off history** to the beginning of the prompt via `previous_tie_off`. This grows unbounded with every strand event — the tie-off file is an append-only record of all processing, and every call to the agent re-injects the full document. This wastes context window, slows down agent responses, and provides diminishing value (the agent already has the input file via `@{path}`).

The tie-off section headers also used a three-line format (`## Event:`, `## Strand:`, `## Timestamp:`) that is verbose and not well-suited for quick scanning when reading tie-offs back through the pi agent.

## Target

Agent prompt contains only:
1. **System prompt** from agent profile (via `--system-prompt`) — already correct
2. **Knot instruction** (prompt body) — already correct
3. **Input file** via `@{strand_path}` in CLI args — already correct
4. **Short trigger line** — `**{knot-name}** triggered by **{event-type}** on **{file-name}**`

Tie-off section headers use a single-line format:
`## {knot-name} triggered by {event-type} {file-name}`

## Implementation Status: ✅ Complete (2026-06-15)

All changes implemented in a single session. 359 tests pass (303 lib + 56 integration).

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `subprocess::tests::runner_passes_event_metadata` | Subprocess runner prepends event context | ✅ Unit — updated for new format |
| `tieoff_sink::tests::append_mode_creates_file` | First append creates header section | ✅ Unit — updated assertions |
| `tieoff_sink::tests::append_mode_adds_section` | Second append adds delimiter + section | ✅ Unit — updated assertions |
| `tieoff_sink::tests::append_mode_preserves_history` | Three appends produce three sections | ✅ Unit — updated assertions |
| `tests/tie_off::full_tie_off_history` | End-to-end create/modify/delete sections | ✅ Integration — updated assertions |
| `tests/tie_off::tie_off_sections_readable` | Parse tie-off sections and verify metadata | ✅ Integration — updated assertions |
| `tests/pipeline::full_pipeline_create_modify_delete` | Full pipeline including delete handling | ✅ Integration — updated assertions |

## Test Gaps

None — all existing tests updated and passing.

## Phases

### Phase 0: Domain and Port Changes ✅ Done
- [x] Add `knot_name: Option<String>` to `TieOff` entity (`src/domain/entities.rs`)
- [x] Remove `previous_tie_off: String` from `ExecutionContext` (`src/application/ports.rs`)
- [x] Add `knot_name: Option<String>` to `ExecutionContext`
- [x] Update domain entity test fixtures

### Phase 1: Adapter Changes ✅ Done
- [x] Rewrite `build_prompt_with_context()` in `SubprocessAgentRunner` (`src/adapters/subprocess.rs`)
  - Old: Multi-line `## Event Context` block with full previous tie-off content
  - New: Single trigger line `**{knot-name}** triggered by **{event-type}** on **{file-name}**`
- [x] Update tie-off header format in `FileSystemTieOffSink::append()` (`src/adapters/outbound/tieoff_sink.rs`)
  - Old: `## Event: {type}\n## Strand: {path}\n## Timestamp: {ts}\n---\n`
  - New: `## {knot-name} triggered by {event-type} {strand-path}\nTimestamp: {ts}\n---\n`
- [x] Update all adapter unit tests

### Phase 2: Use Case Changes ✅ Done
- [x] Remove `previous_tie_off` read from `ProcessStrand::execute()` (`src/application/usecases.rs`)
  - No longer reads existing tie-off file before calling the agent
- [x] Pass `knot_name` in `ExecutionContext`
- [x] Pass `knot_name` in all `TieOff` constructions (success + error paths)

### Phase 3: Integration Test Updates ✅ Done
- [x] Update `tests/tie_off.rs` — header assertions (`## Event:` → `triggered by`)
- [x] Update `tests/pipeline.rs` — deleted event header assertion
- [x] All 359 tests pass

## Notes

### Design Decision — Why Not Keep Previous Tie-Off at All?

The original design (Plan #12, `design-tie-off-append-and-event-context.md`) had the agent receive the full previous tie-off to make "context-aware decisions." In practice:
- The agent already has the input file via `@{strand_path}`
- The tie-off is an append-only log — early sections are irrelevant to later processing
- The full history quickly consumes the context window for high-throughput looms
- The trigger line gives the agent event awareness (created/modified/deleted) without the bloat

The tie-off file itself still preserves full history for human review — it's just no longer re-injected into the agent's context.

### Header Format Choice

The new single-line header `## {knot-name} triggered by {event-type} {file-name}` is designed to be easily scannable when reading tie-offs back through the pi agent. Bold formatting in the trigger line (`**knot-name**`) aids visual parsing.

### Files Changed

| File | Layer | Change |
|------|-------|--------|
| `src/domain/entities.rs` | Domain | `TieOff.knot_name` field added |
| `src/application/ports.rs` | Application | `ExecutionContext.previous_tie_off` removed, `knot_name` added |
| `src/application/usecases.rs` | Application | `ProcessStrand` no longer reads tie-off; passes `knot_name` |
| `src/adapters/subprocess.rs` | Outbound Adapter | `build_prompt_with_context()` rewritten |
| `src/adapters/outbound/tieoff_sink.rs` | Outbound Adapter | Header format updated |
| `tests/tie_off.rs` | Integration | Assertions updated |
| `tests/pipeline.rs` | Integration | Assertions updated |
