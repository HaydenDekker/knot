# Plan: Split `usecases.rs` God Class into Isolated Modules

## Problem

`src/application/usecases.rs` is a single 8,725-line file containing 12 use cases and 14 test modules (87 tests). The file has grown through incremental feature additions without structural organisation. This makes:

- Any change requiring a read of the full file expensive (context window, compile time)
- Test mocks and helpers are **duplicated** across 6+ test modules (`TrackingEventSource`, `MockLoomLogPort`, `MockLoomRepository`, `build_knot`, `build_loom`)
- `ensure_strand_dir_and_watch()` is copy-pasted across `DiscoverLooms`, `RegisterLoom`, and `ConfigEventHandler`
- No clear boundary between unrelated concerns (loom CRUD, strand processing, config event handling, state writing)

## Target

`src/application/usecases/` becomes a directory with:

```
src/application/
  usecases/
    mod.rs                  — re-exports all public types
    types.rs                — LoomSummary, KnotStatus, format_timestamp
    test_fixtures.rs        — shared mock impls and domain builders (cfg(test))
    loom/
      mod.rs                — re-exports
      discover.rs           — DiscoverLooms
      reload.rs             — ReloadConfig
      register.rs           — RegisterLoom
      unregister.rs         — UnregisterLoom
      mod_watchers.rs       — ensure_strand_dir_and_watch() shared helper
    query/
      mod.rs                — re-exports
      list_looms.rs         — ListLooms
      get_loom.rs           — GetLoom
      get_activity.rs       — GetLoomActivity
      get_knot_status.rs    — GetKnotStatus
    process_strand.rs       — ProcessStrand (largest use case + all its tests)
    config_event_handler.rs — ConfigEventHandler + all its tests
    manage_knot.rs          — ManageKnot + KnotAction enum + tests
    write_state.rs          — WriteState + tests
```

**Module grouping rationale** (by dependency profile and domain responsibility):

| Group | Use Cases | Shared Dependencies |
|-------|-----------|---------------------|
| `loom/` | DiscoverLooms, ReloadConfig, RegisterLoom, UnregisterLoom | `LoomRepository`, `LoomLogPort`, `LoomStore`, `EventSource` |
| `query/` | ListLooms, GetLoom, GetLoomActivity, GetKnotStatus | `LoomStore` (± `LoomLogPort`) |
| standalone | ProcessStrand, ConfigEventHandler, ManageKnot, WriteState | Each has unique dependency profile |

**Shared test fixtures to extract** (currently duplicated 2-6 times across test modules):

| Fixture | Current locations | Target |
|---------|-------------------|--------|
| `TrackingEventSource` | `config_handler_tests`, `phase2_tests` | `test_fixtures.rs` |
| `MockLoomLogPort` (recorded) | `config_handler_tests`, `phase3_tests`, `phase4_tests`, `phase6_timeout_tests`, `phase7_timeout_resolution_tests`, `write_state_tests` | `test_fixtures.rs` |
| `MockLoomRepository` | `config_handler_tests`, `phase2_tests` | `test_fixtures.rs` |
| `MockAgentRunner` (configurable) | `phase3_tests`, `phase3_profile_resolution_tests`, `phase4_tests`, `phase6_timeout_tests`, `phase7_timeout_resolution_tests`, `phase8_git_versioning_tests`, `phase9_session_title_tests` | `test_fixtures.rs` |
| `MockTieOffSink` | `phase4_tests`, `phase6_timeout_tests`, `phase7_timeout_resolution_tests` | `test_fixtures.rs` |
| `MockProfileRepo` | `phase3_profile_resolution_tests` | `test_fixtures.rs` |
| `MockGitVersioningPort` | `phase8_git_versioning_tests` | `test_fixtures.rs` |
| `build_knot()` / `build_knot_with_strand_dir()` | `config_handler_tests`, `phase2_tests`, `phase3_tests`, `phase4_tests`, `manage_knot_tests`, `reload_config_tests` | `test_fixtures.rs` |
| `build_loom()` | `config_handler_tests`, `phase2_tests`, `phase3_tests`, `phase4_tests`, `manage_knot_tests`, `reload_config_tests` | `test_fixtures.rs` |

Note: The mock implementations in `ports.rs` tests stay where they are — those are contract tests for the port traits themselves, not usecase test fixtures.

