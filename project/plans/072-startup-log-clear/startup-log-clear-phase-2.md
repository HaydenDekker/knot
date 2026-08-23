# Phase 2: Wire the clear into `run_startup`

**Plan:** [Clear Loom-Logs and Rig-Log at Startup](startup-log-clear-plan.md)

## Checklist
- [x] Write acceptance-level composition test `test_startup_clears_logs_before_discovery` in `src/server.rs` — real adapters, pre-populated runtime root (rig-log with prior-run events + unparseable line, review-loom prior-run `.loom-log` + tie-off file + dispatch-dir file, orphan `old-loom` dir), rig with one knot; confirmed red before wiring (rig-log not cleared)
- [x] Insert `clear_all()` + `clear()` in `run_startup` after `migrate_legacy_rig_layout`, before `DiscoverLooms` — non-fatal `WARNING:` on error (matches established `run_startup` error style); updated the `run_startup` doc comment's step list
- [x] Adjust `test_startup_migrates_legacy_layout` — see Deviations
- [x] Run full suite (`cargo test --no-fail-fast`) — all integration suites green (adapters 33, rig_log 12, composition 6, smoke 11, …); only failure is the pre-existing unrelated `build_listener_context_prompt_includes_do_not_edit_guidance`
- [x] Open question decided: **no** pruning of loom dirs that contain only an empty `.loom-log` — deletion of anything beyond log files is out of scope (plan default); empty dirs are harmless and reusable if the loom returns

## Deviations
- `test_startup_migrates_legacy_layout` adjustment (required by the plan): the test's pre-startup legacy `.loom-log` content (`legacy-line`) is now cleared by the startup sequence, so the post-startup assertion was inverted from `content.contains("legacy-line")` to `!content.contains("legacy-line")`, and the doc comment updated. The test's intent — migration occurred — is preserved: the file exists at the new path (the `expect` + non-empty read), and the fresh `LoomStarted` append at the new path still proves migration ran before log appends. The pure `migrate_legacy_rig_layout` tests (no startup) are untouched — migration itself still preserves content; only `run_startup` clears afterwards.

## Discoveries
- Discovery event order per loom is `KnotRegistered` (per knot) → `KnotParseWarning` (if any) → `LoomStarted`, so the first line of a fresh loom-log is `KnotRegistered` — the test pins that.
- `build_app_context` does not load the persisted event queue (that is `start_event_pipeline`), so the composition test needs no events-dir setup and `ctx.strand_queue` is `None` — `WriteState` handles it.
- The state.json assertion runs `WriteState::execute()` directly after `run_startup` (the state-writer background task is not started by `run_startup`), verifying `derive_knot_state` against the cleared log: knot `idle` with `last_event_at` set from the fresh `KnotRegistered`.
- Live rig residue confirmed before implementation: `tie-offs/rig/workflow-loom/.loom-log` 681 lines, `tie-offs/rig/new-loom/.loom-log` 547 lines — exactly the cross-run residue the plan targets.

## Notes
- Clear placement: after `migrate_legacy_rig_layout` (moved legacy logs cleared at new paths), before `git init` and `DiscoverLooms` (no fresh event discarded). Order between the two clears is irrelevant (plan).
- `run_startup` step-list doc comment renumbered (clear is new step 2).
