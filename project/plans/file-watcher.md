# Plan: Outbound Adapters

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan implements all outbound adapters — concrete types that satisfy the port traits defined in Plan 2. Each adapter wraps real IO (filesystem, notify, subprocess). Tests use `tempfile` for isolation and mock binaries for subprocess testing.

## Problem

Knot has domain types (Plan 1) and port interfaces (Plan 2) but no concrete adapters. The ports are empty contracts — nothing actually reads from disk, watches directories, invokes the agent CLI, or writes tie-offs. Without adapters, the system cannot interact with the real world.

## Target

- `FileSystemLoomRepository` — scans workspace, discovers looms and knot files
- `FileSystemKnotStateStore` — reads/writes knot-state JSON files
- `FileSystemLoomLog` — appends/reads loom-log JSONL files
- `NotifyEventSource` — wraps `notify::Watcher`, emits raw events to a channel
- `SubprocessAgentRunner` — invokes agent CLI via `tokio::process::Command`
- `FileSystemTieOffSink` — writes tie-off files to disk
- All adapters implement their respective port traits from Plan 2

## Implementation Status: ⬜ Draft

## Hex Layer: Outbound Adapters

Each adapter implements a port trait. Depends on domain types and application ports. Uses concrete crates (`std::fs`, `notify`, `tokio::process`).

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `http_interface.rs` | Health endpoint, agent listing | ✅ Green — baseline HTTP tests |
| `filesystem_interface.rs` | Filesystem operations | ✅ Green — baseline FS tests |
| `domain::*` tests | Domain entities, value objects, events | Plan 1 |
| `application::*` tests | Use cases with mock ports | Plan 2 |

## Test Gaps

- No adapter tests for filesystem loom scanning.
- No adapter tests for knot-state file read/write.
- No adapter tests for loom-log append/read.
- No adapter tests for notify event emission.
- No adapter tests for subprocess agent execution.
- No adapter tests for tie-off file writing.
- No adapter tests verify trait implementation (`assert_impl!` or trait usage).

## Phases

### Phase 0: FileSystemLoomRepository
**Failing tests created:** `adapters::filesystem::tests::scan_empty_workspace`, `adapters::filesystem::tests::scan_workspace_with_one_loom`, `adapters::filesystem::tests::scan_workspace_with_multiple_looms`, `adapters::filesystem::tests::scan_skips_invalid_knot_files`, `adapters::filesystem::tests::scan_parses_knot_definition_files`, `adapters::filesystem::tests::get_nonexistent_loom`, `adapters::filesystem::tests::save_and_list_loom`

- [x] Failing test: `adapters::filesystem::tests::scan_empty_workspace` — scan a `tempfile` dir with no subdirs; returns empty `Vec<Loom>`
- [x] Failing test: `adapters::filesystem::tests::scan_workspace_with_one_loom` — create a subdir with one valid `.md` knot file; scan returns one loom with one knot
- [x] Failing test: `adapters::filesystem::tests::scan_workspace_with_multiple_looms` — create two subdirs each with knot files; scan returns two looms
- [x] Failing test: `adapters::filesystem::tests::scan_skips_invalid_knot_files` — knot file with malformed frontmatter; scan returns loom but skips the invalid knot (logs warning)
- [x] Failing test: `adapters::filesystem::tests::scan_parses_knot_definition_files` — verify knot name, agent config, and prompt template are parsed from file content
- [x] Failing test: `adapters::filesystem::tests::get_nonexistent_loom` — `get()` for unknown ID returns `Ok(None)`
- [x] Failing test: `adapters::filesystem::tests::save_and_list_loom` — `save()` a loom, `list()` returns it
- [x] Implement `FileSystemLoomRepository` in `src/adapters/outbound/loom_repository.rs`
- [ ] Uses `std::fs::read_dir` to scan workspace, `KnotFileParser` from Plan 1 to parse knot files
- [ ] Implements `LoomRepository` port trait
- [ ] **Alert:** `FileSystemLoomRepository` depends on `KnotFileParser` (domain layer) — this is correct, adapters depend inward

