# Plan: Generic Task Management Tests

## Problem

`tests/task_management.rs` validates the concurrency cascade shutdown pattern, but every test is tightly coupled to Knot domain types — `AppConfig`, `spawn_server_with_shutdown`, loom directories, knot definitions, mock agent scripts, tie-off files, loom-logs, HTTP health checks. The tests prove the Knot pipeline works, but they don't document the raw tokio concurrency pattern in isolation.

A generic test file that captures only the concurrency primitives — `JoinSet`, `mpsc` channel cascade, cooperative drain, abort safety net — would:

1. Serve as living documentation of the pattern (readable without Knot context)
2. Catch regressions in the shutdown logic at the primitive level
3. Be reusable as a reference for any Rust service using the same pattern

## Target

A new `tests/generic_task_management.rs` that imports **only `tokio`** (zero Knot types) and validates the channel-cascade shutdown pattern through 10 focused tests covering:

- JoinSet child lifecycle and cooperative drain
- Channel closure propagation through chained stages
- Flush on channel close (pending work drains before exit)
- In-flight work completion during shutdown
- N-stage cascade propagation (3+ stages)
- Leaked sender prevention of shutdown (negative test)
- Oneshot trigger starting cascade
- Post-shutdown hook execution
- Abort as safety net for hung tasks
- Biased `select!` prioritising channel close

## Implementation Status: ✅ Complete (2026-06-07)

## Existing Tests

| Test File | What it covers | Status |
|-----------|---------------|--------|
| `tests/task_management.rs` | Knot-specific cascade shutdown (DebounceEngine → ProcessStrand) via full server | ✅ Green — 5 tests (1 ignored) |
| `src/application/debounce.rs` unit tests | Debounce engine isolation (single event, rapid events, different files) | ✅ Green — 4 tests |
| `tests/helpers.rs` | Shared `spawn_server_with_shutdown`, `wait_for_port`, mock agents | ✅ Shared infrastructure |

## Test Gaps

- No test of the concurrency pattern in isolation (all tests require Knot domain, filesystem, HTTP)
- No test of the "leaked sender" negative case (proving why `output_tx` must not escape the task)
- No test of the abort safety net (what happens when a task genuinely hangs)
- No test of 3+ stage cascade (Knot uses only 2 stages)

## Phases

### Phase 0: Scaffolding and core drain test
- [x] Create `tests/generic_task_management.rs`
- [x] Define `spawn_stage<T>()` helper: a generic stage that `recv()`s from input, does configurable async work, flushes pending on close, sends to output
- [x] Write `tasks_drain_on_shutdown` — two-stage pipeline (A → B), `JoinSet`, `while let Some join_next()` — all tasks complete cooperatively, no abort

### Phase 1: Channel cascade and flush
- [x] Write `channel_closure_propagates_cascade` — chain 2 channels, drop last sender, verify downstream `recv() → None` propagates
- [x] Write `flush_on_channel_close` — stage holds pending items; on `recv() → None` flushes them before returning; verify flushed items received
- [x] Write `leaked_sender_prevents_shutdown` — extra `Sender` clone held outside pipeline → `recv()` never yields `None` → task hangs → timeout proves the hang

### Phase 2: In-flight work and multi-stage
- [x] Write `in_flight_work_completes` — stage is mid-`await` (sleep) when upstream closes → current work finishes → stage exits
- [x] Write `multiple_stages_drain_sequentially` — 3-stage pipeline (A → B → C), verify exit order matches cascade direction
- [x] Write `oneshot_trigger_starts_cascade` — oneshot signal stops ingestion, channel closures propagate, all stages exit

### Phase 3: Post-shutdown and safety net
- [x] Write `post_shutdown_hook_executes` — callback fires only after `join_next()` loop completes
- [x] Write `abort_is_safety_net` — one stage intentionally hangs (never returns) → `JoinSet` Drop with timeout → hung task is aborted, others completed cleanly
- [x] Write `biased_select_prioritises_shutdown` — `select! { biased; }` with channel close branch fires at next `.await` when channel closes during active work
- [x] Run `cargo test --test generic_task_management` — all 10 tests pass

## Notes

- Executed via plan-orchestrator skill with `qwen3-27b` (llama-workhorse provider)
- Phase 0 required `--skip-baseline` due to pre-existing compile errors in unrelated test files (`tests/pipeline.rs`, `tests/rig_lifecycle.rs`)
- Phase 3 sub-agent was killed due to stall timeout; all work was already written and passing — recovered by committing its dirty tree
- All 10 tests pass: `cargo test --test generic_task_management`
