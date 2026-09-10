# Plan 087: Self-Continuation for Event-Source Knots + Queue-Wait-Exempt Budget

## Related Plans

Builds on and **completes**
[086 Graceful Task Handoff](../086-graceful-task-handoff/graceful-task-handoff-plan.md)
(the continuation chain, `HANDOFF_NOTE`, the two bounds, and the
"delivered into the knot's existing input" self-continuation model —
which 086 **scoped to filesystem-strand-triggered knots** in v1). This
plan:

1. **Extends the self-continuation to event-source knots**
   (`strand-dir: event:<producer>:<EventId>`), which v1 left out
   (`dispatch_self_continuation` returns `Ok(None)` for them — "event
   source knots are a later extension").
2. **Refines 086's global-deadline model** from an *absolute*
   `batch-deadline-epoch` (which subtracts queue wait) to a **batch
   total execution budget carried as remaining seconds**
   (`budget-secs`), which only execution decrements — while preserving
   086's core invariant: **a handoff buys a fresh context, never a
   fresh budget.**

It rides on, unchanged:

- [084 Graceful Completion](../084-graceful-completion/graceful-completion-plan.md)
  — the water-mark steer / `HANDOFF_NOTE` / stop-resume that produces
  the `TasksIncomplete` declaration.
- [079](../079-context-overflow-compact-and-continue/) /
  [080](../080-overflow-error-fail-fast/) — compaction recovery and the
  `ContextLimitReached` "Tier 2" safety net (the last-resort fallback
  when the chain is exhausted).
- [065 Persistent Event Queue](../065-persistent-event-queue/) — the
  durable `tie-offs/<rig>/events/` queue the continuation lands in.
- [068 Rig Repo Separation](../068-rig-repo-separation/) — the two-repo
  git model: the project's per-turn commit is the unified audit trail
  (agent work + the entire `tie-offs/<rig>/` runtime tree in one
  commit; only the source-only `rig/` repo is unstaged). That
  project-commit trail is what makes *replay-by-commit* possible.
- The **late-removal / at-least-once queue** model
  (`execute_with_pending`) whose idempotency makes the chain safe.

## Problem

Two v1 limitations, both in the self-continuation path
(`src/application/usecases/process_strand_helpers.rs`):

1. **Event-source knots do not auto-continue.** The self-continuation
   is dispatched by `dispatch_self_continuation`, which resolves the
   target directory from `knot.strand_source`:

   ```rust
   let strand_dir = match &knot.strand_source {
       StrandSource::Filesystem(path) => path.clone(),
       _ => {
           // v1: task-bearing knots are filesystem-strand-triggered.
           // Event-source knots are a later extension.
           return Ok(None);
       }
   };
   ```

   For an event-source knot the continuation file is never written, so
   the batch **pauses with work remaining**. The knot still gets the
   graceful wrap-up (water-mark steer → `HANDOFF_NOTE` → commit + state
   + the `TasksIncomplete` loom event + the `[task-loop] handoff`
   service-log line all still fire — see *Observability wrinkle* below),
   but the chain does not re-enter. Re-entry falls back to the knot's
   explicit event chain (an upstream producer must re-emit) or the 080
   `ContextLimitReached` safety net.

2. **The absolute deadline subtracts queue wait.** 086 stamps
   `batch-deadline-epoch = now + profile_timeout` at the first handoff
   and inherits it verbatim. At dequeue the code computes
   `remaining = deadline - now` (where `now` is the *dequeue* time) and
   uses it as the runner timeout:

   ```rust
   // process_strand_helpers.rs ~216-236
   let remaining = deadline.saturating_sub(now);   // now == dequeue time
   if remaining < MIN_REMAINING_SECS { degenerate_deadline_tieoff(...) }
   effective_timeout = Some(Duration::from_secs(remaining));
   ```

   If a lengthy queued event runs **between** the handoff and the
   continuation's dequeue, that wall-clock time erodes `remaining`. The
   knot loses budget to queue wait it did not cause.

## Design input (locked-in)

Three decisions fixed before this plan:

- **D1 — No self-subscription / no dedicated TaskIncomplete queue.** The
  `TasksIncomplete` continuation is *interim status of the explicit
  event*, so rather than add a mechanism (a rig-wide TaskIncomplete
  queue, or a second `strand_source` on the knot), the continuation is
  **delivered into the knot's existing input (its inbox)** — the same
  input the parent event used. "The parent event determines the target
  inbox; the tie-off is appended to the target knot anyway." The
  existing watcher (already bound to that dir for that knot) picks it up
  like any other strand-with-event-context. This is exactly 086's
  "knot-scoped self-match, delivered into the existing input" — it just
  needs to be completed for event-source knots.
- **D2 — Two bounds; queue wait excluded.** The run is bounded by
  (a) **iterations** (`continuations`, capped at `MAX_CONTINUATIONS`)
  and (b) **a batch total execution budget**: one `profile_timeout`
  for the whole batch, all hops included. The stamp carries the
  **remaining duration in seconds** (`budget-secs`); each handoff
  subtracts only that hop's execution — queue wait never subtracts.
  Example (seconds): 1800s profile, first iteration executes 600s →
  the continuation carries `budget-secs: 1200`; however long it waits
  in the queue, the next hop's runner timeout is 1200s.
- **D3 — Order + durable queue + commit.** Write continuation events to
  the durable queue (FIFO, **append to the end** — no front-insert)
  because it makes the chain **replayable by navigating the project's
  git commits** (the runtime tree is committed with the project —
  see 068).
  Order: water-mark fires → `TasksIncomplete` injected → tie-off
  provided with `occurred: true` → continuation event written to the
  **original event source** carrying the remaining budget + iteration
  count → **commit before the next queued item is taken**.

## Target — behavioural contract

- An **event-source** knot whose context crosses the water-mark and whose
  work remains (`TasksIncomplete occurred: true`) gets a **stamped
  continuation delivered into its own event dispatch dir**, re-triggers
  through its existing watcher, and resumes the batch — bounded by the
  same two caps as a filesystem-strand knot. No new subscription, no
  new queue, no second `strand_source`.
- The batch's **total execution** is bounded to one `profile_timeout`
  for the whole batch (all hops, including hop 1); **queue wait does not
  count**. A lengthy event between the handoff and the continuation's
  dequeue does not shrink the continuation's budget.
- The continuation is a **durable, git-versioned artifact** in the
  project's runtime tree (the event dispatch dir under `tie-offs/<rig>/`;,
  the knot's strand dir — a project path — in the filesystem case),
  captured by the project's per-turn commit and replayable by
  navigating the **project's** commits.
- Filesystem-strand behaviour is **preserved** (continuation still lands
  in the knot's own strand dir).

## The model

### How the two bounds are tracked

Both bounds live as **Knot-owned stamps** on the continuation event file's
front-matter (the agent never touches them; the agent never sees a clock —
086's state-split, unchanged).

**Bound 1 — Iterations (`continuations: N`).**

- Fresh dispatch (no stamp): `N = 0`.
- Each handoff: `N + 1`.
- Cap: if `N + 1 > MAX_CONTINUATIONS` (global `10`), suppress the
  continuation and record `BatchIncomplete (reason: caps)`. (Existing
  behaviour — unchanged.)
- **Time-independent**: a lengthy queued event does not change `N`; only
  hops that actually execute increment it. This bound is unaffected by
  D2.

**Bound 2 — Batch total execution budget (`budget-secs: B`).**

This replaces 086's absolute `batch-deadline-epoch` **for enforcement**
(see *Backstop* below for the absolute value). The budget is the
**total execution duration** of the whole batch — `profile_timeout`
seconds (the profile's per-invocation timeout; `300s` default when
unset), all hops included. The stamp carries the **remaining duration in
seconds** only; "consumed" means *executed*, and only execution consumes
— queue wait never does.

- **First handoff** (hop 1 completes, work remains):
  `B = profile_timeout - execution_secs(hop 1)`. Example: 1800s
  profile, hop 1 executes 600s → the continuation carries
  `budget-secs: 1200` (20 min).
- **Continuation dequeue**: the runner timeout is **`B` directly** — the
  stored remaining seconds, applied in full. **Queue wait is not
  subtracted** (the queue wait is `dequeue_time - handoff_time`; we
  simply do not fold it into the budget): however long the continuation
  waited, it still gets its full `B`. If `B < MIN_REMAINING_SECS` (`5s`),
  the hop is a **degenerate deferral**: no session spawned, a
  Knot-authored `Failed` tie-off is written, `BatchIncomplete (reason:
  deadline)` is recorded.
- **Continuation handoff** (hop completes, work remains):
  `B' = B - execution_secs`, where `execution_secs` is this hop's
  measured run time (dequeue → handoff, wall-clock; locked decision —
  see Risks). `B'` is stamped onto the next continuation.

**Invariant (the locked budget model):**

```
Σ (execution_secs over all hops, including hop 1)  ≤  profile_timeout
```

The batch gets **one total execution budget** — `B` only ever
*decreases* (by execution time), it is never reset to a full
`profile_timeout` at any handoff. This extends 086's "never a fresh
budget" rule to the whole batch: 086 already gave the continuation chain
(from the first handoff on) one shared budget; 087 additionally counts
hop 1's execution against that same budget and *excludes* queue wait
(086 subtracted it). 086's feared "silent per-hop budget reset" is
**not** reintroduced, because the budget decays by execution time
rather than being recomputed from `now + profile_timeout`.

**Backstop (staleness) + observability absolute value.** Keep a single
absolute `batch-start-epoch` T0 (when the batch's first handoff
happened) as a stamp. The `LoomEvent::TasksIncomplete` /
`BatchIncomplete` records carry `budget-secs` (remaining) +
`batch-start-epoch` (origin) so a reviewer can reconstruct both the
remaining budget and the batch's origin.

Because queue wait is exempt, `budget-secs` never decays while the
continuation waits — a hop with 20 minutes left still has 20 minutes
tomorrow morning. The optional staleness backstop adds one dequeue-time
check: if `now > T0 + k·profile_timeout` (k ≫ 1, e.g. 4), the hop is a
**degenerate deferral** — no session, `BatchIncomplete (reason:
backstop)` — i.e. **do not run the remainder** of a batch that went
stale while its continuation waited. *Needs your call — flagged*:
adopt the backstop (default `k = 4`) in this plan, or leave it future.

### Continuation event front-matter (schema)

```yaml
---
event-id: TasksIncomplete
target-knot: <knot-id>          # the knot this continuation re-enters
timestamp: <handoff time>
continuations: N                # Bound 1 — iterations
budget-secs: B                  # Bound 2 — remaining execution budget (seconds; only execution decrements)
batch-start-epoch: T0           # absolute origin (first handoff; backstop + observability only)
background-additional: |       # accumulated, hop-labelled (086)
  [hop N-1] ...
  [hop N] ...
---

## Accumulated Background
...
## Handoff
<tie-off body — pointer to checklist + committed state>
## Next Task Context
<operational brief for the next session>
```

Changes vs 086's schema: `batch-deadline-epoch` (a wall-clock deadline
eroded by queue wait) is replaced by `budget-secs` (remaining seconds,
eroded only by execution) + `batch-start-epoch` (absolute origin, used
only for the backstop check and observability).

### Delivery into the existing input (D1)

The continuation file is written to the **knot's existing input dir** —
the dir the watcher already watches, bound to `(loom_id, knot_id)`:

- **Filesystem** (`StrandSource::Filesystem(path)`): write to `path`
  (the watched strand dir). *Current behaviour — preserved.*
- **Event-source** (`StrandSource::EventUri { event_id, .. }`): write to
  the event dispatch dir
  `derive_runtime_root(rig_dir).join(loom_id).join(event_id)` — the same
  dir `FileSystemEventDispatcher::dispatch` writes to and the same dir
  `ensure_event_uri_watch` watches (bound to this knot). The existing
  watcher fires `StrandEvent::Created { loom_id, knot_id, strand_path }`
  for **this knot** → debounce → durable queue → re-run.

This is a **knot-scoped** delivery: the watch is bound per
`(loom_id, knot_id)`, so the continuation re-triggers the knot that
produced it — the cross-fire a loom-level subscription would cause is
eliminated (086's knot-scoped matching). See *Shared event-id* in Risks
for the one edge case.

### Order + durable queue + commit (D3)

The continuation is written to the **original event source** (the knot's
inbox). It then enters the durable queue through the **existing**
watcher → debounce → `DiskBackedEventQueue` path — naturally **appended to
the end** (FIFO by `timestamp_ms` filename), no front-insert.

Processing order (inside `execute_with_pending` / the success path):

1. Agent runs; water-mark stop-resume delivers `HANDOFF_NOTE`; tie-off
   written with `TasksIncomplete occurred: true`.
2. **Self-continuation**: `dispatch_self_continuation` writes the stamped
   continuation file to the knot's existing input (strand dir **or**
   event dispatch dir) with `continuations: N+1`, `budget-secs: B'`,
   `batch-start-epoch: T0`, accumulated `background-additional`.
3. `KnotCompleted`, `StrandProcessed`, event enforcement.
4. Late removal of the *current* pending event (`remove_pending_event`).
5. **Commit** — the per-turn commit discipline runs. The project
   `git_versioner` commit (`git add -A` at `repo_root`, then
   `git reset -q -- rig/`) captures the agent's edits to the code
   **and the entire rig runtime tree**: the tie-off, the loom-log
   entries, the queue changes, the state snapshot, and **the
   continuation file** now on disk. The rig's own git repo (plan 068)
   is source-only — looms, knots, profiles, config — and is excluded
   from this commit; it never captures runtime artifacts.
6. The process-strand loop peeks the **next** queued item (the
   continuation, once the watcher/debounce has queued it) and processes
   it.

The commit therefore lands **before the next queued item is taken**, and
the continuation is a durable artifact in the **project** repo —
**replay = navigate the project's commits** (the continuation file +
the stamped queue entry appear in the commit, so a reviewer can step
through the batch hop by hop).

### Observability wrinkle (fixed by this plan)

Today, for an event-source knot the code records
`LoomEvent::TasksIncomplete` (`continuations: incoming+1`) **and** writes
the `[task-loop] handoff (knot=…, continuations=N, …)` service-log line
**before** calling `dispatch_self_continuation` — which then returns
`Ok(None)`. The log/event thus *claims* a handoff/continuation that never
materialized into a file. Because D1 makes the delivery succeed for
event-source knots, the claim becomes true; the log/event now always
corresponds to a written continuation. (If a continuation is suppressed
by a cap, it is suppressed *before* the `LoomEvent`/log line — see
Phase 2.)

## Phases

### Phase 0 — Batch total execution budget (Bound 2)

**Files:** `src/application/usecases/process_strand_helpers.rs`,
`src/application/usecases/process_strand.rs` (if `ResolvedExecution`
changes), `src/domain/events.rs` (loom-event fields).

1. `ResolvedExecution`: replace/augment `batch_deadline_epoch: Option<u64>`
   with `budget_secs: Option<u64>` + `batch_start_epoch: Option<u64>`.
2. `read_continuation_stamps`: read `budget-secs` + `batch-start-epoch`
   (fall back to `batch-deadline-epoch` for any pre-087 continuation
   files still on disk, deriving `budget_secs = deadline - now` at
   read time — a one-time compatibility shim, removable after a release).
3. **Dequeue check** (the `~216-236` block): timeout = `budget_secs`
   **directly** (drop the `deadline - now` subtraction). If
   `budget_secs < MIN_REMAINING_SECS` → `degenerate_deadline_tieoff`.
   If the backstop is adopted (see *Backstop*): also degenerate-defer
   with `BatchIncomplete (reason: backstop)` when `now >
   batch_start_epoch + 4·profile_timeout`.
4. **Capture the hop start** so `execution_secs` can be computed at
   handoff: record `dequeue_epoch` (Unix seconds) at the top of
   `execute_with_pending` / `execute_inner`, thread it into the success
   path (e.g. via `ResolvedExecution`).
5. `dispatch_self_continuation`: compute `B' = B - execution_secs` and
   stamp `budget-secs: B'` + `batch-start-epoch: T0` (inherited, not
   recomputed). First handoff: `B = profile_timeout -
   execution_secs(hop 1)`, `T0 = now` (the batch's first handoff).
6. `degenerate_deadline_tieoff`: take the budget/start-epoch for its
   `BatchIncomplete` record.
7. `LoomEvent::TasksIncomplete` / `BatchIncomplete`: carry
   `budget_secs` + `batch_start_epoch` (replacing/alongside
   `deadline_epoch`); update the 082 system-event payload + service-log
   line to print `remaining=B's` (budget) alongside `hop=N/10`.

### Phase 1 — Event-source delivery (D1)

**Files:** `src/application/usecases/process_strand_helpers.rs`.

1. In `dispatch_self_continuation`, replace the v1 `return Ok(None)`
   short-circuit with an event-source target resolution:

   ```rust
   use crate::domain::knot_file::derive_runtime_root;
   let target_dir = match &knot.strand_source {
       StrandSource::Filesystem(path) => path.clone(),
       StrandSource::EventUri { event_id, .. } =>
           derive_runtime_root(&ps.rig_dir).join(loom_id.0).join(event_id),
   };
   ```

   (All inputs are already in scope: `ps.rig_dir`, `loom_id`, and the
   destructured `event_id` field — the same `derive_runtime_root` +
   `EventUri { event_id }` shape `ensure_event_uri_watch` and
   `FileSystemEventDispatcher::dispatch` already use.)
2. Keep the existing `create_dir_all` + `fs::write` — the file lands in
   the watched dir and is picked up by the existing watcher.
3. The `[task-loop] handoff` service-log line gains a
   `source=filesystem|event` tag so the two delivery paths are greppable.

### Phase 2 — Order, commit, and suppression ordering (D3 + observability)

**Files:** `src/application/usecases/process_strand_helpers.rs`.

1. **Confirm** the continuation file is written to disk before the
   per-turn commit runs (it already is — step 2 before step 5 in the
   order above). Note the split explicitly: the project `git_versioner`
   commit (`git add -A` + `git reset -q -- rig/`) **captures** the
   continuation — the event dispatch dir
   (`tie-offs/<rig>/<loom>/<event-id>/`) is inside the project repo by
   construction, and the knot's strand dir (filesystem case) is a
   project path; the rig's own git repo (068) is source-only and never
   contains runtime artifacts. The test asserts the continuation file
   appears in the project's commit after the turn.
2. **Reorder suppression**: when a cap or an exhausted budget suppresses
   the continuation, record the `BatchIncomplete` and **skip** the
   `LoomEvent::TasksIncomplete` (continuations: N+1) + `[task-loop]
   handoff` line — so the log never claims a hop that did not happen.
   (Today the event/line are written unconditionally for
   `occurred: true`.)
3. Keep the commit-before-next-item order intact (it is the existing
   late-removal → commit → loop-peek sequence).

### Phase 3 — Tests

**New/updated** (mirror the existing 086 test seams in
`process_strand_helpers.rs`, `process_strand.rs`,
`disk_event_queue.rs`, `mod_watchers.rs`):

| Test | Covers |
|------|--------|
| Filesystem continuation still lands in the strand dir | D1 preservation |
| **Event-source continuation lands in `tie-offs/<rig>/<loom>/<event-id>/`** and re-triggers the same knot | D1 (the v1 gap) |
| **Queue-wait exemption**: continuation dequeued after a long gap keeps its full `budget-secs` (not `budget - gap`) | D2 (the core fix) |
| Budget decays by `execution_secs` across hops; `Σ exec ≤ profile_timeout` | D2 invariant |
| Iteration cap: hop that would exceed `MAX_CONTINUATIONS` → `BatchIncomplete (caps)`, no continuation file | Bound 1 |
| Budget exhaustion: `budget-secs < MIN_REMAINING_SECS` at dequeue → degenerate `Failed` tie-off + `BatchIncomplete (deadline)` | Bound 2 |
| Continuation file captured in the project's per-turn commit (replay-by-commit source; rig repo stays source-only) | D3 |
| Suppression ordering: on cap/budget-exhaustion, no spurious `TasksIncomplete`/handoff log | observability fix |
| Compatibility shim: a pre-087 file with only `batch-deadline-epoch` still resolves a budget | migration |
| FIFO: a continuation written to the inbox is queued **after** any already-queued events (append, not front) | D3 ordering |

## Risks / open questions

- **`execution_secs` source — decided (wall-clock, dequeue → handoff).**
  `execution_secs = handoff_epoch - dequeue_epoch` (total processing
  time for the hop, including Knot overhead). The runner-metadata
  variant (agent-invocation duration only) is a future refinement; both
  satisfy the invariant.
- **Shared event-id (multiple consumers).** The event dispatch dir
  `tie-offs/<rig>/<loom>/<event-id>/` is per `(loom, event-id)`, not per
  knot. If *another* knot in the same loom subscribes to the same
  `event-id`, it watches the same dir, so a continuation written there
  would also re-trigger *it* (identical to how a normal dispatch fans
  out — see `dispatch_fan_out_two_consumers_same_event`). **Checked
  2026-09-10 against this repo's rig:** two looms (`new-loom`,
  `workflow-loom`), each with exactly one filesystem-strand knot
  (`src/new`, `src/workflow`); no loom has two knots subscribed to the
  same event-id, and no event-source knot exists at all — so the
  cross-fire case does not arise here. **Decision (locked):** accept
  this (document it) in v1; a knot-scoped subdir would change the
  watcher contract and is out of scope.
- **Debounce coalescing.** The continuation enters the queue via the
  watcher/debounce, so a burst of handoffs on the same file coalesces
  (at-most-one `PendingEvent` per file, `push_or_replace`). For a serial
  single-knot batch this is a non-issue; noted for completeness.
- **`notified()` wake.** The continuation is picked up through the
  existing watcher → debounce → `push`, which arms the `notified()`
  permit the loop already relies on; no new wake path is introduced.
- **Backstop drift.** Because queue wait is excluded, a pathological
  chain (each hop just under the cap, long queue waits) can span a large
  wall-clock span even though total *execution* ≤ `profile_timeout` —
  e.g. a batch started yesterday whose last hop still has 20 minutes
  left this morning. The `MAX_CONTINUATIONS` cap bounds the hop *count*,
  not the span. The concrete mechanism (a dequeue-time staleness check
  against `batch-start-epoch + k·profile_timeout`): see the *Backstop*
  subsection above — needs your call.
- **Migration.** Any continuation files already on disk from a 086 build
  carry `batch-deadline-epoch` only. The Phase 0 shim (derive
  `budget_secs = deadline - now` at read) keeps them working for one
  release; after that the shim is removed.

## Out of scope

- Front-insertion / priority queueing (D3 explicitly chooses FIFO
  append-to-end).
- A dedicated rig-wide `TasksIncomplete` queue or a second
  `strand_source` on a knot (D1 explicitly rejects both).
- Changes to the water-mark trigger, `HANDOFF_NOTE`, or the 079/080
  overflow/safety-net ladder.
