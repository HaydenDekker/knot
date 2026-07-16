# Phase 0: Create `ProcessStrandBuilder` in `tests/helpers.rs`

**Plan:** [Consolidate `build_process_strand` Test Helpers](consolidate-build-process-strand-plan.md)

## Checklist
- [x] Define `ProcessStrandBuilder` struct with builder-pattern setters — done in `tests/helpers.rs`
- [x] Define `ProcessStrandResult` struct with all possible fields — 12 fields, optional tracking ports
- [x] Implement the common setup: loom registration, mock ports, profile repo, `ProcessStrand::new()` — wires all 12 constructor arguments
- [x] Use `default_profile()` for the default profile; support `with_profile(AgentProfile)` override
- [x] Support single loom by default; `with_looms(Vec<Loom>)` for multi-loom
- [x] Support optional tracking ports via `with_tracking_git()`, `with_tracking_event_dispatcher()`, `with_tracking_file_checker()`
- [x] Add a `build()` method returning `ProcessStrandResult`
- [x] Verify: `cargo test` — all 64 existing integration tests still pass, clippy clean

## Deviations
- Used `unwrap_or_else(default_profile)` to avoid temporary value lifetime issues with `&default_profile()`

## Discoveries

## Notes
- `ProcessStrandResult` holds both concrete mock types (for `.with_tracking_*`) and uses trait objects internally for `ProcessStrand::new()`
- The builder is in `tests/helpers.rs` which is already used by 3 other test files via `mod helpers;`
