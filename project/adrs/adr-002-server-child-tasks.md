# ADR-002: Server Child Tasks — Graceful Cascade Shutdown

**Date**: 2026-06-07
**Status**: Accepted

## Context

The Knot server spawns two background tasks on startup — the debounce engine and the event processor (ProcessStrand). Both sit in `recv().await` loops that drain when their channel sender is dropped.

The original implementation used `tokio::spawn` directly for both tasks. This created **runtime-level tasks** — children of the tokio runtime, not children of the server task. When the server task dropped (e.g., test end, `start_server` returns), those tasks continued running on the runtime. On a test runtime, this blocked shutdown because the runtime waits for all tasks to complete before exiting.

This caused the integration test migration (ADR-001) to hang: `tokio::spawn(knot::start_server(config))` would drop the server task at test end, but the background tasks survived and blocked the test runtime from cleaning up.

### The Abort Problem

The first fix — moving tasks into a `JoinSet` owned by the server task — solved the hang by aborting tasks when the `JoinSet` dropped. However, **abort is a blunt instrument**: it cuts execution at the very next `.await` point. If ProcessStrand is midway through executing an agent or writing state to a database, an abort leaves the system in an inconsistent state.

Since the background tasks already sit in `recv().await` loops that drain when their channel senders are dropped, **graceful cooperative shutdown** is the correct approach — tasks exit naturally through channel closure, not forced abortion.

## Decision

Background tasks are spawned into a `tokio::task::JoinSet` owned by the server task. On shutdown, a **sequential cascade** ensures all tasks drain cooperatively before the `JoinSet` is dropped. The `JoinSet` Drop is a **safety net**, not the primary shutdown mechanism.

### Shutdown Cascade

```
[Trigger: Ctrl+C / Test End / Oneshot Channel]
       │
       ▼
1. Stop Ingestion
   axum::serve exits → AppContext dropped → NotifyEventSource dropped
   → file watcher stopped → event_sender clone dropped
       │
       ▼
2. Close Channels
   event_sender (last sender) dropped → debounce input rx.recv() yields None
       │
       ▼
3. Drain Debounce Engine
   flushes all pending entries → output_tx dropped → task exits naturally
       │
       ▼
4. Drain ProcessStrand
   finishes in-flight agent → writes tie-off → debounce_rx.recv() yields None
   → task exits naturally
       │
       ▼
5. JoinSet Completes
   `while let Some(res) = join_set.join_next().await` — collects all tasks
   → no aborts needed
       │
       ▼
6. Post-Shutdown
   LoomStopped written to each loom-log → function returns
```

### Architecture Overview

```
Server task (start_server_with_shutdown)
├── JoinSet
│   ├── DebounceEngine::run()    — reads raw events, emits debounced events
│   └── ProcessStrand::run()     — reads debounced events, executes agents
├── AppContext (held by axum Router)
│   ├── NotifyEventSource        — file watcher, drops when router drops
│   └── event_sender (clone)     — drops when router drops
├── axum::serve(listener, app)   — HTTP server
├── graceful_shutdown future     — waits for Ctrl+C or channel
└── shutdown_log_port, shutdown_loom_ids — preserved for post-shutdown
```

### Changes

**`src/lib.rs` — `start_event_pipeline`** — Spawns both tasks into the provided `JoinSet`:

```rust
pub fn start_event_pipeline(
    ctx: &AppContext,
    event_rx: Receiver<StrandEvent>,
    join_set: &mut JoinSet<()>,
) {
    // DebounceEngine: input_rx → pending map → output_tx → output_rx
    // output_tx is held ONLY by the spawned debounce task.
    let mut debounce_rx =
        DebounceEngine::spawn_with_receiver(event_rx, join_set);

    // ProcessStrand: debounce_rx → execute(agent) → tie-off → next event
    join_set.spawn(async move {
        let use_case = ProcessStrand::new(...);
        while let Some(event) = debounce_rx.recv().await {
            if let Err(e) = use_case.execute(event) {
                eprintln!("ProcessStrand error: {e}");
            }
        }
    });
}
```

**`src/lib.rs` — `start_server_with_shutdown`** — Preserves shutdown references, drains JoinSet with a loop:

```rust
pub async fn start_server_with_shutdown(
    config: AppConfig,
    shutdown_signal: ShutdownSignal,
) -> std::io::Result<()> {
    let (mut ctx, event_rx) = build_app_context(&config);
    let mut join_set = JoinSet::new();
    start_event_pipeline(&ctx, event_rx, &mut join_set);

    // Preserve references needed AFTER AppContext is consumed by the router.
    let shutdown_log_port = Arc::clone(&ctx.loom_log_port);
    let shutdown_loom_ids: Vec<_> = looms.iter().map(|l| l.id.clone()).collect();

    let app = build_app(ctx);
    // ... axum::serve with graceful_shutdown ...

    // Drain ALL pipeline tasks — not just the first one.
    while let Some(res) = join_set.join_next().await {
        if let Err(e) = res {
            eprintln!("Background task failed: {e}");
        }
    }

    // Write LoomStopped using preserved references.
    for loom_id in &shutdown_loom_ids {
        let _ = shutdown_log_port.append(LoomEvent::LoomStopped { loom_id });
    }

    Ok(())
}
```

