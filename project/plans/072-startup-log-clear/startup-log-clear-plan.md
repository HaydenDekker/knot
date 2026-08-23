# Plan: Clear Loom-Logs and Rig-Log at Startup

## Problem

The rig's operational logs accumulate indefinitely across process runs:

- Rig-log: `tie-offs/<rig>/.rig-log` (JSONL: `TimeoutExceeded`, `QueueIdle`)
- Loom-logs: `tie-offs/<rig>/<loom-id>/.loom-log` (JSONL: `LoomStarted`,
  `KnotRegistered`, `KnotProcessing`, `KnotCompleted`, `KnotFailed`, …)

On every startup, `DiscoverLooms` re-appends `KnotRegistered` +
`LoomStarted` for each loom on top of the previous run's events, and
shutdown appends `LoomStopped`. Nothing is ever removed. Two consequences:

1. **Console-log pollution that outlives its cause.** `read_all` on a
   loom-log is called on every 5-second state-write cycle
   (`write_state.rs::derive_knot_state`) and by the activity/knot-status
   queries. The reader skips unparseable lines with a `WARN:` eprintln
   (defence against non-JSON content and legacy event shapes — e.g. the
   pre-0.33 3-tuple `EventsDispatched` entries). A single stale bad line
   therefore emits a warning **every read, every cycle, every run,
   forever** — drowning the console in noise and pushing genuinely
   important runtime output out of sight.
2. **Unbounded, low-value growth.** The logs' purpose is to let knots and
   operators react to events *in the current run* (knots that watch for
   rig events, `knot-inspect`/`knot-analyst` workflows). The durable
   audit history already lives in the tie-offs, which are
   git-versioned and permanent. Cross-run log content is residue: in this
   repository the two loom-logs already hold 547 and 681 lines of
   previous-run events that nothing consumes.

## Target

When this plan is complete:

1. **Every startup clears the operational logs first.** Before loom
   discovery runs, `run_startup` truncates:
   - the rig-log (`tie-offs/<rig>/.rig-log`), and
   - **every** `.loom-log` found under the runtime root
     (`tie-offs/<rig>/*/`), including orphaned loom-logs belonging to
     looms that no longer exist in the rig directory.

   After the clear, the first lines of each loom-log are the current
   run's `KnotRegistered`/`LoomStarted` events — the log file contents
   always equal "everything that happened since this knot process
   started".

2. **Only log files are touched.** Tie-off files, event dispatch
   directories, `state.json`, and the `events/` queue inside loom
   directories are never cleared — they are the audit history and the
   pending work that must survive restarts.

3. **The clear is non-fatal.** A failure to clear a log file logs a
   `WARNING:` line (stderr) and startup proceeds, matching the
   established `run_startup` error style (migration, config-file
   creation, rig git init).

4. **Clearing happens after legacy-layout migration and before
   discovery**, so migrated legacy logs are cleared at their new paths
   and no discovery event is ever discarded by the clear.

5. **Documentation and skills are corrected** so the per-run semantics
   are stated instead of the old append-forever semantics:
   `docs/concepts.md`, `knot-inspect` and `knot-analyst` skills, and a
   `knot-update` changelog entry for the new binary version.

## Existing Tests

| Test | What it covers | Status |
|------|----------------|--------|
| `src/adapters/outbound/loom_log.rs` unit tests | append/read_all round-trips, concurrent writes, non-JSONL and legacy 3-tuple lines skipped with warning | ✅ Green — defines current reader behaviour |
| `src/adapters/outbound/rig_log.rs` unit tests | append/read_all round-trips, parent-dir creation, concurrent writes | ✅ Green |
| `src/server.rs` composition tests (`migration_*`) | Legacy layout migration at startup with real adapters (pre-populated `rig/tie-offs/`, `rig/.rig-log` moved to runtime root) | ✅ Green — the template for a startup-sequence test |
| `tests/rig_log.rs` | Rig-log event recording via `ProcessStrand` with mocked ports | ✅ Green |
| `tests/adapters.rs` (loom-log open/append) | Real `FileSystemLoomLog` open/append at the runtime root | ✅ Green |
| `tests/helpers.rs` (`read_loom_log_events`) | Reads `.loom-log` from `tie-offs/<rig-basename>/<loom-id>/` for integration assertions | ✅ Green |

## Test Gaps

- No test that any log file is cleared at startup — no `clear` method
  exists on either port yet.
- No test that **orphaned** loom-logs (loom removed from the rig dir)
  are cleared.
- No test that the clear touches only `*.loom-log`/`.rig-log` files and
  leaves tie-off files, dispatch dirs, `state.json`, and `events/`
  intact.
- No startup-sequence test pinning the order: migration → clear →
  discovery (i.e. the current run's `LoomStarted` is the first log line,
  and a migrated legacy log is emptied, not preserved).

## Phases

### Phase 1: `clear` / `clear_all` port methods with adapter tests (TDD)

Extend the ports and filesystem adapters:

- `RigLogPort::clear() -> Result<(), PortError>` — truncate
  `<runtime-root>/.rig-log`; no-op when the file does not exist.
- `LoomLogPort::clear_all() -> Result<(), PortError>` — truncate every
  `*/.loom-log` under the runtime root; no-op when none exist. The
  adapter knows its own layout (`derive_runtime_root`), so the port
  needs no path argument.

Implementation notes:

- Truncate in place (open with write/truncate, or `fs::File::create`),
  do not delete: appends reopen in append mode on every write and no
  production code holds a long-lived log handle (`SharedLoomLog` /
  `SharedRigLog` are test-only), so in-place truncation has no
  offset-desync risk and keeps file identity stable.
- `clear_all` enumerates runtime-root subdirectories and truncates a
  file only when it is named `.loom-log` — never descend into or touch
  dispatch dirs or tie-off files.