### Phase 1: FileSystemKnotStateStore
**Failing tests created:** `adapters::filesystem::tests::knot_state_create_new_file`, `adapters::filesystem::tests::knot_state_update_state`, `adapters::filesystem::tests::knot_state_read_current`, `adapters::filesystem::tests::knot_state_status_transitions`, `adapters::filesystem::tests::knot_state_get_nonexistent`

- [ ] Failing test: `adapters::filesystem::tests::knot_state_create_new_file` — `create(knot_id)` writes a JSON file with `status: idle`; file exists on disk in `tempfile` dir
- [ ] Failing test: `adapters::filesystem::tests::knot_state_update_state` — `update(state)` overwrites the file; read back matches new state
- [ ] Failing test: `adapters::filesystem::tests::knot_state_read_current` — `get(knot_id)` reads the file and returns parsed `KnotState`
- [ ] Failing test: `adapters::filesystem::tests::knot_state_status_transitions` — write `idle`, update to `processing`, update to `completed`; each read reflects latest
- [ ] Failing test: `adapters::filesystem::tests::knot_state_get_nonexistent` — `get()` for unknown ID returns `Ok(None)`
- [ ] Implement `FileSystemKnotStateStore` in `src/adapters/outbound/knot_state.rs`
- [ ] Writes JSON to `<loom-dir>/.knots/<knot-name>.state`
- [ ] Implements `KnotStatePort` trait
- [ ] **Alert:** uses `std::fs` and `serde_json` — adapter layer, correct

### Phase 2: FileSystemLoomLog
**Failing tests created:** `adapters::filesystem::tests::loom_log_create_and_append`, `adapters::filesystem::tests::loom_log_read_all`, `adapters::filesystem::tests::loom_log_multiple_events`, `adapters::filesystem::tests::loom_log_concurrent_writes`

- [ ] Failing test: `adapters::filesystem::tests::loom_log_create_and_append` — `open(loom_id)` creates file, `append(event)` writes one line; file has one JSONL entry
- [ ] Failing test: `adapters::filesystem::tests::loom_log_read_all` — after appending 3 events, `read_all()` returns 3 entries in order
- [ ] Failing test: `adapters::filesystem::tests::loom_log_multiple_events` — append events of different types (`KnotRegistered`, `LoomStarted`); all preserved
- [ ] Failing test: `adapters::filesystem::tests::loom_log_concurrent_writes` — 10 concurrent `append()` calls; all 10 entries present (no data loss)
- [ ] Implement `FileSystemLoomLog` in `src/adapters/outbound/loom_log.rs`
- [ ] Writes JSONL (one JSON object per line) to `<loom-dir>/.loom-log`
- [ ] Implements `LoomLogPort` trait
- [ ] Uses `Arc<Mutex<File>>` or similar for concurrent write safety
- [ ] **Alert:** uses `std::fs::OpenOptions` with append mode — adapter layer, correct

### Phase 3: NotifyEventSource
**Failing tests created:** `adapters::notify::tests::watcher_starts`, `adapters::notify::tests::create_event_emitted`, `adapters::notify::tests::modify_event_emitted`, `adapters::notify::tests::delete_event_emitted`, `adapters::notify::tests::directory_events_filtered`, `adapters::notify::tests::event_outside_source_dir_filtered`, `adapters::notify::tests::event_mapping_correct_types`

