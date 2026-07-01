# Plan: Integration Test Strategy

## Problem

The integration test suite has three interconnected problems that make it unreliable and slow:

### 1. Mock agent identity race (process-global PATH/env vars)

`tests/helpers.rs` mock creation helpers (`create_mock_pi`, `create_mock_agent`) set `PATH` or `KNOT_TEST_CLI_PATH` via `std::env::set_var()`, which is process-global. When tests run in parallel (`--test-threads > 1`, the default), one test's PATH modification overwrites another's. The `PiStdioAgentRunner::resolve_cli_path()` reads the env var once at construction time, but by the time `execute()` runs on a worker thread, another test may have overwritten the env var for its own runner construction.

This manifests as:
- Tests picking up the wrong mock binary → unexpected behaviour (success instead of failure, hangs, wrong output)
- Flaky failures that pass under `--test-threads=1`
- The `acquire_test_lock()` mutex pattern in some test files mitigates this partially but is not consistently applied

### 2. Tie-off path assertions on old nested structure

Plan 052 (Flatten Tie-Off Paths) changed tie-off paths from `rig/tie-offs/{loom-id}/{knot-name}/{knot-name}-tie-off.md` to `rig/tie-offs/{loom-id}/{knot-name}-tie-off.md`. Several integration tests still assert on the old nested paths, causing deterministic failures:

| File | Tests affected |
|------|---------------|
| `tests/agent_integration.rs` | `agent_execution_produces_tie_off`, `agent_execution_append_mode_tie_offs`, `tie_off_contains_agent_output` |
| `tests/pipeline.rs` | `pipeline_processes_strand_create`, `pipeline_ignores_binary_files_and_processes_text_files`, `pipeline_processes_non_md_text_files`, `delete_event_large_tieoff_bounded_context` |

### 3. Test suite too slow (~205s total)

| Test | Current duration | Why |
|------|-----------------|-----|
| `process_strand_retry_exhausted_fails` (unit) | 60s+ | 10 retries × 10s delay = 100s worst case |
| `test_session_resume_success` | 60s+ timeout | Mock agent hangs (wrong mock picked up) |
| `test_session_resume_transparent_on_success` | 60s+ timeout | Same |
| `test_session_resume_delay_between_retries` | ~30s | Retry loop with delays |
| `test_session_resume_exhausted` | ~30s | Exhausts retries |
| Multiple pipeline/agent tests | 5-15s each | Full Knot lifecycle per test |

The session_resume tests are the biggest offenders — they're designed to test retry loops with real delays (10s between retries), which makes them inherently slow. The unit test `process_strand_retry_exhausted_fails` has the same problem.

### Current state (2026-07-01)

**591 passed, 14 failed, ~205s total:**

| Suite | Passed | Failed | Duration |
|-------|--------|--------|----------|
| Unit tests (`lib.rs`) | 476 | 0 | 100.15s |
| `adapter_integration` | 18 | 0 | 15.13s |
| `agent_integration` | 22 | **3** | 45.54s |
| `multi_loom` | 17 | 0 | 5.06s |
| `rig_lifecycle` | 20 | 0 | 0.05s |
| `profile_timeout` | 16 | 0 | 5.04s |
| `pipeline` | 23 | **7** | 35.11s |
| `session_resume` | 19 | **3** | 98.65s |

## Target

After this plan:
- **All integration tests pass under default parallel execution** — mock isolation via per-test CLI path injection (no process-global env vars)
- **Full test suite under 60s** — fast retry parameters in test context, reduced session_resume test count, optimized debounce timing already in place
- **Tie-off path assertions updated** to flat structure
- **No `acquire_test_lock()` needed** — tests can run fully in parallel

## Existing Tests

| Test Class | What it covers | Status |
|------------|---------------|--------|
| `tests/adapter_integration.rs` | JSON + stdio adapter full pipeline | ✅ Green, uses `KNOT_TEST_CLI_PATH` correctly |
| `tests/agent_integration.rs` | Agent execution, tie-offs, failures | ❌ 3 failures (tie-off paths + mock race) |
| `tests/pipeline.rs` | Full pipeline: watch → debounce → process | ❌ 7 failures (tie-off paths + mock race) |
| `tests/session_resume.rs` | Session resume retry protocol | ❌ 3 failures (mock agent hangs) |
| `tests/multi_loom.rs` | Multi-loom independence | ✅ Green |
| `tests/rig_lifecycle.rs` | Rig startup, state file, profiles | ✅ Green, no mock agent |
| `tests/profile_timeout.rs` | Profile timeout enforcement | ✅ Green |
| `tests/tie_off.rs` | Tie-off append mode, context | ✅ Green (uses flat paths) |
| Unit tests `session_resume.rs` | Retry logic with mock ports | ✅ Green, but `retry_exhausted_fails` is 60s+ |
| Unit tests `process_strand.rs` | All processing paths with mock ports | ✅ Green |

