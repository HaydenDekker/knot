# Plan: Loom Config, Path Resolution and Agent Error Logging

## Related PRD

This plan contributes to [AI-Driven File Generation from Loom Events](../prds/prd-ai-driven-file-generation.md).

This plan hardens the filesystem adapter layer: canonical path resolution for watchers, configurable loom source/tie-off directories via `.loom-config.yaml`, and proper error logging when the agent CLI fails.

## Problem

Three issues were discovered during test-project integration:

1. **Relative paths break the watcher** — when `base_dir` is relative (e.g. `.`), loom `source_dir` is stored as a relative path (`./summary-loom`). `notify::Watcher` resolves paths differently, so file events never match the watched directory prefix. A patch was applied (canonicalise `source_dir` in `FileSystemLoomRepository::scan()`), but it has no test coverage.

2. **Strands and tie-offs live outside the loom** — strands come from external project-specific directories and tie-offs land in separate output directories. Currently the loom's `source_dir` defaults to the loom directory itself and `tie_off_dir` to `<loom>/.knot-output`. There is no mechanism to point a loom at an external source or output directory.

3. **Agent errors are silent in knot-state** — when the agent CLI (`pi`) is not found or exits non-zero, `ProcessStrand` catches the error and writes a `Failed` tie-off, but the knot-state error field and loom-log do not clearly surface the root cause. Users need to see *why* a knot failed without hunting through logs.

## Target

- `FileSystemLoomRepository::scan()` resolves `source_dir` to an absolute path (canonicalised).
- Looms support `.loom-config.yaml` to override `source_dir` and `tie_off_dir` with absolute or relative paths (resolved relative to the loom directory).
- Agent CLI failures produce a `Failed` knot-state entry with the error message and a `StrandProcessed` event that includes the error details in the loom-log.
- All changes are covered by unit tests (adapter layer) and integration tests (full pipeline).

## Implementation Status: ⬜ Draft

## Hex Layer: Outbound Adapters + Application

| Change | Hex Layer |
|--------|-----------|
| Canonical path in `scan()` | Outbound adapter (`FileSystemLoomRepository`) |
| `.loom-config.yaml` parsing | Outbound adapter (`FileSystemLoomRepository`) |
| Agent error in knot-state + loom-log | Application (`ProcessStrand` use case) |

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `loom_repository.rs` tests | `FileSystemLoomRepository::scan()` with valid/invalid knot files | ✅ Green — no path-resolution tests |
| `integration.rs` tests | Full pipeline with mock agent CLI | ✅ Green — no external source_dir tests |
| `usecases.rs` tests | `ProcessStrand` with mock ports | ✅ Green — error path tested against mocks |

## Test Gaps

- No test verifies that `source_dir` is canonicalised to an absolute path.
- No test for `.loom-config.yaml` parsing (feature does not exist yet).
- No integration test with an external source directory (strands outside the loom).
- No test verifies knot-state error message content when agent CLI is not found.
- No test verifies loom-log contains agent error details.

## Phases

### Phase 0: Canonical Path Tests for `FileSystemLoomRepository::scan()`
**Goal:** Verify the existing canonicalisation patch is correct and covered by tests.

- [ ] Unit test: `scan_workspace_with_relative_path` — pass a relative path (`"./workspace"`) to `scan()`; assert every loom's `source_dir` is absolute (contains no `..` or `.` components).
- [ ] Unit test: `scan_workspace_with_absolute_path` — pass an absolute path; assert `source_dir` is canonicalised (no double-slashes, symlinks resolved).
- [ ] Verify existing `loom_repository.rs` tests still pass.

### Phase 1: `.loom-config.yaml` for External Source and Tie-off Directories
**Goal:** A loom can declare its source and output directories via `.loom-config.yaml`, supporting both absolute and relative paths.

- [ ] Domain value object: add a `LoomConfig` struct (or reuse YAML parsing in the adapter layer — keeping it in the adapter avoids domain dependency on `serde_yaml`).
- [ ] Adapter change: `FileSystemLoomRepository::scan()` reads `<loom-dir>/.loom-config.yaml` (if present).
- [ ] Config schema:
  ```yaml
  source_dir: ../app          # relative to loom dir, or absolute
  tie_off_dir: ../output      # relative to loom dir, or absolute
  ```
- [ ] Path resolution: relative paths are joined to the loom directory then canonicalised. Absolute paths are canonicalised directly.
- [ ] Fallback: when `.loom-config.yaml` is absent, `source_dir` = loom directory, `tie_off_dir` = `<loom>/.knot-output` (current behaviour).
- [ ] Failing tests first:
  - `scan_uses_loom_config_source_dir` — `.loom-config.yaml` with `source_dir: ../app`; loom's `source_dir` resolves to the external directory.
  - `scan_uses_loom_config_tie_off_dir` — `.loom-config.yaml` with `tie_off_dir: ../output`; loom's `tie_off_dir` resolves correctly.
  - `scan_fallback_defaults_without_config` — no `.loom-config.yaml`; uses loom dir for source, `.knot-output` for tie-off.
  - `scan_loom_config_absolute_paths` — absolute paths in config; used as-is (canonicalised).
  - `scan_loom_config_malformed_yaml` — invalid YAML; falls back to defaults with a warning.
- [ ] Integration test: `full_pipeline_external_source_dir` — workspace loom with `.loom-config.yaml` pointing `source_dir` to an external directory; create a strand in that external directory; verify tie-off is produced at the configured `tie_off_dir`.

### Phase 2: Agent Error Logging in Knot-State and Loom-Log
**Goal:** When the agent CLI fails, the error is visible in knot-state and loom-log.

- [ ] Application change: `ProcessStrand::execute()` already writes `error: Some(error_msg)` to knot-state on failure. Verify the `error` field is populated with the `PortError` display string.
- [ ] Loom-log change: the `StrandProcessed` event currently only carries `loom_id` and `strand_path`. Extend it to carry an optional `error` field (or a separate event type like `StrandFailed`).
  - Domain change: add `error: Option<String>` to `LoomEvent::StrandProcessed` (or add a new variant `StrandFailed`).
  - Adapter change: `FileSystemLoomLog` serialises the event with the error field.
- [ ] Failing tests first:
  - Unit test (usecases): `process_strand_agent_not_found_logs_error` — `AgentRunner` returns `PortError::CommandNotFound`; verify knot-state `error` field is set and loom-log event contains the error message.
  - Unit test (usecases): `process_strand_agent_nonzero_exit_logs_error` — `AgentRunner` returns `PortError::AgentExecutionFailed`; same verification.
  - Integration test: `full_pipeline_agent_error_in_state_and_log` — use `WorkspaceAgentConfig` with a nonexistent `cli_path`; create a strand; verify knot-state shows `Failed` with error message and loom-log contains the error.

### Phase 3: Full Integration Verification
**Goal:** End-to-end test that ties all three phases together.

- [ ] Integration test: `full_pipeline_external_source_with_agent_error` — loom with `.loom-config.yaml` pointing to external source; nonexistent agent CLI; verify:
  1. Loom discovered with correct absolute `source_dir` and `tie_off_dir`.
  2. Strand in external directory triggers processing.
  3. Knot-state shows `Failed` with descriptive error.
  4. Loom-log contains `StrandProcessed` with error details.
  5. Tie-off file written at external `tie_off_dir` with `Failed` status.
- [ ] Integration test: `full_pipeline_external_source_with_mock_agent_success` — same setup but with a mock agent CLI (`echo "summary"`); verify the full happy path with external directories.

## Notes
