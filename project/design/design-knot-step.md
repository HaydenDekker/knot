# Design: Knot Step — Single-Event Stepping and Late Queue Removal

**Type:** Subsystem reference
**Subsystem:** event queue consumption (`StrandEventQueue` port,
`ProcessStrand` use case) and the `step_knot` lifecycle (`src/server.rs`,
`src/main.rs`)

## What It Is

Two coupled changes to how queued events are consumed:

1. **Late removal (at-least-once queue semantics).** The queued event
   file is no longer removed when the event is *popped* for processing.
   On success it is removed as the **last step before the git commit**;
   on failure/skip it is removed **at the point of failure**. The only
   window in which an event survives a crash is "processing in flight"
   — a restart re-queues it and the knot re-runs (safe by knot
   idempotency).
2. **`knot step`** — a CLI command that processes exactly **one**
   queued event and exits, running the full service startup and the
   service-identical shutdown cascade around that single execution.
   It is the manual trigger/observation tool: watch one cycle unfold,
   inspect rig state between events, debug a misbehaving knot.

```
knot step [--rig <rig-name>] [--event <event-filename>]
```

## Why

- **The service was all-or-nothing.** `start_knot` runs until Ctrl+C,
  draining the queue continuously. Debugging a misbehaving knot (wrong
  prompt, runaway event loop, event-enforcement follow-ups) required
  letting the service process events back-to-back with no observation
  point between them.
- **In-flight events were lost on crash.** `pop()` removed the event
  file *before* the agent ran. A crash or Ctrl+C during a long agent
  run lost the event even though the knot's work (tie-off, git commit)
  had not happened yet; a restart silently discarded it.

## Late-Removal Semantics

### The ordering contract

`ProcessStrand::execute_with_pending(&pending)` converts the
`PendingEvent` to a `StrandEvent` and runs the pipeline with the
event id threaded to the terminal points:

- **Success** (`handle_success`): the event file is removed as the
  **last step before the git-commit block** — dispatch, `KnotCompleted`,
  `StrandProcessed`, and event enforcement all happen first, and the
  commit captures everything, including the removal. Unconditional on
  success (even when the knot is not `git_versioned`).
- **Failure / skip / early error** (`handle_failure` agent
  failure/timeout, config resolution errors, `validate_strand` skip,
  `LoomNotFound` / knot-not-found, unknown event kind): removed before
  returning — consume-on-failure, no poison-pill retry loops.

**Invariant (pinned by tests): every return from event processing
removes the event file exactly once, and on success the removal
precedes the commit.**

`execute(event)` (the queue-less entry point) delegates with `None` —
it never touches the queue.

### Crash windows

| Window | Consequence |
|---|---|
| Crash while processing in flight (before removal) | Event re-queued on restart; knot re-runs — safe by idempotency |
| Crash between removal and commit (success path) | Event lost, but the artefacts remain on disk uncommitted — accepted (the commit is best-effort anyway) |
| Crash after a failure-path removal | Same as consume-on-failure today; no retry policy (System Reliability PRD territory) |

### The service loop peeks instead of pops

`spawn_process_strand_loop` replaces the `pop()`-based read with
`front()` (read head, no removal) + `notified()` wait; it processes via
`execute_with_pending` and breaks when `front()` is `None` and
`shutdown_signaled()`. `pop()` stays on the port (tests use it; it
remains a valid primitive) — only the service loop stops using it.

## The `step_knot` Lifecycle

`step_knot(config, event_spec) -> io::Result<()>` — a public entry
beside `start_knot`:

1. **Full startup, step options.** `run_startup(StartupOptions::step())`
   — the same sequence as the service (migration, config seeding, rig
   git init, discovery, watcher registration) except logs are **not**
   cleared. `StartupOptions { clear_logs: false, register_watchers:
   true }` — a multi-step session accumulates in the logs (tie-offs
   remain the durable record), but watchers are registered so in-cycle
   writes are captured. The service keeps `StartupOptions::service()`
   (`true/true`) — unchanged.
2. **Pipelines.** The config pipeline, event pipeline (debounce
   engine), and 5-second state writer start exactly as in the service.
   The process-strand loop is **not** spawned.
3. **Resolve the target event** (`step_execute_one`):
   - `--event` given: match via `queue.snapshot()` — (1) exact id
     (`.json` optional), (2) unique id prefix, (3) strand filename.
     No match → print the queue to **stderr**, `Err` (exit 1).
     Ambiguous prefix/filename → same.
   - Otherwise: `front()`. If `None`, wait up to **5× the debounce
     window** on `notified()` (so a just-touched strand can clear its
     debounce window); still empty → print `queue empty`, `Ok` (exit 0).