## Implementation Status: ⬜ Draft

## Existing Tests

| Test Module | What it covers | Lines |
|-------------|---------------|-------|
| `config_handler_tests` (line 1802) | ConfigEventHandler all event types | ~1,195 |
| `phase2_tests` (line 2997) | RegisterLoom watchers, ConfigEventHandler targeted scan | ~368 |
| `phase3_tests` (line 3365) | ProcessStrand basic pipeline | ~205 |
| `phase4_tests` (line 3570) | ProcessStrand tie-off write + error handling | ~232 |
| `manage_knot_tests` (line 3802) | ManageKnot CRUD | ~252 |
| `phase3_profile_resolution_tests` (line 4054) | ProcessStrand profile resolution | ~436 |
| `phase6_timeout_tests` (line 4490) | ProcessStrand timeout handling | ~1,040 |
| `phase7_timeout_resolution_tests` (line 5530) | ProcessStrand timeout + profile timeout | ~473 |
| `phase8_git_versioning_tests` (line 6003) | ProcessStrand git versioning | ~336 |
| `reload_config_tests` (line 6339) | ReloadConfig | ~250 |
| `phase9_session_title_tests` (line 6589) | ProcessStrand session title / --name | ~734 |
| `write_state_tests` (line 7283) | WriteState build + write | ~471 |
| `phase2_text_check_tests` (line 7754) | ProcessStrand text file check | ~471 |
| `phase2_file_existence_tests` (line 8225) | ProcessStrand missing file handling | ~499 |

**Total:** 87 tests across 14 modules, ~6,500 lines of test code.

## Test Gaps

No new tests needed — this is a structural refactor. All existing tests must continue to pass. The test is `cargo test` and `cargo clippy` on the final state.

## Phases

### Phase 0: Skeleton — `types.rs`, `mod.rs`, `test_fixtures.rs`

**Approach per phase:** Each extraction follows the same cycle: create new file(s) with the use case → remove from `usecases.rs` → add re-export in `mod.rs` → `cargo test` → next use case. This validates every step immediately rather than accumulating changes.

**Goal:** Create the new directory structure with `types.rs` (shared types), `mod.rs` (re-exports), and `test_fixtures.rs` (shared mocks). Verify skeleton compiles with all code still in `usecases.rs`.

- [ ] Create `src/application/usecases/` directory
- [ ] Create `usecases/mod.rs` — `pub mod` declarations for all sub-modules + `pub use` re-exports for all public types (maintains backward compat for `application::usecases::X`)
- [ ] Create `usecases/types.rs` — extract `LoomSummary`, `KnotStatus`, `format_timestamp()` from `usecases.rs`
- [ ] Create `usecases/test_fixtures.rs` (`#[cfg(test)]`) — extract shared fixtures:
  - `TrackingEventSource` (full variant from `config_handler_tests` with `watch`, `unwatch`, `set_loom_ids` tracking)
  - `MockLoomLogPort` (recorded events variant with `Arc<Mutex<Vec<LoomEvent>>>`)
  - `MockLoomRepository` (with `scan_looms`, `scan_warnings`, `scan_knots`)
  - `MockAgentRunner` (configurable output variant with `Arc<RwLock<...>>`)
  - `MockTieOffSink` (content-recording variant)
  - `MockProfileRepo` (profiles map variant)
  - `MockGitVersioningPort` (commits-recording variant)
  - `build_knot()`, `build_knot_with_strand_dir()`, `build_loom()` domain builders
- [ ] Convert `usecases.rs` → `usecases/_all.rs` (temporary, rename from `.rs` to `.rs` by moving content)
  - Actually: keep `usecases.rs` for now, add `mod.rs` that re-exports from it. The plan is to **migrate** code out of `usecases.rs` in later phases.
- [ ] Update `src/application/mod.rs`: change `pub mod usecases;` to reference the directory's `mod.rs` (directory-style module)
- [ ] **Verify:** `cargo test` — all 87+ existing tests still pass (no code moved yet, just structure)

### Phase 1: Extract shared test fixtures ✅ DONE

**Goal:** Move duplicated mock implementations from individual test modules into `test_fixtures.rs`. Each test module that had its own copy switches to `use super::super::test_fixtures::*`.

