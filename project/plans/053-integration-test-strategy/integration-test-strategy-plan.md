# Plan: Integration Test Strategy

## Problem

The integration test suite has three interconnected problems that make it unreliable and slow:

### 1. Mock agent identity race (process-global PATH/env vars)

`tests/helpers.rs` mock creation helpers (`create_mock_pi`, `create_mock_agent`) set `PATH` or `KNOT_TEST_CLI_PATH` via `std::env::set_var()`, which is process-global. When tests run in parallel (`--test-threads > 1`, the default), one test's PATH modification overwrites another's.

This manifests as:
- Tests picking up the wrong mock binary → unexpected behaviour (success instead of failure, hangs, wrong output)
- Flaky failures that pass under `--test-threads=1`
- `TEST_MUTEX` serialisation in 11 test files as a workaround

### 2. Test suite too slow (~205s total)

| Test | Current duration | Why |
|------|-----------------|-----|
| `process_strand_retry_exhausted_fails` (unit) | 60s+ | 10 retries × 10s delay = 100s worst case |
| `test_session_resume_success` | 60s+ timeout | Mock agent hangs (wrong mock picked up) |
| Multiple pipeline/agent tests | 5-15s each | Full Knot lifecycle per test |

### 3. Composition-heavy tests

Most integration tests spin up the full Knot runtime (notify watching, debounce, state writer, subprocess agent) to test application logic. This conflates two concerns: testing business logic against trait contracts, and testing that adapters wire correctly.

### Current state (2026-07-02)

**475 passed, 1 failed (unit), ~158s total (integration not fully run):**

| Suite | Passed | Failed | Duration |
|-------|--------|--------|----------|
| Unit tests (`lib.rs`) | 475 | **1** | 100.15s |
| `agent_integration` | 25 | 0 | 45s |
| `pipeline` | 30 | 0 | 112s |
| Other suites | ~60 | 0 | ~10s |

The one failing unit test (`runner_passes_event_metadata`) is a mock path collision: two parallel unit tests both write to `/tmp/knot-test-mock-stdio`.

## Target

After this plan:
- **~100 tests total, all fully parallel, no `TEST_MUTEX`**
- **Full test suite under 30s**
- **Three test tiers** — application (mock ports), adapter (real I/O + `tempfile`), smoke (full composition + mock agent via `cli_path`)
- **No process-global env manipulation** — mock helpers return paths, never call `std::env::set_var()`
- **ADR-011** documents the strategy

## Design

See [ADR-011: Hexagonal Test Strategy](../adrs/adr-011-hexagonal-test-strategy.md).

### Test Tiers

| Tier | Count | Strategy | Duration |
|------|-------|----------|----------|
| Application | ~90 | All ports mocked (`TrackingTieOffSink`, `TrackingAgentRunner` etc.) | ~0ms per test |
| Adapter | 8 | One per adapter, real I/O + `tempfile::tempdir()` | ~1s per test |
| Smoke | 2 | Full composition, mock agent via `cli_path` injection | ~5s per test |

## Phases

### Phase 0: Smoke tests and composition wiring

**Goal:** Establish the smoke tests first so they always pass as we strip back integration tests.

- Add `cli_path: Option<PathBuf>` to `AppConfig` and wire it into `build_app_context()`
- Add `with_cli_path()` method to `AppConfig`
- Create `tests/smoke.rs` with two tests: `composition_smoke_stdio` and `composition_smoke_json`
- Each uses `tempfile::tempdir()` + mock agent script + `start_knot()` with `cli_path`
- Verify: both smoke tests pass under parallel execution

### Phase 1: Unit test mock path isolation

**Goal:** Fix the remaining unit test failures from shared `/tmp` paths.

- `make_mock_path()` in `src/adapters/pi_stdio.rs` uses `tempfile::tempdir().join("mock-pi")`
- `make_blocking_mock_path()` same pattern
- `make_json_mock_path()` in `src/adapters/pi_json.rs` same pattern
- `make_json_blocking_mock_path()` same pattern
- Test function keeps `tempdir` handle alive (in scope or stored in runner)
- Verify: `cargo test --lib` passes under default parallel execution

### Phase 2: Adapter test extraction

**Goal:** One test per outbound adapter, verifying the I/O contract in isolation.

