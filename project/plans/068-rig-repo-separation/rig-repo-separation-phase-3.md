# Phase 3: Legacy Layout Migration

## Status

Completed

## Summary

At startup, a rig found on the legacy layout (runtime artifacts inside
the rig directory) is migrated automatically to the project-level
runtime root:

- `rig/tie-offs/` → the runtime root itself (subtree preserved —
  `{loom-id}/` tie-off files, dispatch dirs, and `.loom-log` files)
- `rig/state.json` → `<runtime-root>/state.json`
- `rig/.rig-log` → `<runtime-root>/.rig-log`
- `rig/events/` → `<runtime-root>/events/`

The rig is left source-only. A one-line
`[startup] migrated legacy layout: … → <runtime-root>` notice is
logged. The migration is idempotent (a new-layout rig is a no-op that
creates no empty runtime root), and destination-exists conflicts keep
the destination and warn — the legacy source stays in place for manual
resolution (no data loss). All failures are non-fatal warnings.

**Startup ordering fix (discovered by this phase).** The e2e smoke run
exposed a race the unit tests could not see: `start_knot` spawned the
state writer and the event pipeline **before** `run_startup`. On the
multi-threaded tokio runtime the spawned tasks run immediately — the
state writer's first write and the queue's `events/` dir creation beat
the migration to creating the runtime root, turning legacy
`rig/tie-offs/` and `rig/events/` into false "conflicts" that were
never moved. Fix: `start_knot` now completes `run_startup`
(migration → rig git init → discovery → watcher registration) before
any pipeline task is spawned. This guarantees the plan's ordering
contract — migration and runtime-root creation precede the first state
write, the queue's persisted-file load, and watcher registration — so
moved dispatch directories, loom-logs, and queue files are used at
their new paths immediately.

## Changes

### Migration (`src/server.rs`)

- `move_legacy_path(src, dst, label, moved)` — moves one legacy path:
  no-op when the source is absent; destination-exists → keep
  destination, warn, leave source in place; rename failure → warn.
  Never returns an error (startup continues with whatever layout is on
  disk).
- `migrate_legacy_rig_layout(rig_dir)` — computes the runtime root via
  `derive_runtime_root(rig_dir)`, moves the legacy tie-off tree to the
  runtime root (creating the `tie-offs/` parent for the rename
  target), then `state.json` / `.rig-log` / `events/` (creating the
  runtime root only when something still needs to move). Logs the
  one-line `[startup] migrated …` notice when anything moved.
- `run_startup` calls `migrate_legacy_rig_layout(rig_dir)` after
  rig-dir + config-file creation and **before** `ensure_rig_repo` and
  `DiscoverLooms` — its doc comment now documents the full ordered
  sequence (migrate → rig git → discover → watch).

### Startup ordering (`src/server.rs::start_knot`)

- `run_startup` moved to the first step after `build_app_context`,
  before `start_config_pipeline`, `start_event_pipeline`,
  `start_state_writer`, and `spawn_process_strand_loop`. Doc comments
  on both `start_knot` and `run_startup` record the invariant and the
  race it prevents.

### Tests

Unit tests (`server.rs` composition tests, `setup_legacy_rig` helper):

- `test_migrate_moves_full_legacy_layout` — full legacy tree (tie-off
  files, `.loom-log`, dispatch dir with event file, state.json,
  .rig-log, events/ with queue file) moves to the runtime root with
  content intact; rig left source-only (looms + profiles untouched).
- `test_migrate_is_idempotent` — second run is a no-op.
- `test_migrate_keeps_destination_on_conflict` — pre-existing runtime
  root with its own state.json: destination content kept, conflicting
  legacy sources **not deleted** (no data loss), non-conflicting items
  still move.
- `test_migrate_noop_when_new_layout` — fresh rig: no-op, no empty
  runtime root created.
- `test_migrate_named_rig` — `dev-rig` migrates to
  `tie-offs/dev-rig/`.
- `test_migrate_partial_legacy` — only `events/` present: exactly what
  exists moves.

Ordering tests:

- `test_startup_migrates_legacy_layout` (`run_startup` level) — legacy
  rig with a valid loom: after `run_startup`, artifacts are at the
  runtime root, the rig is source-only, the loom is discovered, and
  the moved `.loom-log` carries both its legacy line and the
  `LoomStarted` event discovery appended **at the new path** (loom-log
  paths are derived at append time — an append at the new path proves
  migration preceded log appends and watcher registration).
- `cli_startup_migrates_legacy_layout` (`tests/rig_cli.rs`, real
  binary e2e) — the regression test for the spawn-ordering race: a
  legacy rig is started with the real `knot` binary; asserts the
  tie-off subtree, `events/`, `state.json`, and `.rig-log` all land at
  the runtime root, the rig is source-only, and the migration notice
  is logged. **Validated against the old ordering**: with
  `run_startup` temporarily deferred behind the pipeline spawns, the
  test fails with the exact conflict symptoms (legacy `rig/tie-offs/`
  and `rig/events/` never moved); with the fix it passes.

## Verification

- `cargo test --no-fail-fast` — 1000 passed, 1 failed (the pre-existing
  `domain::events` prompt-text failure, documented in phases 0–2).
- `cargo clippy --all-targets` — 162 warnings, identical to HEAD
  (no new warnings).
- Manual smoke (debug binary, temp git project, full legacy layout):
  single notice line
  `[startup] migrated legacy layout: tie-offs/, state.json, .rig-log,
  events/ → …/tie-offs/rig`; no conflicts; rig source-only; live
  `state.json` written at the runtime root by the state writer after
  the migration.

## Test Results

```
cargo test --no-fail-fast
TOTAL passed: 1000 failed: 1
```

The single failure —
`domain::events::tests::build_listener_context_prompt_includes_do_not_edit_guidance`
— is **pre-existing on clean HEAD** (documented in phases 0–2) and
unrelated to this phase.

## Notes / Deferred

- **Watcher re-trigger caveat** — `notify` does not rescan existing
  files when a watch starts. Migrated dispatch directories that already
  contain unprocessed event files must be touched once after migration
  to re-trigger (documented in the `knot-manage` file-watcher section —
  Phase 4).
- Migrated queue files are loaded by the event pipeline at the new
  path (verified by ordering: the queue's `load_persisted` runs in
  `start_event_pipeline`, after `run_startup`).
- Skills, glossary, docs, and the `knot-update` 0.31.0 changelog
  (auto-migration note, conflict-resolution guidance, watcher re-trigger
  caveat) are Phase 4.
- Migrate this repo's own rig (`rig/tie-offs/` + runtime files →
  `tie-offs/rig/`), bump 0.31.0, and verify is Phase 5.
