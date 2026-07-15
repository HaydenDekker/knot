# Refactor Plan: Consolidate `build_process_strand` into Shared Test Helper

## Problem

There are **8 duplicate `fn build_process_strand` helpers** across the integration test files, each a local function. They share ~90% identical body logic (registering a loom, creating mock ports, wiring `ProcessStrand::new`) but diverge in small ways:

| File | Params | Extra Return Values |
|------|--------|---------------------|
| `tie_off.rs` | `(loom, runner)` | none |
| `rig_log.rs` | `(loom, runner)` | none |
| `agent_integration.rs` | `(loom, runner)` | none |
| `profile_timeout.rs` | `(loom, runner, profile)` | none (custom profile) |
| `session_resume.rs` | `(loom, runner, profile)` | none (custom profile) |
| `git_versioning.rs` | `(loom, runner)` | `Arc<MockGitVersioningPort>`, `Arc<Mutex<Vec<git_commits>>>` |
| `event_enforcement.rs` | `(Vec<Loom>, runner)` | `Arc<MockEventDispatcher>` |
| `pipeline.rs` | `(loom, runner)` | `Arc<MockGitVersioningPort>`, `Arc<MockStrandFileChecker>` |

Every copy repeats the same `ProcessStrand::new()` call with 12 arguments. If the `ProcessStrand` constructor signature changes, all 8 copies must be updated. Each copy is a maintenance burden and a regression risk.

## Target

A single shared builder in `tests/helpers.rs` (or a new `tests/strand_builder.rs`) that:

1. **Encapsulates the common setup** — loom registration, mock port creation, profile repository wiring, `ProcessStrand::new()` call.
2. **Supports all current variations** via optional setters:
   - Custom profile (used by `profile_timeout.rs`, `session_resume.rs`)
   - Multiple looms (used by `event_enforcement.rs`)
   - Tracking `MockGitVersioningPort` (used by `git_versioning.rs`, `pipeline.rs`)
   - Tracking `MockEventDispatcher` (used by `event_enforcement.rs`)
   - Tracking `MockStrandFileChecker` (used by `pipeline.rs`)
3. **Returns a typed result struct** — not an ad-hoc tuple — so callers pick only what they need:
   ```rust
   pub struct ProcessStrandResult {
       pub strand: ProcessStrand,
       pub log_events: Arc<Mutex<Vec<LoomEvent>>>,
       pub tie_off_appends: Arc<Mutex<Vec<TieOff>>>,
       pub rig_events: Arc<Mutex<Vec<RigLogEvent>>>,
       pub tie_off_content: Arc<Mutex<HashMap<String, String>>>,
       pub agent_runner: Arc<MockAgentRunner>,
       pub git_port: Option<Arc<MockGitVersioningPort>>,
       pub git_commits: Option<Arc<Mutex<Vec<...>>>>,
       pub file_checker: Option<Arc<MockStrandFileChecker>>,
       pub event_dispatcher: Option<Arc<MockEventDispatcher>>,
   }
   ```
4. **Eliminates all 8 local functions** — each test file just imports the builder and calls it.

## Existing Tests

| Test File | Tests | Coverage |
|-----------|-------|----------|
| `tie_off.rs` | 8 tests | Tie-off path, append-mode, section formatting |
| `profile_timeout.rs` | 6 tests | Profile-level timeout handling |
| `rig_log.rs` | 4 tests | Rig-log event recording |
| `agent_integration.rs` | 11 tests | Agent execution, tie-off, error handling |
| `session_resume.rs` | 8 tests | Session-resume retry logic |
| `git_versioning.rs` | 6 tests | Git commits after processing |
| `event_enforcement.rs` | 9 tests | Event enforcement pipeline |
| `pipeline.rs` | 12 tests | Full strand-processing pipeline |

All 8 files use `use knot::application::usecases::test_fixtures::*;` to import shared mocks. The builder will live in `tests/helpers.rs` (already used by 3 files via `mod helpers;`) or a dedicated `tests/strand_builder.rs`.

## Test Gaps

None — this is a pure refactor. All existing tests must pass before and after. The builder itself gets no new tests (the behaviour it wraps is tested by the 64 existing integration tests).

## Phases

### Phase 0: Create `ProcessStrandBuilder` in `tests/helpers.rs`

- Define `ProcessStrandBuilder` with builder-pattern setters
- Define `ProcessStrandResult` struct with all possible fields
- Implement the common setup: loom registration, mock ports, profile repo, `ProcessStrand::new()`
- Use `default_profile()` for the default profile; support `with_profile(AgentProfile)` override
- Support single loom by default; `with_looms(Vec<Loom>)` for multi-loom
- Support optional tracking ports via `with_tracking_git()`, `with_tracking_event_dispatcher()`, `with_tracking_file_checker()`
- Add a `build()` method returning `ProcessStrandResult`
- Verify: `cargo test` — all existing tests still pass (no callers yet)

### Phase 1: Replace local `build_process_strand` in all 8 test files

- Update each test file to use the shared builder
- `tie_off.rs`, `rig_log.rs`, `agent_integration.rs` — simplest cases, just `builder(loom, runner).build()`
- `profile_timeout.rs`, `session_resume.rs` — add `.with_profile(custom_profile)`
- `git_versioning.rs` — add `.with_tracking_git()`
- `event_enforcement.rs` — add `.with_looms(vec![...])` + `.with_tracking_event_dispatcher()`
- `pipeline.rs` — add `.with_tracking_git()` + `.with_tracking_file_checker()`
- Remove all 8 local `fn build_process_strand` definitions
- Verify: `cargo test` — all 64 integration tests still pass, clippy clean

## Notes

- The `tests/helpers.rs` file already exists and is used by 3 test files (`discovery.rs`, `multi_loom.rs`, `smoke.rs`). The builder can be added there since it's already the designated shared test helper module.
- A `ProcessStrandResult` struct (not a tuple) makes the API self-documenting and allows optional fields. Callers destructure with `let result = builder(...).build(); let ProcessStrandResult { strand, log_events, .. } = result;`
- The builder should NOT be generic over port types — it always uses the same mock types. If a test needs a different mock, it can construct `ProcessStrand` directly (rare case, no current user).
- This refactor does NOT change any test behaviour or assertions — it is purely structural.