4. **Execute exactly one event** via `execute_with_pending` on
   `spawn_blocking` (the tokio task yields; same reason as the service
   loop). Prints `[step] processing event …` / `[step] event …
   processed` (or `failed: …` on stderr).
5. **In-cycle settle** (`step_settle`): a bounded wait of
   `max(5× debounce window, 250 ms)` after the execution. The file
   watcher (50 ms poll) delivers in-cycle file events — a dispatched
   consumer event, the agent modifying its own strand — with a delay;
   without the settle, the shutdown cascade would close the strand
   channel before the event is delivered and it would be lost.
6. **Graceful shutdown** — the same cascade as the service: drop the
   context (channel close → the debounce engine flushes remaining
   in-cycle events to disk + `push_shutdown`), abort the config
   handler, drain the `JoinSet` with the 5-second timeout (the state
   writer runs forever and is aborted by the timeout — every step
   takes ≥ 5 s), write `LoomStopped` to each loom-log.

Events dispatched **during** the step land in `tie-offs/<rig>/events/`
(flushed at shutdown) but are **not executed** — "one cycle's worth of
side effects, no further processing". `state.json` is written during
the step, so it reflects the post-step queue.

### Startup split

| | Service | Step |
|---|---|---|
| Log clear | yes (per-run scope) | **no** (multi-step session accumulates) |
| Watcher registration | yes | yes (in-cycle writes must be captured) |
| Process-strand loop | spawned | not spawned — one `execute_with_pending` |
| Shutdown cascade | on Ctrl+C | after the single execution (identical) |

`build_process_strand(ctx, queue)` is the shared `ProcessStrand`
construction used by both the service loop and `step_knot`.

### Rejected alternative — minimal-startup step

A step with no watchers, no debounce, no state writer was originally
attractive for determinism. Rejected: the point of stepping is to
observe a cycle unfold, and the agent's tie-off/dispatched-event
writes must trigger new queued events. Full pipeline + single
execution gives exactly one cycle's worth of side effects.

## CLI Grammar

`parse_args(&[String]) -> Result<CliCommand, String>` in `src/main.rs`
is a **pure function** (no env, no I/O, no process exit) — the grammar
is unit-testable without a subprocess:

```rust
enum CliCommand {
    Service { rig: Option<String> },
    Share { rig: String },
    Step { rig: Option<String>, event: Option<String> },
    Help,
    Version,
}
```

- `--version`/`-V`, `--help`/`-h` are global — first occurrence wins,
  anywhere.
- The first positional is a command (`share`/`step`) or the service's
  rig name. `step` takes **no positionals** (`knot step myrig` is a
  parse error pointing at `--rig` — the grammar stays unambiguous; the
  open question in the plan was decided "no").
- `--rig <v>` / `--event <v>` are only valid **after** `step` (in
  service mode they remain unknown-flag errors); flag order is free;
  missing values and duplicates are errors.
- `resolve_config` keeps the rig-discovery mapping, split into
  `resolve_service_config` (zero matches → implicit `rig/`) and
  `resolve_step_config` (zero matches → **error** — no implicit
  `rig/` creation; multiple → error).

**Exit codes (all commands):** `0` = success (for `step`: an event was
processed, or the queue was empty); `1` = error.

## Components

| Piece | Location | Role |
|---|---|---|
| `StrandEventQueue::front()` | `src/application/ports.rs` | Read the FIFO head **without** removing it (on-disk content) |
| `StrandEventQueue::shutdown_signaled()` | `src/application/ports.rs` | Expose the shutdown flag set by `push_shutdown` |
| `StrandQueueAccessor::delete()` | `src/domain/events.rs` | Explicit removal of the just-processed event (implemented on both queue adapters) |
| `DiskBackedEventQueue::front` / `delete` | `src/adapters/outbound/disk_event_queue.rs` | Scan → read head, no removal; `remove_event` by id |
| `InMemoryEventQueue::front` | `src/application/in_memory_event_queue.rs` | Same contract for the in-memory adapter |
| `ProcessStrand::execute_with_pending` | `src/application/usecases/process_strand.rs` | Late-removal entry point; threads `event_id` to the terminal points via `remove_pending_event` / `abort_with` |
| `StartupOptions` | `src/server.rs` | `service()` (`true/true`) vs `step()` (`false/true`) |
| `build_process_strand(ctx, queue)` | `src/server.rs` | Shared `ProcessStrand` construction (loop + step) |
| `step_knot` / `step_execute_one` / `resolve_step_event` / `step_head_event` / `step_settle` | `src/server.rs` | The step lifecycle: resolution, single execution, settle, shutdown cascade |
| `parse_args` / `CliCommand` | `src/main.rs` | Pure CLI grammar; `resolve_step_config` (stricter rig discovery) |

