# Refactor Plan: Decompose `ProcessStrand::execute()`

## Problem

`process_strand.rs` is **6,429 lines** — ~950 lines of production code and ~5,480 lines of inline tests. The production code itself is dominated by a single `execute()` method of ~300 lines with 6+ levels of nesting. This method handles: strand validation, config resolution, profile loading (twice), prompt building, agent execution, tie-off writing, event dispatch, event enforcement, git commits, and lifecycle logging — all in one flow.

Specific structural issues:

1. **God method** — `execute()` is ~300 lines with deeply nested branching (success/failure paths, event enforcement with follow-up retries). Any change requires reading the full method.
2. **Double profile load** — `resolve_agent_config()` fetches the profile to resolve `AgentConfig`, then the same profile is fetched again immediately after just to read `profile_prompt`.
3. **Triple `StrandEvent` pattern-match** — loom/knot/strand extracted via `extract_event_fields()`, then matched again for `strand_kind`, `event_type`, and `event_label` — four match arms on the same enum.
4. **Duplicate dispatch loop** — the consumer-matching + event-dispatch logic (iterate all looms → knots → `resolve_for_producer` → match `event_id` → dispatch) appears twice: once in `dispatch_agent_events()` and once in the event enforcement follow-up block.
5. **Helper functions without dedicated tests** — `parse_yaml_frontmatter()`, `extract_event_metadata()`, `extract_expected_event_ids()` are tested only indirectly through `execute()` integration paths, not in isolation.

This makes any change to the pipeline expensive — you must read the full file, understand the nesting, and ensure tests still pass across 8 test modules that share fragile fixtures.

## Target

After refactoring:

- `execute()` decomposed into a thin coordinator (~30 lines) calling staged helpers: `validate_strand()`, `resolve_config_and_build()`, `write_tie_off()`, `handle_success()`, `handle_failure()`.
- Profile loaded once, returned from config resolution.
- `StrandEvent` matched once, fields accessed via helper methods.
- Dispatch loop extracted to `dispatch_events_to_consumers()`, used by both primary dispatch and enforcement.
- Extracted helper modules have their own tests alongside the code (inline with Rust convention).
- Tests that verify orchestration flow remain inline in `process_strand.rs`.
- All existing tests pass after every phase. Zero behaviour change.

## ADR Dependencies

- **ADR-010** (Domain Rule Extraction): Accepted — guides which logic stays in the use case (orchestration) vs domain entities (business rules). This plan stays within the application layer — no domain extraction needed.
- **ADR-007** (Stdin-Only Agent Invocation): Accepted — informs how profile_prompt is delivered (via stdin, not CLI args). Profile loading must preserve this.
- **ADR-009** (Agent-Specific Adapters): Accepted — adapter specificity means port traits are used correctly. No impact on this refactor.
- **ADR-011** (Hexagonal Test Strategy): Accepted — tests use mock ports. This refactor preserves the mock-based strategy.

## Existing Tests

| Test Module (inline in `process_strand.rs`) | Count | What it covers |
|-------------|-------|---------------|
| `profile_resolution_tests` | 5 | Profile resolution, CLI args |
| `execution_tests` | 3 | Happy path, timeout, non-timeout error |
| `execution_deleted_tests` | 5 | Deleted event handling, prompt injection, history |
| `session_resume_tests` | 3 | Retry logic, exhausted retries, no-retry |
| `profile_timeout_tests` | 4 | Profile timeout passthrough |
| `git_versioning_tests` | 3 | Git commit on success, skip when disabled, error handling |
| `session_title_tests` | 4 | `--name` flag, title formats, uniqueness |
| `event_dispatch_tests` (Phase 5) | ~9 | Event dispatch, multi-event, no-events |
| `event_metadata_tests` (Phase 6) | ~6 | Event metadata extraction, YAML parsing |
| **Total** | **~38** | **All pipeline paths** |

All tests use mock ports from `test_fixtures.rs` (shared). Integration tests in `tests/` use separate helpers (covered by Plan 61).

## Test Gaps

- **`parse_yaml_frontmatter` bug** — the frontmatter parsing has a logic error (see Phase 4) with no test covering the bare `---` case. A regression test is added during that phase.
- **Event enforcement follow-up path** — the follow-up re-entry in the enforcement block has no dedicated unit test. It is covered only indirectly by integration tests. A targeted test is added during Phase 2 (dispatch loop extraction).

## Progress

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| Phase 0 | ✅ Done | `9bfe8dc` | Extracted 3 helpers + 13 tests to `strand_event_metadata.rs` |
| Phase 1 | ✅ Done | `b0177de` | Extracted `validate_strand()` — returns `Result<bool, PortError>` |
| Phase 2 | ✅ Done | `a0c30aa` | Extracted `dispatch_events_to_consumers()` — shared by both call sites |
| Phase 3 | ✅ Done | `aef7a76` | Single profile load + single `StrandEvent` match |
| Phase 4 | ✅ Done | `bf8a0aa` | Bug fix was done in Phase 0; added 2 regression tests (670 tests total) |
| Phase 5 | ✅ Done | `e68aae5` | Decomposed `execute()` into staged methods |
| Phase 6 | ✅ Done | `8096480` | Extracted 4 helpers + ResolvedExecution to `process_strand_helpers.rs` |
| Phase 7 | ✅ Done | `8096480` | Full verification: 670 tests pass, clippy clean, release build ok |

