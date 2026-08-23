# Plan: `knot step` — Single-Event Stepping and Late Queue Removal

## Related PRD

This plan contributes to [Persistent Events — Disk-Backed Event Queue](../../prds/prd-persistent-events.md).

It delivers the PRD's "user control over queued work" goal via a CLI (complementing the
PRD's HTTP-interface stories) and revises one PRD goal: the queued event file is no longer
removed when the event is *popped* for processing — it is removed after the work is done
(late removal), so a crash mid-processing re-queues the event instead of losing it.

## Problem

Two gaps in how a user works with the event queue:

1. **The service is all-or-nothing.** `start_knot` runs until Ctrl+C, draining the queue
   continuously. There is no way to process exactly one event and stop, to target a
   specific queued event, or to inspect rig state between events. Debugging a misbehaving
   knot (wrong prompt, runaway event loop, event-enforcement follow-ups) requires letting
   the service process events back-to-back with no observation point between them.

2. **In-flight events are lost on crash.** `DiskBackedEventQueue::pop()` removes the event
   file *before* `ProcessStrand::execute` runs. A crash or Ctrl+C during a long agent run
   loses the event even though the knot's work (tie-off, git commit) has not happened yet.
   A restart silently discards the unprocessed work.

## Target

When this plan is done:

1. **A `knot step` command processes exactly one event, then exits.**

   ```
   knot step [--rig <rig-name>] [--event <event-filename>]
   ```

   - `--rig` targets a specific rig (same `./<name>` resolution as the service-mode
     positional). Without it, auto-discovery applies — but zero matches is an *error*
     (no implicit `rig/` creation), multiple matches is an error.
   - `--event` targets a specific queued event: exact event id (`.json` optional), unique
     id prefix, or strand filename. No match → list the queue, exit 1.
   - Without `--event`, the FIFO head is processed. Empty queue → print "queue empty",
     exit 0.
   - **Full startup, single execution**: legacy-layout migration, config seeding, rig git
     init, loom discovery, watcher registration, debounce engine, config pipeline, and the
     5-second state writer all run. Events that occur *during* the step (dispatched agent
     events, tie-off writes) are captured into the queue but **not executed**. After the
     single event, graceful shutdown: the debounce buffer is flushed to disk, pipelines
     drain, `LoomStopped` is written.
   - Exit codes: `0` = stepped an event or queue empty; `1` = error (unknown event,
     multiple rigs, processing failure).

2. **Late-removal (at-least-once) queue semantics for the normal service.** On success,
   `ProcessStrand` removes the event file as the **last step before the git commit** —
   dispatch, tie-off, loom-log entries, and event enforcement all happen first, and the
   commit captures everything including the removal. On failure/skip paths the event is
   removed at the point of failure (preserving today's consume-on-failure behaviour — no
   poison-pill retry loops). Invariant: *every* return from event processing removes the
   file exactly once; the only window in which the event survives a crash is
   "processing in flight" → restart re-queues → knot re-runs (safe by knot idempotency).

3. **The service loop peeks instead of pops.** `pop()` (delete-on-read) is replaced by
   `front()` (read head) plus the explicit removal inside processing. Shutdown detection
   moves from the `pop()` sentinel to `shutdown_signaled()`.

4. **CLI parsing is a pure, unit-testable function** (`parse_args` → command enum). No new
   dependencies.

5. **Docs, skills, PRD, and changelog are corrected** to the new semantics.

## Existing Tests