## Testing

| Test | Pins |
|---|---|
| `front_returns_head_without_removing`, `front_none_when_empty`, `front_fifo_order`, `front_reflects_on_disk_edits`, `shutdown_signaled_false_initially_true_after_push_shutdown` (unit, disk + in-memory queues) | Phase 1 port contract |
| `success_removes_event_before_git_commit` (`tests/late_removal.rs`, mock git versioner records call order) | Removal precedes `commit(...)`, after dispatch/log writes |
| `success_removes_event_file`, `agent_failure_removes_event_file`, `profile_not_found_removes_event_file`, `loom_not_found_removes_event_file`, `binary_skip_removes_event_file`, `unknown_kind_consumes_event` | One removal per failure path; file gone |
| `execute_without_pending_id_leaves_queue_untouched` | `execute()` (queue-less path) never touches the queue |
| `failing_event_consumed_once_then_loop_proceeds` | Poison-pill: a knot that always fails is consumed exactly once; no tight re-fail loop |
| `event_file_present_during_processing_gone_after` | Crash window: the event file exists **while the agent runs**, is gone after success |
| `front_loop_drains_queue_then_idles` | The front-based service loop drains a 2-event queue, then `QueueIdle` in rig-log |
| `step_processes_head_and_leaves_rest` (`tests/step.rs`, real adapters + mock CLI agent) | Exactly one agent run; head file removed; second event still in `events/`; no tie-off for it |
| `step_event_targets_specific` | `--event` naming the *second* event → only that one runs; head stays queued |
| `step_captures_dispatched_events_without_executing` | Producer dispatches mid-step → consumer event file in `events/` after the step; consumer tie-off **not** written |
| `step_writes_state_json` | Post-step queue in `state.json` (state writer ran during the step) |
| `step_does_not_clear_logs` | Two sequential steps; the second step's loom-log still contains the first step's `KnotCompleted` |
| `step_event_unknown_lists_queue_and_fails`, `step_empty_queue_noop`, `step_rig_flag_targets_named_rig`, `step_multiple_rigs_without_flag_is_error`, `step_zero_rigs_is_error` (binary-level, `CARGO_BIN_EXE_knot`) | CLI contract: stderr queue listing + exit 1; `queue empty` + exit 0; strict rig discovery |
| `parse_args_tests::*` (unit, table-driven) | Full grammar incl. `--rig`/`--event` only after `step`, positional-after-`step` error, global flags anywhere |

## Notes

- **Concurrency hazard — stepping while the service runs.** Two
  processes share the disk queue; `front()` + late removal is not
  cross-process atomic, so the same event can be read twice (double
  execution). Tolerable under idempotency but wasteful — the
  `knot-dispatch` skill states that `knot step` is for when the service
  is not running. A lock file is a future option, out of scope.
- **Every step takes ≥ 5 s.** The state-writer task runs forever, so
  the 5-second `JoinSet` drain always times out and aborts it (same as
  the service's Ctrl+C path). If step latency becomes a problem, the
  state writer could take a stop signal — follow-up.
- **Loom-log brackets per step.** Step mode runs full discovery, so
  each step appends `KnotRegistered`/`LoomStarted` and shutdown
  appends `LoomStopped`. Accepted: each step is a self-contained run.
- **Early-error paths skip the cascade.** An unknown `--event` returns
  before the context is dropped, so `LoomStopped` is not written — the
  bracket is unbalanced only on error exits, mirroring the service's
  startup-failure behaviour.
- **Empty-queue wait and settle are internal constants** (5× the
  debounce window; settle floored at 250 ms), not flags. Tests shorten
  the debounce window via `KNOT_TEST_DEBOUNCE_MS`.
- **The in-cycle settle is a timing guarantee, not an optimisation.**
  Disabling it makes `step_captures_dispatched_events_without_executing`
  pass on a fast dev machine (the post-dispatch git-commit tail happens
  to cover the watcher poll) but the capture becomes timing-dependent.
- **Pending events from older versions read identically** — the
  `events/*.json` schema is unchanged, so no queue migration was
  needed for 0.35.0.

## Related Documents

- [docs/concepts.md](../../docs/concepts.md) — Event Queue section
  (at-least-once semantics for users), Processing Flow
- [docs/release-notes.md](../../docs/release-notes.md) — v0.35.0 entry
- `knot-dispatch` skill — `knot step` workflow (flags, event
  resolution, direct queue write, observation steps)
- `knot-update` skill changelog — Knot 0.35.0 entry (no document
  migration)
- [design-event-dispatch.md](design-event-dispatch.md) — the dispatch
  side of the producer→consumer chain that stepping observes
