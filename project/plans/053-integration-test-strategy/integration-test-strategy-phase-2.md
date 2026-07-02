# Phase 2: Adapter test extraction

**Plan:** [Integration Test Strategy](integration-test-strategy-plan.md)

## Checklist
- [ ] Create `tests/adapter_test.rs` (or individual modules) for adapter contract tests
- [ ] `PiStdioAgentRunner` adapter test:
  - [ ] Subprocess spawn with mock script → captures stdout
  - [ ] Non-zero exit code → returns `PortError::AgentExecutionFailed`
  - [ ] Timeout enforcement → returns `PortError::Timeout`
  - [ ] Session ID surfaced in error (when mock returns session JSON)
  - [ ] Unique `tempfile::tempdir()` + mock path per test
- [ ] `PiJsonAgentRunner` adapter test:
  - [ ] JSON-L parsing → extracts `session_id` from `agent_end` event
  - [ ] `stopReason: "stop"` → produces response text
  - [ ] `stopReason: "toolUse"` → excluded from response
  - [ ] `stopReason: "error"` → excluded from response
  - [ ] `stopReason: "length"` → included (truncated response)
  - [ ] Unique `tempfile::tempdir()` + mock path per test
- [ ] `FileSystemTieOffSink` adapter test:
  - [ ] `write()` creates file at correct path
  - [ ] `append()` adds delimiter + header before new content
  - [ ] `read_content()` returns existing content
  - [ ] Directory creation (creates `tie-offs/{loom}/` if missing)
- [ ] `FileSystemLoomLog` adapter test:
  - [ ] `open()` creates directory + empty log file
  - [ ] `append()` writes JSONL entry
  - [ ] `read_all()` returns parsed events
  - [ ] Idempotent `open()` (no error on re-open)
- [ ] `FileSystemStateWriter` adapter test:
  - [ ] `write_state()` writes valid JSON to `state.json`
  - [ ] Atomic write: `.state.json.tmp` → `rename` to `state.json`
  - [ ] Concurrent writes do not corrupt (two writers, verify valid JSON)
- [ ] `FileSystemLoomRepository` adapter test:
  - [ ] `scan()` discovers looms ending in `-loom`
  - [ ] `scan_knot_files()` parses `.md` knot files with YAML frontmatter
  - [ ] Parse warnings for unknown YAML properties
  - [ ] `save()` writes loom definition
- [ ] `FileSystemAgentProfileRepository` adapter test:
  - [ ] `get()` reads profile from `{name}.md`
  - [ ] `list()` returns all profiles
  - [ ] YAML frontmatter parsing (name, provider, model, prompt body)
- [ ] `NotifyEventSource` adapter test:
  - [ ] `watch()` starts notify watching
  - [ ] File create → event emitted on channel
  - [ ] File modify → event emitted on channel
  - [ ] File delete → event emitted on channel
  - [ ] `unwatch()` stops receiving events
- [ ] All adapter tests use `tempfile::tempdir()`, unique mock paths
- [ ] Verify: `cargo test --test adapter_test --test-threads=4` passes
- [ ] Run full test suite — verify no regressions

## Deviations

## Discoveries

## Notes