## Phases

### Phase 0: Extract event metadata helpers into `strand_event_metadata.rs`

The helper functions `extract_event_metadata()`, `parse_yaml_frontmatter()`, and `extract_expected_event_ids()` are self-contained — they don't depend on `ProcessStrand` state and are purely about parsing strand files and scanning knot subscriptions. Extract them into a new module `src/application/usecases/strand_event_metadata.rs`.

The YAML frontmatter parser and event metadata extractor move here with their tests. The `extract_expected_event_ids` function moves here too — it's used by the enforcement logic and the listener context builder.

`process_strand.rs` imports from this module. The inline tests for these helpers in `process_strand.rs` are removed (they become the tests in the new module).

**Module structure:**

```
src/application/usecases/
├── process_strand.rs          (orchestration + orchestration tests)
├── strand_event_metadata.rs   (helpers + helper tests)
├── ...
```

**Verification:** `cargo test --lib` — all existing tests pass. No behavioural change.

### Phase 1: Extract strand validation into `validate_strand()`

Extract the strand file check block (file existence, binary/temp detection, early-exit logging) into a private `validate_strand(&self, event: &StrandEvent, knot: &Knot) -> Result<(), PortError>` method. This covers lines that currently handle `StrandCheckResult` matching with `SkipBinary`, `SkipTemp`, `SkipMissing` — each with its own logging path.

The method returns `Ok(())` for `Proceed`/`ProceedWithWarning` (the warning is logged inline), and returns `Ok(())` after logging for `SkipBinary`/`SkipMissing` (early-exit cases). For clarity, it can return a `ValidateResult` enum instead, but keeping `Result<(), PortError>` is simpler and matches the current early-return pattern.

Tests for this extracted method stay inline in `process_strand.rs` — they are orchestration tests that verify the validation integrates with the pipeline (loom-log entries, skip behaviour).

**Verification:** `cargo test --lib` — all tests pass. Diff shows the validation block replaced by a single method call in `execute()`.

### Phase 2: Extract dispatch loop into `dispatch_events_to_consumers()`

Extract the consumer-matching + event-dispatch loop into a private method `dispatch_events_to_consumers(&self, events: &[AgentEvent], producer_knot: &Knot, loom_id: &LoomId, all_knot_ids: &[&str]) -> Result<Vec<(String, String)>, PortError>`. This loop currently appears in two places: `dispatch_agent_events()` and the event enforcement follow-up block.

Both call sites are replaced with a single method call. The method returns the list of dispatches `(event_id, consumer_loom_id)` for logging.

While extracting, add a dedicated unit test for the event enforcement follow-up path (currently uncovered) in the inline test module.

**Verification:** `cargo test --lib` — all tests pass. Diff shows two loops replaced by two calls to the same method.

### Phase 3: Fix double profile load + deduplicate `StrandEvent` matching

Two smaller but mechanical improvements:

**A. Single profile load** — Change `resolve_agent_config()` to return `(AgentConfig, Option<Duration>, AgentProfile)` so the caller has the profile without reloading it. Update the caller to use the returned profile directly for `profile_prompt`.

**B. Deduplicate `StrandEvent` matching** — Replace the four separate pattern-matches on `StrandEvent` (in `extract_event_fields()`, `strand_kind`, `event_type`, `event_label`) with a single match that extracts all fields at once, or add helper methods on `StrandEvent`: `fields()`, `kind_label()`, `to_event_type()`.

Tests stay inline. No test changes expected — these are internal refactorings with the same observable behaviour.

**Verification:** `cargo test --lib` — all tests pass. Profile loaded exactly once per execution. `StrandEvent` matched exactly once.

### Phase 4: Fix `parse_yaml_frontmatter` bug + add regression test

The frontmatter parser (now in `strand_event_metadata.rs` from Phase 0) has an inverted condition:

```rust
if !trimmed.starts_with("---\n") && !trimmed.starts_with("---\r\n")
    && trimmed == "---"
```

Due to `&&` precedence, the `trimmed == "---"` check always short-circuits. The intent is to accept bare `---` as a valid frontmatter delimiter, but the logic is broken. Fix the condition and add regression tests for: bare `---` frontmatter, content without frontmatter, and empty frontmatter.

Tests live inline in `strand_event_metadata.rs`.

**Verification:** `cargo test --lib` — all tests pass including the new regression tests.

### Phase 5: Decompose `execute()` into staged methods

The largest structural change. Decompose `execute()` into a thin coordinator that delegates to private methods:

```rust
pub fn execute(&self, event: StrandEvent) -> Result<(), PortError> {
    let (loom_id, knot_id, strand_path) = Self::extract_event_fields(&event);
    let loom = self.lookup_loom(&loom_id)?;
    let knot = self.lookup_knot(&loom, &knot_id)?;

    self.validate_strand(&event, &knot)?;         // Phase 1
    let tie_off_path = self.compute_tie_off_path(&loom, &knot, &strand_path);

    self.log_port.append(LoomEvent::KnotProcessing { ... })?;

    let resolved = self.resolve_config_and_build(loom_id, knot, &strand_path, &event)?;
    // resolve_config_and_build() covers: profile load, prompt building,
    // listener context, agent execution, outcome derivation

    let outcome = resolved.outcome;
    if outcome.should_write_tie_off() {
        self.write_tie_off(&outcome, knot, &tie_off_path, &strand_path)?;
    }

    if outcome.is_timeout() {
        self.log_timeout(&outcome, &loom_id, &knot_id, &strand_path)?;
    }

    match outcome.tie_off_status() {
        Some(TieOffStatus::Produced) => {
            self.handle_success(&outcome, knot, &loom_id, &knot_id, &strand_path, &tie_off_path)?;
        }
        _ => {
            self.handle_failure(&outcome, &loom_id, &knot_id, &strand_path)?;
        }
    }

    Ok(())
}
```

Where:
- `resolve_config_and_build()` — profile resolution, prompt building, listener context, agent execution, outcome derivation. Returns a `ResolvedExecution` struct.
- `write_tie_off()` — constructs `TieOff` from outcome and writes it.
- `handle_success()` — event dispatch, KnotCompleted log, StrandProcessed log, event enforcement, git commit.
- `handle_failure()` — KnotFailed log, StrandProcessed log with error.

The key structural change: the 6+ level nesting in the success path (event dispatch → enforcement → git → logging) is flattened into a linear sequence in `handle_success()`.

All existing inline test modules remain in `process_strand.rs` — they test the orchestration flow end-to-end. The decomposition makes them more resilient (they test the pipeline, not implementation details of the monolithic method).

**Verification:** `cargo test --lib` — all tests pass. `execute()` is ~30 lines of orchestration.

### Phase 6: Extract helpers to `process_strand_helpers.rs`

Move the four helper methods (`resolve_config_and_build()`, `write_tie_off()`, `handle_failure()`, `handle_success()`) and the `ResolvedExecution` struct from the `ProcessStrand` impl into free functions in a new module `src/application/usecases/process_strand_helpers.rs`.

Each helper becomes a free function accepting `&ProcessStrand` as the first parameter (plus its existing arguments). Since `handle_success()` calls `self.dispatch_agent_events()`, `self.resolve_agent_config()`, and `self.dispatch_events_to_consumers()`, and `resolve_config_and_build()` calls `self.resolve_agent_config()` and `Self::collect_all_knots()`, these methods need to be made `pub(crate)` in `ProcessStrand`.

All existing inline test modules remain in `process_strand.rs` — they exercise the helpers through `execute()` end-to-end.

`execute()` is updated to call the free functions: `helpers::resolve_config_and_build(self, ...)`, etc.

**Module structure after Phase 6:**

```
src/application/usecases/
├── process_strand.rs          (orchestration + orchestration tests)
├── process_strand_helpers.rs  (helpers + ResolvedExecution struct)
├── strand_event_metadata.rs   (metadata helpers + tests)
├── ...
```

**Verification:** `cargo test --lib` — all tests pass. `process_strand.rs` production code reduced to ~200 lines.

### Phase 7: Final verification

Full test suite, clippy, and build verification:

- `cargo test --lib` — all unit tests pass
- `cargo test` — all integration tests pass
- `cargo clippy --lib` — no new warnings
- `cargo build --release` — clean build
- Confirm `process_strand.rs` is ~300-400 lines of production code + ~5,480 lines of inline tests
- Confirm `strand_event_metadata.rs` exists with extracted helpers and their tests

## Notes

## Implementation Status: ✅ Complete (2026-07-16)

## Notes
- Phase 6 added `process_strand_helpers.rs` (399 lines) containing 4 extracted helpers + `ResolvedExecution` struct
- Phase 5 decomposed `execute()` into staged methods; Phase 6 extracted them to standalone module
- Full test suite passes (670 tests, 0 failures)
- Version bumped to 0.28.2
- **Dependency on Plan 61:** Plan 61 (consolidate `build_process_strand` integration test helpers) is a separate concern — it covers integration test helpers in `tests/`. This plan covers the use case module. They can proceed independently.
- **ADR-010 already did domain extraction:** ADR-010 extracted domain rules (`should_process`, `resolve_for_knot`, `deleted_prompt`, `TieOffOutcome::derive`) from the use case. This plan works with the already-extracted domain layer — it does not extract further domain rules. The remaining code in `execute()` is orchestration (coordinating ports and domain results), not business rules.
- **No behavioural changes:** Every phase is purely structural. Tests that pass before a phase must pass after it. If any test fails, the phase is incorrect.
- **Empty directory:** `project/plans/055-process-strand-refactor/` was removed (never filled in).