| Test | What it covers | Status |
|------|----------------|--------|
| `src/adapters/outbound/disk_event_queue.rs` unit tests | push/pop/snapshot/dedup/delete/pending_event/shutdown sentinel/crash survival | ✅ Green — defines current pop-removes semantics |
| `tests/persistent_queue.rs` | Full-cycle push/pop, restart survival, malformed files, on-disk modification | ✅ Green |
| `tests/pipeline.rs` | Strand lifecycle through real adapters + mock agent (create/modify/delete, agent failure, temp-file skips) | ✅ Green |
| `tests/rig_cli.rs` | Real binary via `CARGO_BIN_EXE_knot`: rig discovery, share, unknown-flag rejection | ✅ Green — template for `step` CLI tests |
| `tests/helpers.rs` | `ProcessStrandResult` builder (mock ports), `start_knot_with_config` + `KnotHandle`, state/log readers | ✅ Green |
| `src/application/debounce.rs` tests (incl. `shutdown_flush_uses_queue`) | Channel close → pending events flushed to queue → `push_shutdown` | ✅ Green — guarantees in-cycle events land on disk at step shutdown |
| `src/server.rs` composition tests | Startup sequence with real adapters (migration, config seeding, log clear, discovery order) | ✅ Green |
| `tests/smoke.rs` | End-to-end with mock CLI agent via `AppConfig::with_cli_path` | ✅ Green |

## Test Gaps

- No `front()`/peek — no test can read the queue head without consuming it.
- No test pins removal timing relative to the git commit (removal currently happens
  *before* processing, so it cannot even be pinned).
- No crash-window test: event still queued while processing is in flight (late removal).
- No CLI tests for a `step` command; `parse_args` does not exist (parsing is inline in
  `resolve_config`, untestable without process env).
- No test that events dispatched *during* processing are captured but not executed
  (`tests/pipeline.rs` only covers dispatch → reprocess inside a long-running service).
- No test that `step` leaves the rest of the queue intact, or that `--event` targets a
  non-head event.
- No test that repeated `step` invocations accumulate logs (step must not clear
  loom-log/rig-log).

## Phases

### Phase 1: `front()` and `shutdown_signaled()` on the queue port (TDD)

Extend the `StrandEventQueue` port (`src/application/ports.rs`):

