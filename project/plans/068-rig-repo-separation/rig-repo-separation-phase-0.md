# Phase 0: Runtime-root Derivation and Consumers

## Status

Completed

## Summary

Added the single source of truth for the project-side runtime tree —
`derive_runtime_root(rig_dir)` in `src/domain/knot_file.rs` — and re-rooted
every runtime-path consumer from `rig/…` to
`<project-root>/tie-offs/<rig-basename>/…`. The rig directory now holds
reusable source only; all runtime artifacts (tie-offs, event dispatch
dirs, loom-logs, `state.json`, `.rig-log`, `events/`) live under the
runtime root at the project level.

Unit tests in each affected file were updated to the new expected paths
first (red), then the production changes turned them green.

## Changes

### Domain (single source of truth)

**`src/domain/knot_file.rs`:**
- New `derive_runtime_root(rig_dir)`:
  `<project-root>/tie-offs/<rig-basename>/` where `<project-root>` is the
  parent of the rig dir (existing convention) and `<rig-basename>` is the
  rig dir's file name (`rig`, `dev-rig`, …).
- Degenerate case: rig dir at the filesystem root has no parent —
  `project_root = rig_dir` fallback applies (covered by a unit test).
- `derive_tieoff_path()` and `derive_loom_log_path()` now derive from the
  runtime root; the `{loom-id}/…` subtree shape is unchanged.
- New tests: default rig, named rig (`dev-rig` — the namespacing gap),
  relative rig dir, rig at filesystem root, plus named-rig variants of the
  two existing derive tests.

### Consumers (re-rooted)

| File | Change |
|---|---|
| `src/adapters/outbound/event_dispatcher.rs` | Event files at `derive_runtime_root(rig_dir)/{loom}/{EventId}/` |
| `src/adapters/outbound/loom_log.rs` | Docs updated — path derived via `derive_loom_log_path` (auto re-rooted) |
| `src/application/usecases/loom/mod_watchers.rs` | Event dispatch dirs at runtime root |
| `src/application/usecases/context_providers.rs` | Pending-event fallback scan reads runtime root |
| `src/application/usecases/process_strand.rs` | `compute_tie_off_path` doc (path derived via `derive_tieoff_path`) |
| `src/server.rs` | `AppContext.runtime_root` added; `FileSystemRigLog` and `FileSystemStateWriter` constructed with the runtime root; event queue dir at `runtime_root/events/` |
| `src/application/usecases/test_fixtures.rs` | `MockEventDispatcher` synthetic path follows the new derivation |

### Composition notes

- `WriteState` still receives `rig_dir` — the `rig_path` field in
  `state.json` continues to point at the rig directory (source), not the
  runtime root (per plan Notes).
- The rig directory watcher (`WatchType::Rig`) no longer sees
  `state.json`'s 5-second rewrite churn (runtime root is outside `rig/`).
- `knot share` is unaffected (zips `*-loom/` + `profiles/` from the rig).

### Docs updated (path references)

`state_writer.rs`, `rig_log.rs`, `write_state.rs`, `disk_event_queue.rs`,
`event_store.rs`, `pending_event.rs`, `main.rs` (share doc),
`tests/persistent_queue.rs`, `tests/multi_loom.rs`, `tests/helpers.rs`,
`tests/tie_off.rs`.

### Tests updated

Unit: `knot_file.rs`, `loom_log.rs`, `event_dispatcher.rs`,
`mod_watchers.rs`, `context_providers.rs`, `process_strand.rs`,
`config_event_handler.rs`, `loom_repository.rs` (fixture dir renamed —
runtime tree no longer lives inside the rig).

Integration: `tests/helpers.rs` (`read_state_file` / `read_loom_log` now
derive the runtime root — all consumers of these helpers follow),
`tests/tie_off.rs`, `tests/agent_integration.rs`, `tests/pipeline.rs`,
`tests/smoke.rs`, `tests/adapters.rs`, `tests/rig_cli.rs` (share fixture
now creates the project-level runtime tree).

Test fixtures that used the tempdir itself as rig dir (which would write
outside the tempdir under the new derivation) now use a `rig/`
subdirectory, mirroring the real layout.

## Verification

- Red-first: consumer tests updated before production changes produced
  10 red unit tests; production changes turned them green.
- End-to-end (manual, mock `pi` on PATH): fresh project → `knot` →
  strand trigger → tie-off at `tie-offs/rig/review-loom/`, loom-log,
  `state.json`, `.rig-log`, `events/` all under `tie-offs/rig/`;
  `rig/` contains only source (`profiles/`, `*-loom/`,
  `.workspace-agent-config.yaml`); `state.json.rig_path` still points at
  `rig/`.
- Named-rig e2e: `knot dev-rig` → runtime tree at `tie-offs/dev-rig/`.

## Test Results

```
cargo test --no-fail-fast
TOTAL passed: 980 failed: 1
```

The single failure —
`domain::events::tests::build_listener_context_prompt_includes_do_not_edit_guidance`
— is **pre-existing on clean HEAD** (verified via stash) and unrelated to
this phase (asserts prompt template text in `src/domain/events.rs`, no
path involvement).

## Notes / Deferred

- Legacy-layout auto-migration is Phase 3.
- Rig git init + parent `.gitignore` exclusion is Phase 1.
- Git versioner rig-awareness (`git reset -q -- <rig-dir>`) is Phase 2.
- The `state.json` 5s rewrite keeps the project tree dirty while Knot
  runs (accepted — audit trail by design, see plan Notes).