**`src/application/debounce.rs`** — `spawn_with_receiver` creates its own output channel and moves `output_tx` into the spawned task. No sender leaks to the caller.

### Key Design Decisions

1. **`while let Some` drain loop, not single `join_next()`** — The original code called `join_set.join_next().await` once, which collected only the first task (DebounceEngine) and then dropped the `JoinSet`, aborting ProcessStrand. The fix uses a loop to wait for ALL tasks.

2. **No leaked channel senders** — `spawn_with_receiver` creates `output_tx` inside the function and moves it into the spawned task. The caller receives only `output_rx`. When the debounce task exits, `output_tx` is dropped, closing the channel for ProcessStrand.

3. **Preserved shutdown references** — `loom_log_port` (Arc) and `loom_ids` (Vec) are cloned before `build_app(ctx)` consumes the AppContext. This avoids creating a new `FileSystemLoomLog` instance just for the LoomStopped write.

4. **Notify thread lifecycle** — The `notify` crate background thread holds an `Arc` reference to the event sender. When `NotifyEventSource` is dropped, the thread may not exit immediately, slightly delaying the debounce engine's input channel closure. This is acceptable for normal operation but means in-flight agent timing is unreliable in test environments.

### Implications for Design

- **Background tasks are children of the server task** — they cannot outlive the server. This is correct for the server lifecycle.
- **No `JoinHandle` returned from `start_event_pipeline`** — callers that need task handles should manage their own `JoinSet`.
- **Existing `start_with_receiver` preserved** — the `tokio::spawn` variant remains for callers that need independent task ownership (e.g., unit tests of the debounce engine).
- **`graceful_shutdown` function is unused** — it is no longer called from `start_server_with_shutdown` (which now uses the inline cascade). It remains as public API but could be removed in a future cleanup.

### Dependencies

No new dependencies. Uses `tokio::task::JoinSet` which is part of tokio v1 (already a dependency).

### Testing Strategy

Integration test suite `tests/task_management.rs` validates the cascade shutdown:

| Test | What It Verifies |
|------|-----------------|
| `pipeline_tasks_drain_cleanly_on_shutdown` | Baseline: server starts, shutdown signal fires, tasks drain, JoinHandle completes, LoomStopped written |
| `shutdown_flushes_pending_debounce_events` | Debounce flush path: shutdown before debounce window expires → pending events flushed → tie-off produced |
| `multiple_strands_then_graceful_shutdown` | Multiple strands processed, all tie-offs produced, clean shutdown |
| `shutdown_with_failing_agent` | Error handling during shutdown: failed agent → error tie-off + KnotFailed → clean exit |
| `in_flight_processing_completes_on_shutdown` *(ignored)* | In-flight agent completion — unreliable due to notify thread Arc reference; validated by manual testing and by the drain loop code path |

## Consequences

### Positive

- **Tests never hang on shutdown** — child tasks are children of the server task; the JoinSet drain loop waits for them to complete cooperatively.
- **Correct lifetime semantics** — child tasks cannot outlive the server, matching the intended lifecycle.
- **Data-safe shutdown** — in-flight agent work completes before shutdown returns; tie-offs are written, loom-logs are updated.
- **No forced aborts** — tasks exit through their `recv().await` loops naturally. The JoinSet Drop abort is only a safety net.
- **Simple test pattern** — tests don't need to manage shutdown signals or thread joins; sending on the oneshot channel triggers the cascade.

### Negative

- **Notify thread delay** — the notify background thread holds an Arc reference to the event sender, which can delay the debounce engine's input channel closure by milliseconds. This is negligible for production but makes in-flight timing tests unreliable.
- **`start_event_pipeline` is less composable** — it no longer returns a `JoinHandle`, so callers can't await individual pipeline tasks. Callers that need this should manage their own `JoinSet`.
- **`graceful_shutdown` function is unused** — the old standalone function is no longer called. It remains for backwards compatibility but could be removed.

### Trade-offs Considered

| Alternative | Rejected Because |
|-------------|------------------|
| Keep `tokio::spawn`, fix tests with `rt.shutdown_timeout()` | Works but masks the real problem — orphan tasks are a design smell, not a test problem |
| Use `tokio::select!` to run tasks inline | Complex — requires restructuring `start_server_with_shutdown` into a single `select!` with three branches (serve, debounce, process) |
| Return `JoinSet` from `start_event_pipeline` | Adds indirection — the caller already owns the set; passing `&mut` is simpler |
| Single `join_next()` call | Only waits for ONE task — the second task is aborted by the JoinSet Drop, losing in-flight work |
| Drop JoinSet immediately (no drain) | Aborts all tasks — inconsistent state if ProcessStrand is writing tie-offs or logs |

## References

- [ADR-001: Integration Test Server Pattern](adr-001-integration-test-server-pattern.md) — test-side pattern that motivated this change
- `src/lib.rs:340-430` — `start_server_with_shutdown` implementation
- `src/lib.rs:85-125` — `start_event_pipeline` implementation
- `src/application/debounce.rs:60-86` — `start_with_receiver` and `spawn_with_receiver`
- `tests/task_management.rs` — integration test suite for cascade shutdown
- `tokio::task::JoinSet` — aborts all tasks when dropped (safety net)
- `tokio::sync::mpsc::Receiver::recv` — returns `None` when all senders are dropped