- `front() -> Option<PendingEvent>` — read the head event (FIFO order) **without**
  removing it; returns the on-disk content (today's `pop()` minus the removal).
- `shutdown_signaled() -> bool` — expose the shutdown flag set by `push_shutdown`.

Implement in `DiskBackedEventQueue` (scan → read head, no `remove_event`) and
`InMemoryEventQueue`. `pop()` stays on the port (still used by tests; a valid primitive).

Unit tests (failing first):

- `front_returns_head_without_removing` — two events; `front()` twice returns the same
  head; `len()` unchanged.
- `front_none_when_empty`, `front_fifo_order`, `front_reflects_on_disk_edits`.
- `shutdown_signaled_false_initially_true_after_push_shutdown`.

### Phase 2: Late removal in `ProcessStrand` and the service loop (TDD, acceptance-level)

**Port change** — `StrandQueueAccessor` (`src/domain/events.rs`): add
`fn delete(&self, id: &PendingEventId) -> bool` so the use case can remove the event it
just processed. Implement on both queue adapters; update test mocks.

**Use-case change** — `ProcessStrand`
(`src/application/usecases/process_strand.rs`, `process_strand_helpers.rs`):

- New entry point `execute_with_pending(&self, pending: &PendingEvent) -> Result<(),
  PortError>`: converts to `StrandEvent` and calls a new internal
  `execute_inner(event, Some(&pending.id))`. The existing `execute(event)` delegates with
  `None` — every existing call site and test is untouched.
- Thread `event_id: Option<&PendingEventId>` to the terminal points:
  - **Success** (`handle_success`): remove the event file as the **last step before the
    git-commit block** — after dispatch, `KnotCompleted`, `StrandProcessed`, and event
    enforcement; before `git_versioning_port.commit`. Unconditional on success (even when
    the knot is not `git_versioned` or has no commit content).
  - **Failure / skip / early error** (`handle_failure` agent failure/timeout,
    `resolve_config_and_build` error, `validate_strand` skip, `LoomNotFound` / knot-not-
    found): remove before returning.
- **Invariant (pinned by tests): every return removes the event file exactly once, and on
  success the removal precedes the commit.**

**Loop change** — `spawn_process_strand_loop` (`src/server.rs`): replace the `pop()`-based
read with `front()` + `notified()` wait (same burst/idle `QueueIdle` logic); process via
`execute_with_pending`; break when `front()` is `None` and `shutdown_signaled()`.

Tests:

- Ordering test (mock git versioner that records call order + real/in-memory queue):
  `delete(event_id)` is recorded **before** `commit(...)`, and after the dispatch/log
  writes.
- One removal test per failure path (agent failure, profile-not-found, loom-not-found,
  binary skip): file gone, and the loop's next iteration does not re-fail on it.
- Poison-pill test: a knot that always fails → its event is consumed exactly once; the
  loop proceeds to the next event (no tight re-fail loop).
- Crash-window test: the mock agent runner inspects the queue mid-execution — the event
  file must still exist while the agent is running, and be gone after success.
- Loop integration (extend the `tests/pipeline.rs` pattern): the front-based loop drains a
  2-event queue then idles (`QueueIdle` in rig-log).
- Full `cargo test` green — the `execute()` path with `None` id never touches the queue.

### Phase 3: CLI — `step` command parsing (TDD)

Refactor `src/main.rs`:

- Pure `parse_args(&[String]) -> Result<CliCommand, String>`:

  ```rust
  enum CliCommand {
      Service { rig: Option<String> },
      Share { rig: String },
      Step { rig: Option<String>, event: Option<String> },
  }
  ```

- `--rig <v>` / `--event <v>` are only valid after `step` (in service mode they remain
  unknown-flag errors); flag order is free; missing values are errors. `resolve_config`
  keeps the rig-discovery mapping (`RigDiscovery`) but takes the parsed command as input.
- `print_usage` gains the `step` command, its options, and the exit-code contract.

Unit tests (table-driven): `step` alone, `--rig`, `--event`, both, reversed flag order;
missing values; unknown flag after `step`; `--rig` in service mode rejected; `share` and
the service positional unchanged; `--help` / `--version`.

### Phase 4: `step_knot` lifecycle — full startup, one execution, graceful stop (TDD, acceptance-level)

**Startup split** — `run_startup` (`src/server.rs`) gains a small options struct:

```rust
struct StartupOptions { clear_logs: bool, register_watchers: bool }
```

Service: `true/true` (unchanged). Step: `false/true` — no log clear (a multi-step session
accumulates; tie-offs remain the durable record) but watchers registered so in-cycle
writes are captured.

**Shared construction** — factor the `ProcessStrand` construction out of
`spawn_process_strand_loop` into `build_process_strand(ctx, queue)`, used by both the
service loop and step.

**`step_knot(config: AppConfig, event_spec: Option<String>) -> io::Result<()>`** — new
public entry beside `start_knot`:

1. `build_app_context` → `run_startup(StartupOptions { clear_logs: false,
   register_watchers: true })`.
2. Start the config pipeline, event pipeline (debounce engine), and state writer
   (5-second loop) — same as the service.
3. Resolve the target event:
   - `--event` given: match against `events/` — exact id (`.json` optional) → unique id
     prefix → strand-filename suffix (via `snapshot()`); then `pending_event(id)`. No
     match → print the queued events, exit 1.
   - Otherwise: `front()`. If `None`, wait up to 5× the debounce window on `notified()`
     (so a just-touched strand can clear its debounce window before step declares the
     queue empty); still empty → print "queue empty", skip processing.
4. Execute exactly one `execute_with_pending` (via `spawn_blocking`), printing the event
   id, loom/knot, and outcome.
5. Graceful shutdown — the same cascade as the service: drop the context (channel close →
   debounce engine flushes remaining in-cycle events to disk + `push_shutdown`), drain
   the `JoinSet` with the 5-second timeout, write `LoomStopped` to each loom-log.

Acceptance-level tests (real adapters + mock CLI agent via `with_cli_path`; lib-level via
`step_knot` and binary-level via `CARGO_BIN_EXE_knot` following the `tests/rig_cli.rs`
pattern):

- `step_processes_head_and_leaves_rest` — two queued events → exactly one agent run; head
  event file removed; second event still in `events/`; no tie-off for the second.
- `step_event_targets_specific` — `--event` naming the *second* event → only that one
  processed; the head remains queued.
- `step_event_unknown_lists_queue_and_fails` — exit 1; stderr lists the queued ids.
- `step_captures_dispatched_events_without_executing` — the producer knot dispatches an
  event to a consumer during the step → after the step, the consumer's event file is in
  `events/` (debounce flushed at shutdown) and the consumer's tie-off was **not**
  written.
- `step_writes_state_json` — `state.json` reflects the post-step queue (state writer ran
  during the step).
- `step_empty_queue_noop` — "queue empty", exit 0, no agent run, prompt exit.
- `step_does_not_clear_logs` — two sequential steps; the second step's loom-log still
  contains the first step's `KnotCompleted`.
- `step_rig_flag_targets_named_rig`; multiple rigs → error; zero rigs → error (no
  implicit `rig/` creation).
- Full `cargo test` green — the service path (default options) is unchanged:
  `tests/pipeline.rs`, `tests/smoke.rs`, `tests/rig_cli.rs`, and the composition startup
  tests all remain valid as-is.

### Phase 5: Documentation, skills, PRD, changelog

- `docs/concepts.md` — queue semantics: at-least-once; the event file is removed
  just-before-commit on success and at the point of failure on failure; in-flight events
  survive a crash and are re-queued.
- `knot-dispatch` skill — document `knot step` (single-event stepping, `--rig`,
  `--event`, empty-queue behaviour) as the manual trigger/observation tool.
- `knot-update` skill — changelog entry for the bumped version: new `step` command; queue
  removal timing changed (no document migration — pending events from older versions read
  identically).
- `project/prds/prd-persistent-events.md` — revise the "when an event is popped … its file
  is removed" goal to late-removal semantics; add a user story for manual stepping via
  the CLI.
- Version bump (MINOR `0.34.0 → 0.35.0`), release notes, design-document extraction, and
  global skill publication happen at plan completion via the project-plan-completion
  skill.

## Notes

- **Alternative considered and rejected — minimal-startup step** (no watchers, no
  debounce, no state writer): originally attractive for determinism, but rejected: the
  point of stepping is to observe a cycle unfold, and the agent's tie-off/dispatched-
  event writes must trigger new queued events. Full pipeline + single execution gives
  exactly "one cycle's worth of side effects, no further processing".
- **Crash windows.** Success path: crash between processing start and removal → event
  re-queued on restart, knot re-runs (idempotent by design). Crash between removal and
  commit → event lost but the artefacts remain on disk uncommitted (accepted
  consequence of the removal-before-commit ordering; the commit is best-effort anyway).
  Failure paths: removal at the point of failure — same consume-on-failure as today; no
  retry policy (that is System Reliability PRD territory).
- **Concurrency hazard — stepping while the service runs.** Two processes share the disk
  queue; `front()` + late removal is not cross-process atomic, so the same event can be
  read twice (double execution). Tolerable under idempotency but wasteful; the skill
  states that `knot step` is for when the service is not running. A lock file is a
  future option, out of scope here.
- **Loom-log brackets per step.** Step mode runs full discovery, so each step appends
  `KnotRegistered`/`LoomStarted` and shutdown appends `LoomStopped`. Accepted: each step
  is a self-contained run. Suppressing the brackets in step mode is a follow-up option if
  they prove noisy.
- **`pop()` stays on the port** (tests use it; it remains a valid primitive) — only the
  service loop stops using it.
- **Empty-queue wait.** 5× the debounce window (default 100 ms → 500 ms) is an internal
  constant, not a flag; tests shorten it via `KNOT_TEST_DEBOUNCE_MS`.
- **Open question (decide in Phase 4):** should `step` also accept the service-style
  positional rig (`knot step myrig`)? Default: no — `--rig` only, keeping the command
  grammar unambiguous.