- Create `tests/adapters.rs` (or `tests/adapter_<name>.rs` modules)
- `PiStdioAgentRunner` test: subprocess spawn, stdin/stdout capture, exit code, timeout
- `PiJsonAgentRunner` test: JSON-L parsing, session ID extraction, `stopReason` filtering
- `FileSystemTieOffSink` test: write, append, read_content, directory creation
- `FileSystemLoomLog` test: open, JSONL append, read_all
- `FileSystemStateWriter` test: atomic write (`.tmp` + `rename`), valid JSON
- `FileSystemLoomRepository` test: scan rig, parse knot files, save, parse warnings
- `FileSystemAgentProfileRepository` test: profile CRUD from `.md` files
- `NotifyEventSource` test: watch/unwatch, event emission on file changes
- Each adapter test uses `tempfile::tempdir()`, unique mock paths, fully parallel

### Phase 3: Application test migration — core use cases

**Goal:** Migrate `process_strand` integration tests from `start_knot()` to mock-port tests.

- `tests/agent_integration.rs` → rewrite against `TrackingTieOffSink`, `TrackingAgentRunner`, `TrackingLoomLog`
- Remove `start_knot()` calls, `create_mock_pi()` PATH manipulation, `TEST_MUTEX`/`acquire_test_lock()`
- Each test: construct `ProcessStrand` with mocks, call `execute()`, assert on mock state
- Files removed: `tests/agent_integration.rs` (replaced), `acquire_test_lock()` removed
- Verify: all previously-covered scenarios pass, faster execution

### Phase 4: Application test migration — pipeline and file-based tests

**Goal:** Migrate `pipeline.rs` and remaining file-based integration tests.

- `tests/pipeline.rs` → debounce + process flow tests against mock ports
- `tests/tie_off.rs` → tie-off append/read tests against `TrackingTieOffSink`
- `tests/rig_log.rs` → rig-log append tests against `TrackingRigLog`
- `tests/git_versioning.rs` → git commit tests against `TrackingGitVersioningPort`
- Remove `start_knot()` calls, `TEST_MUTEX`, `set_var("PATH")`
- Verify: all scenarios pass, no regressions vs adapter tests

### Phase 5: Application test migration — session resume and edge cases

**Goal:** Migrate session resume and remaining edge case tests.

- `tests/session_resume.rs` → retry loop tests against `TrackingAgentRunner` that fails then succeeds
- Session resume application tests: verify retry count, delay, `--session-id` injection, exhaustion
- Session resume adapter test: verify `PiStdioAgentRunner` surfaces `session_id` in errors
- `tests/profile_timeout.rs` → timeout enforcement against `TrackingAgentRunner`
- `tests/multi_loom.rs` → multi-loom independence (this remains a composition test with 2 looms + mock agents)
- Remove `TEST_MUTEX`, `acquire_test_lock()`
- Verify: session resume tests <5s each

### Phase 6: Composition root and helper cleanup

**Goal:** Remove dead code and consolidate remaining helpers.

- `AppConfig` gains `cli_path` field (used by smoke tests)
- `start_knot()` simplified to smoke-test helper only (no debounce env var overrides needed if smoke tests use fast debounce)
- Remove `TEST_MUTEX` / `acquire_test_lock()` from all remaining files
- Remove `create_mock_pi()`, `create_mock_pi_capturing_stdin()` (replaced by adapter test helpers)
- Remove `derive_tie_off_file()` from `tests/helpers.rs` (moved to domain or removed)
- Remove `wait_for_state_field()`, `wait_for_knot_status_in_state()` etc. (used only by smoke tests, inline them)
- Verify: `cargo test` passes, all files clean

### Phase 7: Verify and record

**Goal:** Full suite verification and documentation.

- Run `cargo test` — target: 0 failures, <30s total
- Run `cargo test --test-threads=4` — verify identical results
- Run `cargo clippy` — verify no new warnings
- Verify `master-plan.md` updated, plan marked complete
- Record final test suite duration and test counts

## Notes

- Phase 0 (smoke tests) must be done first — they prove composition works and prevent us from breaking wiring during the strip-back.
- Phases 3-5 can be done in any order — each migrates a set of integration tests independently.
- `tests/adapter_integration.rs` is replaced by Phase 2's adapter tests and Phase 0's smoke tests.
- `tests/helpers.rs` is heavily reduced by Phase 6 — most helpers become unused.
- ADR-011 documents the strategy; this plan tracks the migration.
