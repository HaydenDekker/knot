# Design: Tie-Off Append Mode and Event Context

## What It Is

A tie-off file (agent response output for a strand) now uses append mode with structured metadata headers instead of overwriting on every strand event. Each event appends a new section with `---` delimiters and `## Event:` headers. The agent receives event metadata (event type, strand path, previous tie-off content) prepended to its prompt via the subprocess runner.

## What It Does

- **Append-mode tie-off files** — Every strand event (create, modify, delete) appends a new section to the tie-off file. Reading the output shows the complete chronological history.
- **Event metadata in tie-off** — Each section has `## Event:`, `## Strand:`, and `## Timestamp:` headers before the agent's response.
- **Event context in agent prompt** — The agent runner prepends a `## Event Context` block to the prompt, giving the agent awareness of the event type and any previous tie-off content.
- **Delete events trigger the agent** — Delete events no longer short-circuit with a static tombstone string. The agent is invoked and produces a response about the deletion, which is appended to the tie-off.

## Components

| Component | Location | Role |
|-----------|----------|------|
| `TieOff` | `src/domain/entities.rs` | Domain entity with optional event metadata fields (`event_type`, `strand_path`, `timestamp`) |
| `TieOffSink` trait | `src/application/ports.rs` | Port interface with `write()`, `append()`, and `read_content()` methods |
| `FileSystemTieOffSink` | `src/adapters/outbound/tieoff_sink.rs` | Append-mode implementation with metadata header formatting and ISO 8601 timestamp generation |
| `ExecutionContext` | `src/application/ports.rs` | Agent execution context extended with `event_type` and `previous_tie_off` |
| `SubprocessAgentRunner` | `src/adapters/subprocess.rs` | Prepends `## Event Context` block to agent prompt via `build_prompt_with_context()` |
| `ProcessStrand::execute()` | `src/application/usecases.rs` | Reads previous tie-off content, passes event metadata to agent, uses `append()` instead of `write()` |

## Configuration

### Tie-Off Metadata Header Format

```markdown
## Event: Created | Modified | Deleted
## Strand: /path/to/strand.md
## Timestamp: 2026-06-05T14:00:00Z
---
<agent response content>
```

- **Event label** — One of: `Created`, `Modified`, `Deleted` (from `KnotEventType`)
- **Strand path** — Display path of the strand file
- **Timestamp** — ISO 8601 UTC format (`YYYY-MM-DDTHH:MM:SSZ`)
- **Delimiter** — `---` horizontal rule separates sections

### Event Context Block in Agent Prompt

When `event_type` or `previous_tie_off` are set, the `SubprocessAgentRunner` prepends:

```markdown
## Event Context
Event: Modified
Strand: /path/to/strand.md
Previous tie-off:
<content of existing tie-off file>

<original prompt>
```

### TieOff Struct Fields

| Field | Type | Description |
|-------|------|-------------|
| `event_type` | `Option<String>` | Event label for the section header |
| `strand_path` | `Option<String>` | Strand file path for the section header |
| `timestamp` | `Option<String>` | ISO 8601 timestamp (auto-generated if None) |

## Dependencies

| Dependency | Purpose |
|------------|---------|
| `serde_json` | TieOff serialization (unchanged) |
| `tempfile` | Test fixtures (unchanged) |
| No new runtime dependencies | Timestamp formatting uses manual epoch-to-date conversion |

## How It Works

### Append Flow

1. `ProcessStrand::execute()` processes a `StrandEvent`
2. For the tie-off output, it calls `self.tie_off_sink.append(tie_off)` instead of `write()`
3. `FileSystemTieOffSink::append()` checks if the file exists:
   - **Does not exist**: Creates file with metadata header + content
   - **Exists**: Reads existing content, appends `\n---\n`, then new metadata header + content
4. The tie-off file grows with each event, showing full history

### Event Context Flow

1. `ProcessStrand::execute()` reads existing tie-off content via `self.tie_off_sink.read_content(&tie_off_path)`
2. Builds `ExecutionContext` with `event_type` (from the `StrandEvent`) and `previous_tie_off`
3. `SubprocessAgentRunner::execute()` calls `build_prompt_with_context()` which prepends the event context block to the prompt
4. The agent receives the event metadata as part of its input and can make context-aware decisions

### Timestamp Formatting

ISO 8601 timestamps are generated without the `chrono` crate:
- `FileSystemTieOffSink::format_timestamp(SystemTime)` converts to Unix epoch seconds
- `days_to_ymd(u64)` converts epoch days to year/month/day using the Gregorian calendar algorithm

## API Contract

### TieOffSink Trait

```rust
pub trait TieOffSink: Send + Sync {
    fn write(&self, tie_off: TieOff) -> Result<(), PortError>;
    fn append(&self, tie_off: TieOff) -> Result<(), PortError>;
    fn read_content(&self, path: &TieOffPath) -> Result<String, PortError>;
}
```

### ExecutionContext Struct

```rust
pub struct ExecutionContext {
    pub cli_path: String,
    pub cli_args: Vec<String>,
    pub prompt: String,
    pub strand_path: StrandPath,
    pub event_type: String,       // NEW
    pub previous_tie_off: String, // NEW
}
```

### TieOff Struct (expanded)

```rust
pub struct TieOff {
    pub content: String,
    pub path: TieOffPath,
    pub status: TieOffStatus,
    pub event_type: Option<String>,   // NEW
    pub strand_path: Option<String>,  // NEW
    pub timestamp: Option<String>,    // NEW
}
```

## Testing

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `tieoff_sink::tests::append_mode_creates_file` | First append creates file with header | ✅ Unit |
| `tieoff_sink::tests::append_mode_adds_section` | Second append adds delimiter and new section | ✅ Unit |
| `tieoff_sink::tests::append_mode_preserves_history` | Three appends produce three readable sections | ✅ Unit |
| `subprocess::tests::runner_passes_event_metadata` | Subprocess runner forwards event metadata to agent stdin | ✅ Unit |
| `integration::tests::full_tie_off_history` | End-to-end: write → write → delete → 3 sections in tie-off | ✅ Integration |
| `integration::tests::tie_off_sections_readable` | Parse tie-off sections and verify metadata structure | ✅ Integration |
| `integration::tests::full_pipeline_create_modify_delete` | Updated: delete events produce agent response with Deleted header | ✅ Integration |

## Notes

### File Watcher Event Coalescing

On Linux (inotify), rapid file creation + write can coalesce into a single `Modify` event within the debounce window. Integration tests cannot reliably verify `StrandEvent::Created` from `fs::write()` on a new file — the effective event type is `Modified`. Tests focus on metadata structure rather than specific event types for the initial write.

### Timestamp Implementation Choice

ISO 8601 timestamps are generated with a manual epoch-to-date converter (`days_to_ymd`) rather than adding the `chrono` dependency. This is acceptable because tie-off files are agent responses (not megabyte-scale) and timestamp formatting is a one-time cost per event.

### Disabled Tests Module

The `#[cfg(feature = "__disabled_tests")]` module in `usecases.rs` has pre-existing compilation errors (`KnotState` vs `KnotStatus` naming mismatch). It was left disabled during this plan.

### Previous Tie-Off for Create Events

For create events, there is no previous tie-off (the file does not exist yet). `read_content()` returns an empty string, and `previous_tie_off` is empty in the agent prompt.

### Delete Event Agent Response

Delete events no longer produce a static tombstone string. The agent receives event context about the deletion (strand path, previous tie-off) and produces its own response. The agent's response quality depends on its prompt instructions — it may not always produce useful output for deletions.
