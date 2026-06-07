# ADR-003: Channel-Cascade Shutdown Pattern

**Date**: 2026-06-07
**Status**: Accepted

## Context

Multi-stage async pipelines (file-watcher → debounce → agent → sink) need a reliable shutdown mechanism. When the service stops — SIGTERM, test end, HTTP shutdown endpoint — every stage must drain in-flight work and exit, leaving no orphan tasks.

The naive approaches each fail in a different way:

- **`tokio::spawn` per stage** — tasks are children of the runtime, not the server. Dropping the server task leaves orphan tasks that block runtime shutdown.
- **Single `join_next()` call** — collects only the first completed task, then dropping the `JoinSet` aborts remaining stages mid-work, producing inconsistent state.
- **Immediate `JoinSet` drop** — aborts all tasks at the next `.await`, cutting work short (e.g., agent execution interrupted, tie-off not written).

The correct approach uses **channel closure as the shutdown signal**. Each stage sits in a `recv().await` loop. When the upstream sender is dropped, `recv()` returns `None`, the stage flushes any buffered work, and exits. Its own sender drops, closing the downstream channel, and the cascade continues.

### In Knot

The Knot server implements a two-stage cascade: `NotifyEventSource` → `DebounceEngine` → `ProcessStrand`. See [ADR-002](adr-002-server-child-tasks.md) for the Knot-specific wiring. This ADR captures the generic pattern that applies to any Rust/tokio service with multi-stage async pipelines.

## Decision

All background pipeline tasks use a **channel-cascade shutdown** pattern built on four primitives:

1. **`JoinSet`** — owns all pipeline tasks as children. Dropping the set aborts remaining tasks (safety net only).
2. **`mpsc` channel closure** — `recv() → None` when all senders are dropped; this is the shutdown signal.
3. **Cooperative drain** — `while let Some(res) = join_set.join_next().await` loop collects all tasks.
4. **No leaked senders** — each stage's output sender lives only inside its task. If any sender escapes, the channel never closes and the stage hangs.

### Architecture Overview

```
Source (ingestion)
    │
    ▼
┌──────────┐   mpsc   ┌──────────┐   mpsc   ┌──────────┐
│ Stage A  │ ────────>│ Stage B  │ ────────>│ Stage C  │
│          │   ch_0   │          │   ch_1   │          │
│ recv()   │          │ recv()   │          │ recv()   │
│ → work   │          │ → work   │          │ → sink   │
│ → flush  │          │ → flush  │          │ → exit   │
└──────────┘          └──────────┘          └──────────┘
     │                     │                     │
     └─────────────────────┼─────────────────────┘
                           │
                    ┌──────┴──────┐
                    │   JoinSet   │
                    └─────────────┘

Shutdown sequence:
  1. Drop source sender
  2. ch_0 closes → Stage A recv()→None → flush → exit → drops ch_1 sender
  3. ch_1 closes → Stage B recv()→None → flush → exit → drops ch_2 sender
  4. ch_2 closes → Stage C recv()→None → flush → exit
  5. join_next() loop collects all three tasks (all Ok)
  6. JoinSet drained — all tasks completed cooperatively
```

### Key Patterns

**1. No leaked senders (single point of failure)**

Each stage creates its output channel internally and moves the `Sender` into the spawned task. The caller receives only the `Receiver`:

```rust
fn spawn_stage(input_rx: Receiver<T>, set: &mut JoinSet<()>) -> Receiver<T> {
    let (output_tx, output_rx) = mpsc::channel::<T>(capacity);
    set.spawn(async move {
        // output_tx lives ONLY here — dropped when task exits
        while let Some(item) = input_rx.recv().await {
            let _ = output_tx.send(process(item)).await;
        }
    });
    output_rx  // only receiver returned to caller
}
```

If any `Sender` clone escapes the task scope, the channel never closes and the downstream stage hangs forever. This is the most common bug in cascade pipelines.

**2. Flush on channel close**

A buffered stage must drain its pending buffer when `recv()` returns `None`:

```rust
while let Some(item) = input_rx.recv().await {
    pending.push(item);
    if pending.len() >= capacity {
        for buffered in pending.drain(..) {
            let _ = output_tx.send(buffered).await;
        }
    }
}
// Channel closed — flush remaining
for buffered in pending.drain(..) {
    let _ = output_tx.send(buffered).await;
}
```

Without this, any items sitting in the buffer when the channel closes are lost.

**3. Sequential drain with `join_next()` loop**

```rust
while let Some(res) = join_set.join_next().await {
    if let Err(e) = res {
        eprintln!("Task failed: {e}");
    }
}
// All tasks completed — safe to run post-shutdown hooks
```

A single `join_next()` call collects only the first task. The loop ensures all tasks are awaited.

**4. `JoinSet` Drop is a safety net**

Cooperative drain is the primary shutdown mechanism. If a task genuinely hangs (bug, deadlock), the `join_next()` loop never completes for that task. `join_set.abort_all()` or dropping the set terminates hung tasks:

```rust
let timeout = tokio::time::timeout(shutdown_limit, async {
    while let Some(res) = join_set.join_next().await {
        // collect completed tasks
    }
}).await;

if timeout.is_err() {
    // Some tasks didn't complete — abort remaining
    join_set.abort_all();
}
```

**5. `biased select!` for shutdown priority**

When a stage uses `select!` with multiple branches, `biased` ensures the channel-close branch is checked first:

```rust
loop {
    tokio::select! {
        biased;

        maybe_item = rx.recv() => {
            if let Some(item) = maybe_item {
                // process
            } else {
                break; // channel closed — exit immediately
            }
        }
    }
}
```

Without `biased`, the work branch may be polled before the close branch, delaying shutdown by one work cycle.

**6. Post-shutdown hooks after drain**

Any cleanup (writing shutdown markers, closing connections) must run **after** the `join_next()` loop completes, not during or before:

```rust
// Drain all tasks
while let Some(res) = join_set.join_next().await { /* ... */ }

// Post-shutdown — runs only after all tasks completed
write_shutdown_markers().await;
```

### Implications for Design

- **Pipeline stages are stateless about shutdown** — a stage doesn't need to know about shutdown; it simply exits when `recv()` yields `None`.
- **Shutdown is single-directional** — signal flows upstream→downstream via channel closure. No reverse signalling needed.
- **In-flight work always completes** — a task that has already pulled an item from the channel finishes processing it before looping back to `recv()`.
- **Testing is straightforward** — the pattern uses only `tokio` primitives. Tests can validate each property (drain, flush, cascade, abort, biased select) without domain-specific types.

### Testing Strategy

The pattern is validated by `tests/generic_task_management.rs` — a test file that imports **only `tokio`** (zero domain types). Each test isolates one property:

| Test | Property Verified |
|------|------------------|
| `tasks_drain_on_shutdown` | Two-stage pipeline drains cooperatively via `join_next()` loop |
| `channel_closure_propagates_cascade` | Dropping last sender propagates `None` downstream through chained stages |
| `flush_on_channel_close` | Buffered items flush before stage exits on `recv() → None` |
| `leaked_sender_prevents_shutdown` | Extra `Sender` clone prevents channel closure (negative test) |
| `in_flight_work_completes` | Stage mid-`await` finishes current item before exit |
| `multiple_stages_drain_sequentially` | 3-stage cascade exits in order (A→B→C) |
| `oneshot_trigger_starts_cascade` | Oneshot signal stops ingestion, cascade follows |
| `post_shutdown_hook_executes` | Hook runs after `join_next()` loop completes |
| `abort_is_safety_net` | `abort_all()` terminates hung task, normal tasks complete OK |
| `biased_select_prioritises_shutdown` | `biased select!` fires close branch at next `.await` |

The Knot-specific integration tests in `tests/task_management.rs` validate the full pipeline (server + agents + filesystem). The generic tests validate the concurrency primitives in isolation. Together they form a complete test strategy: **generic tests prove the pattern, integration tests prove the wiring**.

## Consequences

### Positive

- **No orphan tasks** — child tasks are children of the `JoinSet`, which is owned by the server task.
- **Data-safe shutdown** — in-flight work completes, buffers flush, no partial writes.
- **Simple and composable** — each stage is independent; add/remove stages without changing shutdown logic.
- **Living documentation** — generic tests serve as reference for the pattern, readable without Knot context.
- **Reusable** — the pattern applies to any Rust/tokio service with async pipelines (web servers, ETL, message processing).

### Negative

- **Single point of failure** — leaked sender prevents entire cascade from working. Requires discipline in stage implementation.
- **Timing-sensitive in tests** — notify background threads holding `Arc` references can delay channel closure by milliseconds, making precise timing assertions unreliable.
- **Abort is a last resort** — if the safety net fires, it means a task genuinely hung (bug), and the abort may leave inconsistent state.

### Trade-offs Considered

| Alternative | Rejected Because |
|-------------|------------------|
| `tokio::spawn` per stage | Orphan tasks survive server drop, block runtime shutdown |
| Single `join_next()` call | Only waits for one task; remaining tasks are aborted |
| `select!` with all stages inline | Complex — requires restructuring server into single `select!` with N branches |
| Shutdown channel (explicit signal) | Adds boilerplate per stage; channel closure is free and automatic |
| `tokio::signal::ctrl_c()` only | Covers only one trigger; doesn't handle test shutdown or HTTP endpoints |
| Drop `JoinSet` immediately | Aborts all tasks — inconsistent state if stage is mid-write |

## References

- [ADR-002: Server Child Tasks — Graceful Cascade Shutdown](adr-002-server-child-tasks.md) — Knot-specific cascade shutdown implementation
- `tests/generic_task_management.rs` — generic pattern tests (10 tests, zero domain types)
- `tests/task_management.rs` — Knot integration tests for cascade shutdown
- `src/lib.rs` — `start_server_with_shutdown` and `start_event_pipeline` implementation
- `tokio::task::JoinSet` — aborts remaining tasks when dropped (safety net)
- `tokio::sync::mpsc::Receiver::recv` — returns `None` when all senders are dropped
- `tokio::select!` — `biased` mode for deterministic branch ordering
