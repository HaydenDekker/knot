# Design: Startup Sequence — Per-Run Log Clearing

**Type:** Subsystem reference
**Subsystem:** server lifecycle / startup (`run_startup` in `src/server.rs`)

## What It Is

The ordered sequence Knot runs at startup, and the per-run scoping of the
operational logs (rig-log + loom-logs). Every startup truncates the
rig-log and **every** loom-log *after* legacy-layout migration and
*before* loom discovery, so each log always contains exactly the events
of the current knot process run: it starts with the fresh
`KnotRegistered`/`LoomStarted` events and ends with `LoomStopped` at
shutdown.

## Why

The logs exist so knots and operators can react to events *in the
current run* (event-watching knots, `knot-inspect`/`knot-analyst`
workflows). The durable audit history lives in the tie-off files —
plain text, git-versioned, permanent. Without the clear, two problems
compounded across runs:

1. **Console-log pollution that outlived its cause.** `read_all` on a
   loom-log runs on every 5-second state-write cycle
   (`write_state.rs::derive_knot_state`) and in the activity/
   knot-status queries. The reader skips unparseable lines with a
   `WARN:` eprintln (defence against intra-run corruption and legacy
   event shapes, e.g. pre-0.33 3-tuple `EventsDispatched` lines). A
   single stale bad line therefore re-fired its warning on every read,
   every cycle, every run, forever.
2. **Unbounded, low-value growth** — prior-run events that nothing
   consumes accumulate indefinitely.

Rolling logs (`.loom-log.1` / timestamped archives) were considered and
rejected: no consumer of cross-run log history exists, so retention
adds files for nothing. Truncation is the minimal behaviour that
matches intent.

## The Sequence

`run_startup` (in order):

1. **Create rig dir** if missing (non-fatal warning on failure).
2. **Create `.workspace-agent-config.yaml`** if missing (never
   overwrites an existing config).
3. **Create `models.yml`** if missing (never overwrites).
4. **Migrate legacy layout** (`migrate_legacy_rig_layout`) — moves
   runtime artifacts out of the rig dir to the runtime root
   (`tie-offs/<rig-basename>/`). Idempotent, non-fatal.
5. **Clear operational logs** — `LoomLogPort::clear_all()` then
   `RigLogPort::clear()`. Non-fatal: a failure logs `WARNING: failed
   to clear …` and startup proceeds (same error style as migration,
   config creation, and rig git init).
6. **Ensure rig git repository** (`ensure_rig_repo`, idempotent,
   non-fatal).
7. **Discover looms** (`DiscoverLooms`) — scans the rig dir, registers
   looms in the store, appends the fresh `KnotRegistered` (per knot) →
   `KnotParseWarning` (if any) → `LoomStarted` events, starts
   watchers.
8. **Register the rig-dir watch** (auto-discovery of new looms/knots).

### Ordering invariants

- **Migration → clear:** moved legacy logs are cleared *at their new
  paths*. Clearing before migration would leave the moved files full of
  residue at the runtime root.
- **Clear → discovery:** the fresh `KnotRegistered`/`LoomStarted` are
  the first log lines of the run. Clearing after discovery would
  discard them and leave `last_event_at` unset in the state snapshot.
- **Clear is before the state writer starts** (the state writer is
  spawned in `start_knot` after `run_startup` returns), so no 5-second
  cycle ever reads a half-cleared log.

## Components

| Piece | Location | Role |
|---|---|---|
| `RigLogPort::clear()` | `src/application/ports.rs` | Truncate `<runtime-root>/.rig-log`; no-op when missing. Required trait method. |
| `LoomLogPort::clear_all()` | `src/application/ports.rs` | Truncate every top-level `*/.loom-log` under the runtime root; no-op when the root is missing. Required trait method. |
| `FileSystemRigLog::clear` | `src/adapters/outbound/rig_log.rs` | In-place truncate via `fs::File::create`. |
| `FileSystemLoomLog::clear_all` | `src/adapters/outbound/loom_log.rs` | `read_dir` over `derive_runtime_root(rig_dir)`; per subdir, truncate only a file named `.loom-log` (top level only). |
| `run_startup` wiring | `src/server.rs` | Steps 5 above; `WARNING:` on error, startup proceeds. |

