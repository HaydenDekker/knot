# Phase 2: Incident-reproduction integration tests (full composition)

**Plan:** [Queue Entry Identity Self-Heal — Filename Is the Event ID](queue-identity-self-heal-plan.md)
**Commit:** `088dee5` (branch `refactor/queue-identity-self-heal-plan`)
**Date:** 2026-08-25

## Checklist
- [x] New `tests/queue_identity.rs` — Part-B style (real `DiskBackedEventQueue`, real `spawn_process_strand_loop` / `step_knot`, mock `pi` script with a per-run log, `wait_until` helper), using the established watcher-free pre-seeding technique (direct file writes to `tie-offs/<rig>/events/`, picked up by `load_persisted` at startup)
- [x] `backdated_head_processes_first_and_drains` — the incident repro: E1 (tail) + E2 (head) pre-seeded, E2's file renamed to an earlier `{timestamp}-{hex}.json` name with the JSON id untouched (the 14:52 backdate). Asserts in order: E2 processed **first** (loom-log `KnotCompleted` ordering + one git commit per run, head commit older than tail's), E2's queue file removed with **no orphan** (neither the renamed nor the original-id file survives; events dir empty), tie-offs in the same order, and `QueueIdle` in the rig-log with the queue **empty** — clean drain, not the pre-fix idle-with-full-queue phantom head
- [x] `restart_over_duplicate_key_files_collapses_to_one` — pre-seeds the exact post-incident state (renamed file + file with the same JSON id and same dedup key); `load_persisted` collapses to one entry for that key, exactly one agent run / one `KnotCompleted`, no orphan file, `QueueIdle`
- [x] `step_mode_renamed_head_does_not_panic` — `step_knot` head path over a renamed head: event processed (one agent run, one `KnotCompleted`), exit 0, file removed, no orphan
- [x] `cargo build` clean; `cargo test --no-fail-fast` fully green — 29 binaries, 0 failures, across three runs; `late_removal` (21), `persistent_queue` (8), `step` (19) unchanged in expectation; plan 076's work on this branch untouched; the known `thinking_level` environment flake (Phase 1 record) did not recur

## TDD red-check
The sub-agent confirmed all 3 tests are genuine regression proofs: temporarily disabling Phase 1's heal block made all 3 fail exactly as the incident mechanism predicts (two phantom-head wedge timeouts, one step-mode "queue empty" skip), then restored the code via `git checkout` — the commit contains only the new test file.

## Deviations
1. **Red-check method**: rather than checking out pre-Phase-1 code (impractical — the tests are new and the fix is interleaved in the same files), the red state was produced by temporarily commenting out the heal block in `scan_events`, observing the three predicted failures, and restoring via `git checkout`. Equivalent proof, no history rewrite.
2. **Test 1 asserts git-commit ordering** (one commit per run; the head's commit older than the tail's) — stronger than the plan's minimum (loom-log + "git commit"), and it pins the FIFO-order intent of the operator's backdate end-to-end.
3. **Test 3 invokes `step_knot` at the function level** (the pattern used by `tests/step.rs`) rather than through the CLI binary.

## Discoveries
- The incident repro passes **only** with Phase 1's normalisation — the two wedge tests time out (loop idles with a non-empty queue, the exact field symptom) and the step test reports "queue empty" for a file sitting in `events/` when the heal is disabled. Together with the Phase 1 unit tests, both incident paths (wedge + orphan) are now pinned at two levels (unit + full composition).
- `QueueIdle` with an **empty** events dir is the clean-drain signature the repro asserts — the pre-fix failure mode is `QueueIdle`-adjacent silence with the queue still full.

## Notes
- Phase 3 (documentation, skills, changelog) is next.