## Test Gaps

- No per-test mock agent isolation — all integration tests share process-global PATH/env vars
- Tie-off path assertions not parameterised — each test hardcodes the path string
- Session resume tests use real delays (10s) instead of test-configurable delays
- No test that verifies the mock race condition itself (to prevent regression)

## Phases

### Phase 0: Fix tie-off path assertions

Straightforward find-and-replace: update hardcoded tie-off paths in `tests/agent_integration.rs` and `tests/pipeline.rs` from nested format (`tie-offs/{loom}/{knot}/{knot}-tie-off.md`) to flat format (`tie-offs/{loom}/{knot}-tie-off.md`). Extract a shared helper in `tests/helpers.rs` so future tests derive the path from loom + knot names rather than hardcoding strings.

**Expected result:** 7 tests go from red to green (the pure path-mismatch failures).

### Phase 1: Per-test mock agent isolation

Eliminate process-global `PATH`/`KNOT_TEST_CLI_PATH` manipulation. The `PiStdioAgentRunner` and `PiJsonAgentRunner` already support `with_cli_path()` for direct path injection. The composition root (`server.rs`) creates runners from env var at startup — we need a test-only composition path that accepts a pre-built runner or CLI path.

Two options:
1. **`AppContext` builder pattern** — add `with_agent_runner()` / `with_cli_path()` to a test-only `AppContextBuilder` that overrides the composition root's runner creation
2. **`KNOT_TEST_CLI_PATH` per-runtime** — the runner resolves the env var at construction time. If each test's Knot runs in its own thread with its own `tokio::Runtime` (already the case via `start_knot()`), and we set the env var *before* the thread starts, the runner sees only its own value. The race only exists because `start_knot()` doesn't take a CLI path parameter.

The cleanest approach is: add `cli_path: Option<PathBuf>` to the `start_knot()` helper, pass it through to `AppConfig`, and have the composition root use it when creating the agent runner. Then test helpers (`create_mock_pi`, etc.) return the path instead of setting env vars.

**Expected result:** All tests that currently use `acquire_test_lock()` can drop it. Tests that don't use it yet (and were silently racing) become reliable.

### Phase 2: Fast session resume for tests

The `session_resume.rs` unit test module and `tests/session_resume.rs` integration tests use hardcoded retry delays (10s) and retry counts (10). Add test-only configuration:

- `RETRY_DELAY` and `MAX_RETRIES` read from env vars (`KNOT_TEST_RETRY_DELAY_MS`, `KNOT_TEST_MAX_RETRIES`) with production defaults as fallback
- Unit test `process_strand_retry_exhausted_fails` uses reduced values (e.g. 5 retries × 100ms delay = 500ms instead of 100s)
- Integration tests in `tests/session_resume.rs` use reduced values via profile timeout or env var

**Expected result:** `process_strand_retry_exhausted_fails` drops from 60s+ to <1s. Session resume integration tests drop from 30-60s each to <5s each.

### Phase 3: Consolidate mock helpers and verify full suite

Clean up `tests/helpers.rs`:
- Remove duplicate mock creation functions that set PATH (keep only the `with_cli_path` variants)
- Add a helper that creates a mock + returns the path for `start_knot()` to use
- Ensure all test files use the consolidated helpers

Run the full suite under parallel execution and verify all tests pass within budget.

**Expected result:** 0 failures, <60s total, no `acquire_test_lock()` needed.

## Notes

- The `acquire_test_lock()` pattern already exists in `agent_integration.rs`, `session_resume.rs`, and `profile_timeout.rs`. It's a workaround, not a fix — Phase 1 removes the need for it.
- The `adapter_integration` test file already uses `KNOT_TEST_CLI_PATH` correctly (sets it before starting Knot) and doesn't have a mutex — it's the model for how Phase 1 should work.
- Phase 0 is independent and can be done first to unblock the tie-off path failures immediately.
- The `process_strand.rs` unit tests use mock ports (not real subprocess mocks) and don't have the PATH race problem — only the retry delay is slow.