### Adapter path conventions (gotcha)

`FileSystemLoomLog` is constructed with the **rig dir** and derives the
runtime root internally (`derive_runtime_root`), while
`FileSystemRigLog` is constructed with the **runtime root** directly.
`clear_all` enumerates `derive_runtime_root(&self.rig_dir)`; `clear`
uses its field as-is.

## What the Clear Touches — and Doesn't

| Path under `tie-offs/<rig>/` | Touched? |
|---|---|
| `.rig-log` | Truncated (in place; file identity stable) |
| `*/.loom-log` (top level per loom dir, **including orphans**) | Truncated |
| `*/{EventId}/` dispatch dirs (pending events) | No — pending work survives restarts |
| Tie-off files | No — durable audit history |
| `state.json` | No |
| `events/` queue | No — persistent-queue behaviour |
| `*/.loom-log` nested *inside* a dispatch dir | No — only top-level per loom dir |

Truncation is in place (`fs::File::create`), not deletion: appends
reopen in append mode on every write and no production code holds a
long-lived log handle (`SharedLoomLog`/`SharedRigLog` are test-only),
so there is no offset-desync risk and file identity stays stable.

Loom dirs that contain *only* an emptied `.loom-log` are **not**
pruned — deletion of anything beyond log files is out of scope; the
dirs are harmless and reusable if the loom returns.

## Reader Behaviour (unchanged)

The `WARN:` skip for non-JSONL/legacy lines stays in
`FileSystemLoomLog::read_all` as defence against *intra-run*
corruption (e.g. an agent writing into a log file). It simply no longer
re-fires on ancient content after every restart, because the content is
gone at startup.

## State Derivation After the Clear

`WriteState::derive_knot_state` reads the (cleared) loom-log and finds
the latest knot-referencing event. Post-startup, that is the fresh
`KnotRegistered` → knot status `idle` with `last_event_at` set. The
clear therefore does not regress state derivation; it makes it
deterministic (no prior-run `KnotCompleted` shadowing the new run).

## Testing

| Test | Pins |
|---|---|
| `rig_log_clear_truncates` / `rig_log_clear_missing_file_is_noop` (unit) | Truncate-in-place; no-op on fresh rig |
| `loom_log_clear_all_truncates_every_loom_log` (unit) | All loom-logs emptied, files kept, appends work after |
| `loom_log_clear_all_includes_orphan_looms` (unit) | Orphaned loom dirs cleared (enumeration is over the runtime root, not discovered looms) |
| `loom_log_clear_all_leaves_other_files_alone` (unit) | Tie-offs, dispatch dirs, `state.json`, `events/`, and nested `.loom-log` byte-identical |
| `test_startup_clears_logs_before_discovery` (composition, real adapters) | Full order: prior-run rig-log (incl. unparseable line) gone; first loom-log line is fresh `KnotRegistered`; orphan cleared; durable files byte-identical; post-clear state derives `idle` + `last_event_at` |
| `test_startup_migrates_legacy_layout` (composition) | Migration → clear → discovery for *moved* legacy logs: new-path file exists, legacy line gone, fresh `LoomStarted` present |

## Notes

- Clearing is non-fatal by design: a permission error on one log must
  not block the whole rig from starting (matches every other
  `run_startup` side effect).
- The previous run's `LoomStopped` disappears with the clear; each
  run's log is a clean `LoomStarted … LoomStopped` bracket.
- Any workflow that assumed cross-run log history must count "since the
  last startup" instead (knot-analyst guidance updated accordingly).

## Related Documents

- [docs/concepts.md](../../docs/concepts.md) — Logs section (per-run
  semantics for users)
- `knot-update` skill changelog — Knot 0.34.0 entry (no document
  migration; first run discards earlier runs' log content)