- Add the methods to every mock port implementation so the suite
  compiles: `MockLoomLogPort` / `MockRigLogPort` in
  `src/application/ports.rs`, `src/application/usecases/test_fixtures.rs`,
  and the local mock in `src/application/session_resume.rs` (record the
  call for assertions).

Unit tests (failing first, then green):

- `rig_log_clear_truncates` — append events, clear, `read_all` is empty
  and the file exists but is empty.
- `rig_log_clear_missing_file_is_noop` — clear on a fresh dir returns
  `Ok`.
- `loom_log_clear_all_truncates_every_loom_log` — two loom-logs with
  events; both emptied.
- `loom_log_clear_all_includes_orphan_looms` — a loom-log exists under
  the runtime root with no corresponding `*-loom` dir in the rig; it is
  still cleared.
- `loom_log_clear_all_leaves_other_files_alone` — tie-off files, a
  dispatch-dir file, `state.json`, and `events/` contents in the same
  loom dirs are byte-identical after `clear_all`.

### Phase 2: Wire the clear into `run_startup` (acceptance-level test)

In `run_startup` (`src/server.rs`), after
`migrate_legacy_rig_layout(rig_dir)` and before `DiscoverLooms`:

```rust
// Clear operational logs: per-run scope, tie-offs hold the history.
if let Err(e) = ctx.loom_log_port.clear_all() {
    eprintln!("WARNING: failed to clear loom-logs: {e}");
}
if let Err(e) = ctx.rig_log_port.clear() {
    eprintln!("WARNING: failed to clear rig-log: {e}");
}
```

(Exact placement: after migration so moved legacy logs are cleared at
their new paths; before discovery so the fresh `LoomStarted` is line 1.
Order between the two clears is irrelevant.)

Acceptance-level test — composition test in `src/server.rs`
(`composition_tests`, following the `migration_*` test pattern with real
adapters, no mocks):

- Pre-populate the runtime root: `tie-offs/rig/.rig-log` with prior-run
  events (including one unparseable line), `tie-offs/rig/review-loom/`
  with a prior-run `.loom-log` plus a tie-off file and a dispatch-dir
  file; also an orphan loom dir `tie-offs/rig/old-loom/.loom-log` with
  no `old-loom` loom in the rig.
- Populate the rig dir with a `review-loom` containing one knot.
- Run `run_startup`.
- Assert: `.rig-log` exists and is empty; `review-loom/.loom-log`
  exists, is non-empty, and its **first** event is `KnotRegistered`
  (or `LoomStarted` — pin whichever the discovery order produces) with
  no prior-run events present; `old-loom/.loom-log` is empty; the
  tie-off file, dispatch-dir file are byte-identical.
- Assert `state.json` knot status is still derived correctly after the
  clear (knot `idle` with `last_event_at` set from the fresh
  `KnotRegistered`) — i.e. the clear does not regress
  `derive_knot_state`.

Then run the full suite (`cargo test`) — in particular
`tests/adapters.rs`, `tests/rig_log.rs`, and the `migration_*`
composition tests must stay green (migration tests pre-populate
`rig/.rig-log` in the *legacy* location and assert its *content*
survives the move; with the clear wired in, their post-startup
assertions see the cleared file — adjust those assertions to verify
the move happened (file at new path) rather than preserving legacy
content, or pre-populate the legacy log with content and assert the
new path file exists and is empty. The test's intent — migration
occurred — is preserved; document the adjustment in the phase doc.)

### Phase 3: Documentation, skills, changelog

- `docs/concepts.md` — replace "The rig-log survives server restarts
  and supports multiple consumers" with the per-run semantics: logs are
  truncated at startup; tie-offs are the durable record.
- `knot-inspect` / `knot-analyst` skills (project-level
  `.agents/skills/`) — annotate the log descriptions: "append-only
  JSONL, **cleared at knot startup** (per-run scope)". Update the
  analyst's "count events in the last 24 hours" guidance to
  "since the last startup".
- `.agents/skills/knot-update/SKILL.md` — new changelog entry for the
  bumped version: no document migration required; on first run of the
  new binary all `.rig-log`/`.loom-log` content from earlier runs is
  discarded (tie-offs unchanged). Bump the skill's compatibility line.
- Version bump (MINOR, `Cargo.toml` 0.33.0 → 0.34.0) is part of plan
  completion via the project-plan-completion skill — record the
  completed behaviour in `docs/release-notes.md` there as well.
- Publish updated skills globally per AGENTS.md (copy to
  `~/.agents/skills/` and verify with diff).

## Notes

- **Alternative considered and rejected — rolling logs** (rename to
  `.loom-log.1` / timestamped files before a new run): the stated goal
  is that no cross-run audit history is needed (tie-offs are the
  history), so retention adds files without a consumer. Truncation is
  the minimal behaviour that matches intent.
- **`events/` queue is intentionally out of scope.** Pending events must
  survive restarts (persistent-queue behaviour); only logs are
  operational-scope.
- **`LoomStopped` pairing:** the previous run's `LoomStopped` disappears
  with the clear; the log now starts each run with `LoomStarted` and
  ends with `LoomStopped` — a clean per-run bracket.
- **Reader warning behaviour is unchanged.** The `WARN:` skip logic for
  non-JSONL/legacy lines stays as a defence against *intra-run*
  corruption (e.g. an agent writing into a log file); it simply no
  longer re-fires on ancient content after every restart.
- Open question (decide during Phase 2): whether `clear_all` should also
  prune loom dirs that contain *only* a now-empty `.loom-log` (no
  tie-offs, no dispatch dirs). Default: **no** — deletion of anything
  beyond log files is out of scope for this plan.
