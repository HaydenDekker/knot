# Phase 0: Domain — Flatten `derive_tieoff_path` and Update Tests

**Plan:** [Flatten Tie-Off Paths](flat-tie-off-paths-plan.md)

## Checklist
- [x] Read `src/domain/knot_file.rs` — confirm current `derive_tieoff_path()` implementation
- [x] Update `derive_tieoff_path()` — remove `.join(knot_name)` so it returns `rig.join("tie-offs").join(loom_id)`
- [x] Update doc comment on `derive_tieoff_path()` — "Returns `rig/tie-offs/{loom-id}/`"
- [x] Update doc comment on `derive_loom_log_path()` — verify still accurate (unchanged)
- [x] Read `src/application/usecases/process_strand.rs` — confirm `compute_tie_off_path()` inherits from `derive_tieoff_path()`
- [x] Update doc comment on `compute_tie_off_path()` — "Uses statically derived path: `rig/tie-offs/{loom-id}/{knot-name}-tie-off.md`"
- [x] Update unit test `derive_tieoff_path_builds_correct_path` in `knot_file.rs` — expected path from `/workspace/rig/tie-offs/my-loom/review-knot` to `/workspace/rig/tie-offs/my-loom`
- [x] Run `cargo test --lib derive_tieoff_path` — verify domain path derivation test passes
- [x] Run `cargo test --lib knot_file` — verify all knot_file tests pass
- [x] Run `cargo test --lib` — verify all unit tests pass

## Deviations

None.

## Discoveries

- `process_strand_deleted_includes_strand_history` test (in `process_strand.rs`) also had a hardcoded nested tie-off path (`/rig/tie-offs/test-loom/k1/k1-tie-off.md`) that needed flattening. This was a Phase 1 item but was pulled forward because it blocked `cargo test --lib` verification.

## Notes

- The `knot_name` parameter to `derive_tieoff_path()` is retained in the signature (prefixed `_`) to avoid cascading caller changes. It is a no-op and can be fully removed in a later cleanup.