- [x] In each test module that defines `TrackingEventSource`: remove local definition, add `use super::super::test_fixtures::TrackingEventSource`
- [x] In each test module that defines `MockLoomLogPort`: remove local definition, add `use super::super::test_fixtures::MockLoomLogPort`
- [x] In each test module that defines `MockLoomRepository`: remove local definition, add `use super::super::test_fixtures::MockLoomRepository`
- [x] In each test module that defines `MockAgentRunner` (configurable): remove local, import from fixtures
- [x] In each test module that defines `MockTieOffSink`: remove local, import from fixtures
- [x] In each test module that defines `build_knot` / `build_loom`: remove local, import from fixtures
- [x] Handle variant differences: some test modules use simplified mocks (e.g. `MockLoomLogPort` with no-op `append` vs. recorded). Where a test module used a simplified variant that is semantically different, keep the local variant or parameterise the shared one.
- [x] **Verify:** `cargo test` — all 86 tests still pass

**Variants kept local (semantically different from shared):**

| Module | Local Fixture | Reason |
|--------|---------------|--------|
| `phase2_tests` | `MockLoomRepository` | Has `scan_error` field for error injection |
| `phase4_tests` | `MockLoomRepository` | Simplified `Vec<Loom>` (no Arc/Mutex) |
| `reload_config_tests` | `MockLoomRepository` | Simplified `Vec<Loom>` (no Arc/Mutex) |
| `phase3_profile_resolution_tests` | `MockAgentRunner` | Simplified, no context capture |
| `phase6_timeout_tests` | `TrackingTieOffSink` | Tracks `appends` list (shared doesn't) |
| `phase7_timeout_resolution_tests` | `TrackingAgentRunner` | Returns contexts via tuple (shared uses method) |
| `phase8_git_versioning_tests` | `MockProfileRepository` | Simplified `HashMap` (no Arc/Mutex) |
| `manage_knot_tests` | `build_knot` | Uses `"default"` profile (shared uses `"fast"`) |
| `phase6/7/8/9_timeout_*` | `build_knot(id, profile)` | Takes profile parameter (shared doesn't) |

### Phase 2: Extract `loom/` module (DiscoverLooms, ReloadConfig, RegisterLoom, UnregisterLoom) ✅ DONE

**Goal:** Move loom CRUD + discovery use cases into `usecases/loom/` subdirectory. Extract shared `ensure_strand_dir_and_watch()` helper.

**Extract `ensure_strand_dir_and_watch` first:**
- [x] Create `usecases/loom/mod_watchers.rs` — extract `ensure_strand_dir_and_watch()` as a standalone `pub(crate)` function taking `(loom_id, knot_id, strand_dir, log_port, event_source)`. This eliminates the copy-paste across `DiscoverLooms`, `RegisterLoom`, and `ConfigEventHandler`.
- [x] Create `usecases/loom/mod.rs` — `pub mod mod_watchers` + `pub use mod_watchers::ensure_strand_dir_and_watch`.
- [x] In `usecases.rs`: replace each `ensure_strand_dir_and_watch` method call with `super::loom::ensure_strand_dir_and_watch(...)`. Remove the method from each use case struct.
- [x] **Verify:** `cargo test` — all tests pass before moving any use case structs.

**Then extract each use case one at a time (create file → remove from usecases.rs → re-export → test):**
- [x] Create `usecases/loom/discover.rs` — move `DiscoverLooms` + tests. Re-export in `loom/mod.rs` and `usecases/mod.rs`. Remove from `usecases.rs`. → **`cargo test`**
- [x] Create `usecases/loom/register.rs` — move `RegisterLoom` + tests. Re-export. Remove from `usecases.rs`. → **`cargo test`**
- [x] Create `usecases/loom/unregister.rs` — move `UnregisterLoom` + tests. Re-export. Remove from `usecases.rs`. → **`cargo test`**
- [x] Create `usecases/loom/reload.rs` — move `ReloadConfig` + tests. Depends on `DiscoverLooms` (import from sibling). Re-export. Remove from `usecases.rs`. → **`cargo test`**

### Phase 3: Extract `query/` module (ListLooms, GetLoom, GetLoomActivity, GetKnotStatus) ✅ DONE

**Goal:** Move read-only query use cases into `usecases/query/` subdirectory. Each extracted individually (create file → remove from usecases.rs → re-export → test).

- [x] Create `usecases/query/mod.rs` (skeleton). Re-export in `usecases/mod.rs`.
- [x] Create `usecases/query/list_looms.rs` — move `ListLooms`. Re-export in `query/mod.rs` and `usecases/mod.rs`. Remove from `all.rs`. → **`cargo test`**
- [x] Create `usecases/query/get_loom.rs` — move `GetLoom`. Re-export. Remove from `all.rs`. → **`cargo test`**
- [x] Create `usecases/query/get_activity.rs` — move `GetLoomActivity`. Re-export. Remove from `all.rs`. → **`cargo test`**
- [x] Create `usecases/query/get_knot_status.rs` — move `GetKnotStatus`. (`KnotStatus` already in `types.rs` from Phase 0). Re-export. Remove from `all.rs`. → **`cargo test`**

**Note:** These query use cases have no dedicated unit test modules — they are simple read-through operations tested via the HTTP/integration layer.

### Phase 4: Extract standalone use cases ✅ DONE

**Goal:** Move remaining use cases into flat files under `usecases/`. Each file owns its use case + all related tests. Each extracted individually (create file → remove from usecases.rs → re-export → test).

- [x] Create `usecases/manage_knot.rs` — move `ManageKnot`, `KnotAction` + `manage_knot_tests`. Re-export. Remove from `all.rs`. → **`cargo test`**
- [x] Create `usecases/write_state.rs` — move `WriteState` + `write_state_tests`. Re-export. Remove from `all.rs`. → **`cargo test`**
- [x] Create `usecases/config_event_handler.rs` — move `ConfigEventHandler` + `config_handler_tests` + `phase2_tests`. Uses `super::loom::ensure_strand_dir_and_watch` (from Phase 2). Re-export. Remove from `all.rs`. → **`cargo test`**
- [x] Create `usecases/process_strand.rs` — move `ProcessStrand` + all its test modules:
  - `phase3_tests`, `phase4_tests`, `phase3_profile_resolution_tests`
  - `phase6_timeout_tests`, `phase7_timeout_resolution_tests`
  - `phase8_git_versioning_tests`, `phase9_session_title_tests`
  - `phase2_text_check_tests`, `phase2_file_existence_tests`
  - Re-export. Remove from `all.rs`. → **`cargo test`**

**Verification:** `cargo build` (clean, only pre-existing warnings), `cargo test --lib` (436 tests pass), `cargo clippy` (no new warnings)

### Phase 5: Remove old `usecases.rs`, finalise `mod.rs`, verify server wiring ✅ DONE

**Goal:** Delete the now-empty `usecases.rs`, ensure `mod.rs` re-exports everything, and verify `server.rs` imports still compile.

- [x] Delete `src/application/usecases.rs` (already removed; removed leftover `all.rs` stub with stale imports)
- [x] Removed `mod all;` from `usecases/mod.rs`
- [x] Verified `usecases/mod.rs` re-exports all public types (13 use case types + 3 shared types)
- [x] Verified `server.rs` imports still compile (5 import sites all resolve via `mod.rs` re-exports)
- [x] `cargo build` — compilation succeeds (pre-existing warnings only)
- [x] `cargo test` — all 436 tests pass (1 flaky `runner_passes_name_flag_through_cli_args` due to E26 "Text file busy", passes on retry)
- [x] `cargo clippy` — no new warnings (34 pre-existing)

### Phase 6: Extract `StrandFilePolicy` into domain layer ✅ DONE

**Problem:** `ProcessStrand::execute()` contains ~80 lines of text-check/temp-file/missing-file logic that decides whether a strand should be processed. This is a **domain rule** (what counts as a valid strand file) embedded in application orchestration. It calls `content_inspector::is_text_file()` (adapter) and `temp_file::is_known_temp_file()` (domain), mixing concerns.

**Target:** Move the strand validity decision into `StrandPath.should_process()` as a domain method that returns `Result<StrandCheckResult, StrandCheckError>` where the result is an enum: `Proceed`, `SkipBinary`, `SkipTemp`, `SkipMissing`.

**Changes:**
- [x] Added `StrandCheckResult` enum and `StrandCheckError` to `src/domain/entities.rs` (on `StrandPath`)
  - `StrandCheckResult::{Proceed, SkipBinary, SkipTemp, SkipMissing, ProceedWithWarning}`
  - `StrandCheckError { message: String }`
- [x] Added `StrandPath::should_process(is_deleted, &dyn StrandFileChecker)` domain method:
  - Calls `StrandFileChecker::is_text_file()` via port trait (not adapter-coupled)
  - Calls `temp_file::is_known_temp_file()` (already in domain)
  - Returns structured result instead of side-effecting logs
- [x] Defined `StrandFileChecker` trait in `src/domain/entities.rs` (domain-level port trait with `Send + Sync`):
  - `fn is_text_file(&self, path: &Path) -> Result<bool, std::io::Error>`
- [x] In `ProcessStrand::execute()`: replaced the 80-line text-check block with `strand_path.should_process()` + match on `StrandCheckResult`
- [x] Added `file_checker: Arc<dyn StrandFileChecker>` field to `ProcessStrand` struct
- [x] Added `PortError::StrandCheckFailed(String)` variant
- [x] Created `src/adapters/outbound/strand_file_checker.rs` — `ContentInspectorChecker` adapter implementing `StrandFileChecker`
- [x] Added `MockStrandFileChecker` to `test_fixtures.rs` (configurable for text/binary/error paths)
- [x] Added 7 domain tests in `entities.rs` covering all `StrandCheckResult` variants
- [x] Updated `server.rs` to wire `ContentInspectorChecker` into `ProcessStrand`
- [x] `phase2_text_check_tests` uses real `ContentInspectorChecker` for pipeline validation
- [x] **Verify:** `cargo test` — all 448 tests pass (lib + integration). Domain tests cover all `StrandCheckResult` variants.
- [x] **Verify:** `cargo clippy` — no new warnings

**Note:** The `StrandFileChecker` trait was placed in `src/domain/entities.rs` (not `ports.rs`) because the domain method `StrandPath::should_process()` calls it directly. Keeping it in the domain layer avoids a circular dependency (application/ports → domain → application/ports).

### Phase 7: Extract `Knot::deleted_prompt()` into domain layer ✅ DONE

**Problem:** `ProcessStrand::execute()` contains ~70 lines that build a special prompt for Deleted events: reads tie-off history, extracts last N entries, and composes a prompt with deletion notice + scoped history. This is **Knot business logic** — how a Knot represents its own deletion context.

**Target:** Move into `impl Knot` as `pub fn deleted_prompt(&self, filename: &str, sections: &[TieOffSection]) -> String`.

**Changes:**
- [x] Add `impl Knot` method `deleted_prompt(filename: &str, sections: &[TieOffSection]) -> String` in `src/domain/entities.rs`
  - Takes the deletion notice string as a const or parameter
  - Formats the scoped strand history using `TieOffSection` fields
  - Returns composed prompt string
- [x] In `ProcessStrand::execute()`: replace the 70-line prompt-building block with:
  ```rust
  let prompt = if is_deleted {
      knot.deleted_prompt(&strand_filename, &sections)
  } else {
      knot.prompt_template.instructions.clone()
  };
  ```
- [x] Added 3 domain tests in `entities.rs` covering prompt composition: notice-only, with history, and empty-body sections
- [x] **Verify:** `cargo test` — all 451 tests pass (448 existing + 3 new). Domain tests verify prompt composition without any port mocks.
- [x] **Verify:** `cargo clippy` — no new warnings

### Phase 8: Extract `TieOffOutcome` derivation into domain layer

**Problem:** `ProcessStrand::execute()` has 3-way branching for tie-off writing based on processing result:
- Success → `TieOffStatus::Produced`, write to tie-off
- Non-timeout error → `TieOffStatus::Failed`, write to tie-off
- Timeout error → **skip** tie-off write, write to rig-log instead

This is a **domain rule** about what outcomes mean, expressed as imperative branching in the use case.

**Target:** Define a `TieOffOutcome` enum in domain that captures the three cases, with a derivation method.

**Changes:**
- [ ] Add to `src/domain/entities.rs`:
  ```rust
  pub enum TieOffOutcome {
      Produced { content: String },
      Failed { error: String },
      TimeoutSkipped { error: String },
  }
  ```
- [ ] Add `impl TieOffOutcome`:
  ```rust
  pub fn derive(result: Result<AgentOutput, PortError>) -> Self { ... }
  pub fn tie_off_status(&self) -> TieOffStatus { ... }
  pub fn tie_off_content(&self) -> Option<&str> { ... }
  ```
- [ ] In `ProcessStrand::execute()`: replace the 3-way match with:
  ```rust
  let outcome = TieOffOutcome::derive(result);
  // Write tie-off (skip for TimeoutSkipped)
  // Write loom-log (KnotCompleted vs KnotFailed)
  // Write rig-log for timeout
  ```
- [ ] Move timeout-related tests from `process_strand.rs` (`phase6_timeout_tests`) to `src/domain/entities.rs` tests for the derivation logic
- [ ] **Verify:** `cargo test` — all tests pass. Domain tests cover all three outcome variants.

### Phase 9: Extract `AgentProfile::resolve_for_knot()` into domain layer

**Problem:** `ProcessStrand::resolve_agent_config()` loads a profile and builds an `AgentConfig` from it. The mapping (profile fields → agent config fields) is **domain composition logic** buried in the use case.

**Target:** Move into `impl AgentProfile` as `pub fn resolve_for_knot(&self, knot: &Knot) -> AgentConfig`.

**Changes:**
- [ ] Add to `src/domain/value_objects.rs`:
  ```rust
  impl AgentProfile {
      /// Build an `AgentConfig` by merging this profile's fields
      /// with the knot's prompt instructions.
      pub fn resolve_for_knot(&self, knot: &Knot) -> AgentConfig {
          AgentConfig {
              goal: knot.prompt_template.instructions.clone(),
              provider: self.provider.clone(),
              model: self.model.clone(),
              tools: self.tools.clone(),
              extra_args: Vec::new(),
          }
      }

      /// Return the profile's session timeout as a Duration.
      pub fn session_timeout(&self) -> Option<std::time::Duration> {
          self.timeout.map(std::time::Duration::from_secs)
      }
  }
  ```
- [ ] In `ProcessStrand::resolve_agent_config()`: replace the manual field mapping with:
  ```rust
  let profile = self.profile_repo.get(&knot.agent_profile_ref)?
      .ok_or_else(|| PortError::ProfileNotFound(...))?;
  let config = profile.resolve_for_knot(knot);
  let timeout = profile.session_timeout();
  Ok((config, timeout))
  ```
- [ ] Move profile resolution tests from `process_strand.rs` (`phase3_profile_resolution_tests`, `phase7_timeout_resolution_tests`) to `src/domain/value_objects.rs` tests
- [ ] **Verify:** `cargo test` — all tests pass. Domain tests verify profile→config mapping without port mocks.

### Phase 10: Final verification — `ProcessStrand` slimmed down

**Goal:** After Phases 6–9, `ProcessStrand::execute()` should be primarily orchestration: call domain methods, fan results to ports. Verify the use case is significantly smaller.

- [ ] `ProcessStrand::execute()` body is primarily `match`/port calls (~100 lines of orchestration)
- [ ] Domain tests cover: `StrandCheckResult` variants, `Knot::deleted_prompt()`, `TieOffOutcome::derive()`, `AgentProfile::resolve_for_knot()`
- [ ] Application tests in `process_strand.rs` test the pipeline flow, not individual rules
- [ ] `cargo test` — all tests pass
- [ ] `cargo clippy` — no new warnings
- [ ] `cargo build` — compiles cleanly

**Expected result:** `process_strand.rs` drops from ~3,867 lines to ~2,000 (mostly tests). The ~545 lines of use case code shrink to ~200. Domain layer gains ~200 lines of rule logic with their own test modules.

## Notes

- This is a pure structural refactor — no behaviour changes. Every test that passes before must pass after.
- The `ensure_strand_dir_and_watch` extraction is the only code change beyond moving. It converts a method that captures `self.log_port`, `self.event_source` into a free function that takes those as parameters. This is a straightforward mechanical change.
- Test modules named `phase*_tests` reflect the incremental feature-implementation history. Their names are preserved as-is during the move — renaming them is out of scope for this refactor.
- The ports test module (`ports.rs` tests) is untouched — those are contract tests for the traits themselves.
- `session_resume.rs` and `store.rs` in `application/` are unaffected.