- [ ] Failing test: `adapters::notify::tests::watcher_starts` — create watcher, `watch(dir)` succeeds, watcher is active
- [ ] Failing test: `adapters::notify::tests::create_event_emitted` — create a file in watched dir; event received on channel with `StrandEvent::Created`
- [ ] Failing test: `adapters::notify::tests::modify_event_emitted` — modify a file; event received with `StrandEvent::Modified`
- [ ] Failing test: `adapters::notify::tests::delete_event_emitted` — delete a file; event received with `StrandEvent::Deleted`
- [ ] Failing test: `adapters::notify::tests::directory_events_filtered` — create a subdirectory; no event emitted (only files watched)
- [ ] Failing test: `adapters::notify::tests::event_outside_source_dir_filtered` — event for file outside watched dir; no event emitted
- [ ] Failing test: `adapters::notify::tests::event_mapping_correct_types` — `notify::EventKind::Create` maps to `StrandEvent::Created`, `Modify` → `Modified`, `Remove` → `Deleted`
- [ ] Add `notify = "7"` to `Cargo.toml`
- [ ] Implement `NotifyEventSource` in `src/adapters/outbound/event_source.rs`
- [ ] Wraps `notify::RecommendedWatcher`, maps raw events to `StrandEvent` domain type
- [ ] Implements `EventSource` port trait
- [ ] Emits raw events to an `mpsc::Sender<StrandEvent>` — the debounce engine (application layer) subscribes to this
- [ ] **Alert:** adapter emits raw events only; debounce is NOT part of this adapter (it's application layer, Plan 2 Phase 5)

### Phase 4: SubprocessAgentRunner
**Failing tests created:** `adapters::subprocess::tests::execute_successful_command`, `adapters::subprocess::tests::execute_captures_stdout`, `adapters::subprocess::tests::execute_captures_stderr`, `adapters::subprocess::tests::execute_command_not_found`, `adapters::subprocess::tests::execute_nonzero_exit_error`, `adapters::subprocess::tests::execute_timeout`

- [ ] Failing test: `adapters::subprocess::tests::execute_successful_command` — execute `echo "hello"`; returns `AgentOutput` with exit code 0
- [ ] Failing test: `adapters::subprocess::tests::execute_captures_stdout` — execute `echo "test"`; stdout is `"test\n"`
- [ ] Failing test: `adapters::subprocess::tests::execute_captures_stderr` — execute `sh -c 'echo err >&2'`; stderr captured, stdout empty
- [ ] Failing test: `adapters::subprocess::tests::execute_command_not_found` — execute nonexistent binary; returns error with `PortError::CommandNotFound`
- [ ] Failing test: `adapters::subprocess::tests::execute_nonzero_exit_error` — execute `sh -c 'exit 1'`; returns error with exit code 1
- [ ] Failing test: `adapters::subprocess::tests::execute_timeout` — execute `sleep 30` with 100ms timeout; returns error with `PortError::Timeout`
- [ ] Implement `SubprocessAgentRunner` in `src/adapters/outbound/agent_runner.rs`
- [ ] Uses `tokio::process::Command` to spawn subprocess
- [ ] Captures stdout and stderr, respects timeout (120s default, configurable)
- [ ] Implements `AgentRunner` port trait
- [ ] Takes `ExecutionContext` (cli_path, cli_args, prompt, strand_path), constructs command, runs it

### Phase 5: FileSystemTieOffSink
**Failing tests created:** `adapters::filesystem::tests::tieoff_write_new_file`, `adapters::filesystem::tests::tieoff_overwrite_existing`, `adapters::filesystem::tests::tieoff_filename_derived_from_strand`, `adapters::filesystem::tests::tieoff_create_parent_dirs`

- [ ] Failing test: `adapters::filesystem::tests::tieoff_write_new_file` — write a `TieOff` with content; file created at tie-off path with correct content
- [ ] Failing test: `adapters::filesystem::tests::tieoff_overwrite_existing` — write tie-off twice; file contains second write (overwritten, never deleted)
- [ ] Failing test: `adapters::filesystem::tests::tieoff_filename_derived_from_strand` — strand `input.md` produces tie-off `input.tie-off.md`
- [ ] Failing test: `adapters::filesystem::tests::tieoff_create_parent_dirs` — tie-off point directory does not exist; `write()` creates it
- [ ] Implement `FileSystemTieOffSink` in `src/adapters/outbound/tieoff_sink.rs`
- [ ] Derives tie-off filename from strand filename: `<name>.tie-off.<ext>`
- [ ] Implements `TieOffSink` port trait
- [ ] Uses `std::fs::create_dir_all` + `std::fs::write`

## Notes
