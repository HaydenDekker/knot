# Phase 2: Adapter test extraction

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [x] Create `tests/adapter_test.rs` (or individual modules) for adapter contract tests
- [x] `PiStdioAgentRunner` adapter test:
  - [x] Subprocess spawn with mock script → captures stdout
  - [x] Non-zero exit code → returns `PortError::AgentExecutionFailed`
  - [x] Timeout enforcement → returns `PortError::Timeout`
  - [x] Session ID surfaced in error (when mock returns session JSON) — N/A for stdio (no session ID capture)
  - [x] Unique `tempfile::tempdir()` + mock path per test
- [x] `PiJsonAgentRunner` adapter test:
  - [x] JSON-L parsing → extracts `session_id` from `agent_end` event
  - [x] `stopReason: "stop"` → produces response text
  - [x] `stopReason: "toolUse"` → excluded from response
  - [x] `stopReason: "error"` → excluded from response
  - [x] `stopReason: "length"` → included (truncated response)
  - [x] Unique `tempfile::tempdir()` + mock path per test
- [x] `FileSystemTieOffSink` adapter test:
  - [x] `write()` creates file at correct path
  - [x] `append()` adds delimiter + header before new content
  - [x] `read_content()` returns existing content
  - [x] Directory creation (creates `tie-offs/{loom}/` if missing)
- [x] `FileSystemLoomLog` adapter test:
  - [x] `open()` creates directory + empty log file
  - [x] `append()` writes JSONL entry
  - [x] `read_all()` returns parsed events
  - [x] Idempotent `open()` (no error on re-open)
- [x] `FileSystemStateWriter` adapter test:
  - [x] `write_state()` writes valid JSON to `state.json`
  - [x] Atomic write: `.state.json.tmp` → `rename` to `state.json`
  - [x] Concurrent writes do not corrupt (two writers, verify valid JSON)
- [x] `FileSystemLoomRepository` adapter test:
  - [x] `scan()` discovers looms ending in `-loom`
  - [x] `scan_knot_files()` parses `.md` knot files with YAML frontmatter
  - [x] Parse warnings for unknown YAML properties
  - [x] `save()` writes loom definition
- [x] `FileSystemAgentProfileRepository` adapter test:
  - [x] `get()` reads profile from `{name}.md`
  - [x] `list()` returns all profiles
  - [x] YAML frontmatter parsing (name, provider, model, prompt body)
- [x] `NotifyEventSource` adapter test:
  - [x] `watch()` starts notify watching
  - [x] File create → event emitted on channel
  - [x] File modify → event emitted on channel
  - [x] File delete → event emitted on channel
  - [x] `unwatch()` stops receiving events
- [x] All adapter tests use `tempfile::tempdir()`, unique mock paths
- [x] Verify: `cargo test --test adapters --test-threads=4` passes (33/33)
- [ ] Run full test suite — verify no regressions

## Deviations

- Stdio adapter does not capture session IDs (by design — that's the json adapter's role). The checklist item "Session ID surfaced in error" is marked N/A for `PiStdioAgentRunner`.

## Discoveries

- Adapter trait methods (e.g. `scan`, `save`, `watch`, `unwatch`) require the trait to be in scope when called from integration tests. Fixed by importing `LoomRepository`, `AgentProfileRepository`, `EventSource` etc.
- `with_cli_path_and_timeout` (public) is used instead of `with_cli_path` (#[cfg(test)]) since integration tests are outside the library crate.

## Notes

- `tests/adapters.rs` contains 33 tests across 8 adapter modules.
- All tests run in parallel under `--test-threads=4` (~0.71s total).
- Each test creates its own `tempfile::tempdir()` — no shared state between tests.
