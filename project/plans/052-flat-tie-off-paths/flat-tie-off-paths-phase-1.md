# Phase 1: Application Layer — Update ProcessStrand Tests

**Plan:** [Flatten Tie-Off Paths](flat-tie-off-paths-plan.md)

## Checklist
- [x] Read `src/application/usecases/process_strand.rs` — locate all hardcoded tie-off path strings
- [x] Update `execution_test_shared::build_process_strand` — already flat (`/rig/tie-offs/test-loom/tie-off-k1.md`)
- [x] Update `process_strand_deleted_includes_strand_history` — already flat (`/rig/tie-offs/test-loom/tie-off-k1.md`)
- [x] Search for any other nested tie-off path references in `process_strand.rs` — none found
- [x] Run `cargo test process_strand` — 38 tests passed
- [x] Run `cargo test --lib` — 476 tests passed

## Deviations

## Discoveries

No code changes needed — tie-off paths in `process_strand.rs` were already flattened during Phase 0 (domain changes to `derive_tieoff_path`). Both hardcoded mock paths and the doc comment already reflect the flat layout.

## Notes
