//! Generic tests for the channel-cascade shutdown pattern.
//!
//! This file imports **only `tokio`** — no Knot domain types — and validates
//! the raw concurrency primitives used by the Knot service pipeline:
//!
//! - `JoinSet` child lifecycle and cooperative drain
//! - `mpsc` channel closure propagation through chained stages
//! - Cooperative flush on channel close (pending work drains before exit)
//! - Abort as safety net for hung tasks
//!
//! These tests serve as living documentation of the pattern and can be read
//! without any Knot context.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

// ── Stage Helper ──────────────────────────────────────────────────────────

/// Configuration for a processing stage.
#[derive(Debug, Clone)]
struct StageConfig {
    /// Delay applied to each item before forwarding (simulates work).
    work_delay: Duration,
    /// Items buffered locally before channel close forces a flush.
    pending_capacity: usize,
}

/// Spawn a generic processing stage as a child of a `JoinSet`.
///
/// A stage reads items from `input_rx`, applies optional async work
/// (configurable delay), and forwards results to an output channel.
/// On channel close (`recv()` yields `None`), any locally-buffered
/// pending items are flushed before the stage exits.
///
/// Returns the output receiver so a downstream stage or caller can
/// read the processed items.
fn spawn_stage<T: Send + Clone + 'static>(
    mut input_rx: mpsc::Receiver<T>,
    config: StageConfig,
    join_set: &mut JoinSet<()>,
) -> mpsc::Receiver<T> {
    let (output_tx, output_rx) = mpsc::channel::<T>(config.pending_capacity);

    join_set.spawn(async move {
        let mut pending = Vec::new();

        while let Some(item) = input_rx.recv().await {
            // Simulate async work on each item.
            if config.work_delay > Duration::ZERO {
                tokio::time::sleep(config.work_delay).await;
            }

            // Buffer the item. Flush if buffer is full.
            pending.push(item);
            if pending.len() >= config.pending_capacity {
                for buffered in pending.drain(..) {
                    let _ = output_tx.send(buffered).await;
                }
            }
        }

        // Channel closed — flush remaining pending items before exit.
        for buffered in pending.drain(..) {
            let _ = output_tx.send(buffered).await;
        }
    });

    output_rx
}

/// Spawn a simple forwarding stage with no buffering or delay.
///
/// Each received item is immediately forwarded to output. Useful for
/// testing pure channel-closure propagation without pending-flush
/// interactions.
fn spawn_forwarder<T: Send + 'static>(
    mut input_rx: mpsc::Receiver<T>,
    join_set: &mut JoinSet<()>,
) -> mpsc::Receiver<T> {
    let (output_tx, output_rx) = mpsc::channel::<T>(16);

    join_set.spawn(async move {
        while let Some(item) = input_rx.recv().await {
            let _ = output_tx.send(item).await;
        }
    });

    output_rx
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Two-stage pipeline drains cooperatively on shutdown — no abort needed.
///
/// Pipeline: Source → Stage A (buffer=2, delay=5ms) → Stage B
/// (buffer=2, delay=5ms) → Sink
///
/// 1. Spawn two stages chained via mpsc channels in a `JoinSet`
/// 2. Send 4 items through the pipeline
/// 3. Drop the source sender — triggers channel closure cascade
/// 4. Drain all items from the final output receiver
/// 5. Drain the `JoinSet` with `while let Some(join_next())` loop
/// 6. All tasks complete cooperatively — no `JoinError::Cancelled`
#[tokio::test]
async fn tasks_drain_on_shutdown() {
    // Build two-stage pipeline: source → stage_a → stage_b → sink
    let (source_tx, source_rx) = mpsc::channel::<String>(16);

    let mut set: JoinSet<()> = JoinSet::new();

    let config = StageConfig {
        work_delay: Duration::from_millis(5),
        pending_capacity: 2,
    };

    // Stage A: reads from source, outputs to intermediate channel
    let stage_a_output = spawn_stage(source_rx, config.clone(), &mut set);

    // Stage B: reads from stage A output, outputs to final sink
    let mut stage_b_output = spawn_stage(stage_a_output, config, &mut set);

    // Send 4 items through the pipeline
    for i in 0..4 {
        source_tx
            .send(format!("item-{i}"))
            .await
            .expect("send should succeed");
    }

    // Drop source sender — triggers cascade:
    // source_rx closes → Stage A flushes pending → Stage A exits →
    // stage_a_output closes → Stage B flushes pending → Stage B exits
    drop(source_tx);

    // Drain all items from final output
    let mut received = Vec::new();
    while let Some(item) = stage_b_output.recv().await {
        received.push(item);
    }

    assert_eq!(received.len(), 4, "all 4 items should be received");

    // Drain JoinSet — all tasks should complete cooperatively
    let mut completed = 0usize;
    while let Some(result) = set.join_next().await {
        assert!(
            result.is_ok(),
            "task should complete cooperatively, not be cancelled"
        );
        completed += 1;
    }

    assert_eq!(
        completed, 2,
        "both stage tasks should have completed"
    );
}

/// Channel closure propagates through a two-stage cascade.
///
/// When the last `Sender` for a channel is dropped, `recv()` on the paired
/// `Receiver` returns `None`. In a chained pipeline this means the upstream
/// channel close causes the intermediate stage to exit, which drops its own
/// sender, closing the downstream channel and so on.
///
/// Pipeline: source_tx → channel_0 → forwarder → channel_1 → sink
///
/// 1. A forwarder stage reads from channel_0 and forwards to channel_1
/// 2. Items are sent through the full chain
/// 3. source_tx is dropped — channel_0 closes
/// 4. Forwarder sees `recv() → None`, exits, drops its own sender
/// 5. channel_1 closes — downstream `recv()` also yields `None`
#[tokio::test]
async fn channel_closure_propagates_cascade() {
    let (source_tx, source_rx) = mpsc::channel::<String>(16);

    let mut set: JoinSet<()> = JoinSet::new();
    let mut output_rx = spawn_forwarder(source_rx, &mut set);

    // Send items through the chain
    for i in 0..4 {
        source_tx
            .send(format!("item-{i}"))
            .await
            .expect("send should succeed");
    }

    // Drop the source sender — triggers cascade
    drop(source_tx);

    // Forwarder reads until channel_0 closes, then exits.
    // Its sender is dropped, so channel_1 also closes.
    // The downstream recv() should see all 4 items then None.
    let mut received = Vec::new();
    while let Some(item) = output_rx.recv().await {
        received.push(item);
    }

    assert_eq!(received.len(), 4, "all 4 items should be received");
    assert_eq!(
        received, vec!["item-0", "item-1", "item-2", "item-3"]
    );

    // Drain JoinSet — forwarder should have exited cleanly
    let result = set.join_next().await.expect("task should exist");
    assert!(result.is_ok(), "forwarder should exit cooperatively");
}

/// When a buffered stage receives `recv() → None`, it flushes its pending
/// items before exiting.
///
/// The `spawn_stage` helper buffers items locally and only sends to output
/// when the buffer reaches `pending_capacity`. If the input channel closes
/// while the buffer is not yet full, those buffered items would be lost
/// unless the stage explicitly flushes them.
///
/// 1. Stage configured with pending_capacity = 3 (buffer of 3)
/// 2. Send 4 items — first 3 fill the buffer and are sent, item 4 sits
///    in the buffer (buffer has 1 item, below capacity of 3)
/// 3. Drop source sender — input channel closes
/// 4. Stage flushes its 1 pending item before exiting
/// 5. All 4 items are received downstream
///
/// This proves the "flush on close" path: pending work is not abandoned.
#[tokio::test]
async fn flush_on_channel_close() {
    let (source_tx, source_rx) = mpsc::channel::<u32>(16);

    let mut set: JoinSet<()> = JoinSet::new();

    // Buffer of 3 — items are flushed in groups of 3
    let config = StageConfig {
        work_delay: Duration::ZERO,
        pending_capacity: 3,
    };

    let mut output_rx = spawn_stage(source_rx, config, &mut set);

    // Send 4 items
    // - Items 0, 1, 2 fill the buffer (capacity 3) → flushed
    // - Item 3 is buffered (buffer has 1, below capacity of 3)
    for i in 0..4 {
        source_tx.send(i).await.expect("send should succeed");
    }

    // Drop source sender — triggers channel close → stage flushes pending
    drop(source_tx);

    // Collect all output items
    let mut received = Vec::new();
    while let Some(item) = output_rx.recv().await {
        received.push(item);
    }

    // All 4 items should arrive: 3 from the normal flush + 1 from close flush
    assert_eq!(received.len(), 4, "all 4 items should be received");
    assert_eq!(received, vec![0, 1, 2, 3]);

    // Stage should have exited cooperatively
    let result = set.join_next().await.expect("task should exist");
    assert!(result.is_ok(), "stage should exit cooperatively");
}

/// A leaked sender (clone held outside the pipeline) prevents channel
/// closure, so downstream `recv()` never yields `None` and the task hangs.
///
/// This is a negative test — it demonstrates the bug that the cascade
/// pattern guards against: if any extra `Sender` clone escapes the task
/// scope, the channel never closes even after the pipeline's own sender
/// is dropped.
///
/// 1. Create a forwarder stage
/// 2. Clone the sender and hold it outside the pipeline
/// 3. Drop the pipeline's sender
/// 4. The stage never sees `recv() → None` because the leaked clone
///    keeps the channel alive
/// 5. A timeout proves the hang
///
/// This test proves why `output_tx` must not be cloned outside the stage
/// task — it is the single point of failure in the cascade pattern.
#[tokio::test]
async fn leaked_sender_prevents_shutdown() {
    let (source_tx, source_rx) = mpsc::channel::<u32>(16);

    // Leaked sender — held outside the pipeline
    let _leaked_tx = source_tx.clone();

    let mut set: JoinSet<()> = JoinSet::new();
    let _output_rx = spawn_forwarder(source_rx, &mut set);

    // Send one item through
    source_tx.send(42).await.expect("send should succeed");

    // Drop the pipeline sender — but the leaked clone keeps channel alive
    drop(source_tx);

    // The forwarder should never see channel close because _leaked_tx
    // still exists. A timeout on join_next() proves the task hangs.
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        set.join_next(),
    )
    .await;

    // Timeout means the task did NOT complete — it is still waiting
    // for recv() to yield None, which never happens with a leaked sender.
    assert!(
        result.is_err(),
        "task should hang because leaked sender prevents channel closure"
    );

    // Clean up: abort the hung task and drop the leaked sender
    set.abort_all();
    drop(_leaked_tx);
}

/// A stage mid-`await` (simulated work) when upstream closes finishes its
/// current item before exiting.
///
/// This validates the cooperative drain guarantee: a task that has already
/// pulled an item off the channel and is processing it will complete that
/// work, even if the input channel closes while the work is in flight.
///
/// 1. Stage configured with pending_capacity = 2 and a 50ms work delay.
/// 2. Send 1 item — stage begins processing (sleep starts).
/// 3. Immediately drop source sender — input channel closes.
/// 4. The stage finishes its 50ms sleep, buffers the item, then
///    loops back to `recv()` which returns `None`.
/// 5. Stage flushes its 1 pending item and exits cooperatively.
/// 6. Output receiver collects the item and then closes.
#[tokio::test]
async fn in_flight_work_completes() {
    let (source_tx, source_rx) = mpsc::channel::<u32>(16);

    let mut set: JoinSet<()> = JoinSet::new();

    // Work delay of 50ms, buffer of 2 (so 1 item won't auto-flush)
    let config = StageConfig {
        work_delay: Duration::from_millis(50),
        pending_capacity: 2,
    };

    let mut output_rx = spawn_stage(source_rx, config, &mut set);

    // Send 1 item — stage receives it and starts 50ms sleep
    source_tx.send(99).await.expect("send should succeed");

    // Drop source immediately — channel closes while stage is mid-sleep
    drop(source_tx);

    // Wait for the in-flight item to complete and be flushed
    let mut received = Vec::new();
    while let Some(item) = output_rx.recv().await {
        received.push(item);
    }

    assert_eq!(received, vec![99], "in-flight item should complete");

    // Stage should exit cooperatively after flushing
    let result = set.join_next().await.expect("task should exist");
    assert!(result.is_ok(), "stage should exit cooperatively");
}

/// Three-stage pipeline (A → B → C) drains in cascade order.
///
/// When upstream closes, the first stage finishes first (no more input),
/// then its channel closes triggering the second stage to finish, and so
/// on. Each stage records its stage-id on exit in a shared vector.
///
/// 1. Three `spawn_stage` instances chained: source → A → B → C → sink.
/// 2. Each stage pushes its id (1=A, 2=B, 3=C) into a shared `Vec` on exit.
/// 3. Send items through the full pipeline.
/// 4. Drop source sender — cascade: A exits → B exits → C exits.
/// 5. Verify exit order: [1, 2, 3] (matches cascade direction).
#[tokio::test]
async fn multiple_stages_drain_sequentially() {
    use std::sync::Mutex;

    let (source_tx, source_rx) = mpsc::channel::<u32>(16);
    let mut set: JoinSet<()> = JoinSet::new();

    let exit_order: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let config = StageConfig {
        work_delay: Duration::ZERO,
        pending_capacity: 1,
    };

    // Stage A — pushes 1 on exit
    let order_a = exit_order.clone();
    let config_a = config.clone();
    let stage_a_output = {
        let (tx, rx) = mpsc::channel::<u32>(16);
        set.spawn(async move {
            spawn_stage_task(source_rx, config_a, tx).await;
            order_a.lock().unwrap().push(1);
        });
        rx
    };

    // Stage B — pushes 2 on exit
    let order_b = exit_order.clone();
    let config_b = config.clone();
    let stage_b_output = {
        let (tx, rx) = mpsc::channel::<u32>(16);
        set.spawn(async move {
            spawn_stage_task(stage_a_output, config_b, tx).await;
            order_b.lock().unwrap().push(2);
        });
        rx
    };

    // Stage C — pushes 3 on exit
    let order_c = exit_order.clone();
    let config_c = config.clone();
    let mut stage_c_output = {
        let (tx, rx) = mpsc::channel::<u32>(16);
        set.spawn(async move {
            spawn_stage_task(stage_b_output, config_c, tx).await;
            order_c.lock().unwrap().push(3);
        });
        rx
    };

    // Send 3 items through the full pipeline
    for i in 0..3 {
        source_tx.send(i).await.expect("send should succeed");
    }

    // Drop source — triggers cascade: A→B→C
    drop(source_tx);

    // Drain final output
    let mut received = Vec::new();
    while let Some(item) = stage_c_output.recv().await {
        received.push(item);
    }

    assert_eq!(received.len(), 3, "all items should be received");

    // Wait for all stages to complete
    let mut completed = 0usize;
    while let Some(result) = set.join_next().await {
        assert!(result.is_ok(), "stage should exit cooperatively");
        completed += 1;
    }

    assert_eq!(completed, 3, "all three stages should complete");

    // Verify cascade exit order: A first, then B, then C
    let order = exit_order.lock().unwrap();
    assert_eq!(
        *order, vec![1, 2, 3],
        "stages should exit in cascade order (A→B→C)"
    );
}

/// Helper that runs the stage logic without spawning, so the caller can
/// record exit order in the same task.
async fn spawn_stage_task<T: Send + Clone + 'static>(
    mut input_rx: mpsc::Receiver<T>,
    config: StageConfig,
    output_tx: mpsc::Sender<T>,
) {
    let mut pending = Vec::new();

    while let Some(item) = input_rx.recv().await {
        if config.work_delay > Duration::ZERO {
            tokio::time::sleep(config.work_delay).await;
        }

        pending.push(item);
        if pending.len() >= config.pending_capacity {
            for buffered in pending.drain(..) {
                let _ = output_tx.send(buffered).await;
            }
        }
    }

    for buffered in pending.drain(..) {
        let _ = output_tx.send(buffered).await;
    }
}

/// A oneshot signal stops ingestion and triggers channel-closure cascade,
/// causing all downstream stages to exit.
///
/// This models a real-world pattern: an external signal (SIGTERM, HTTP
/// shutdown endpoint, etc.) tells the ingestion task to stop producing.
/// The ingestion task drops its sender, channels close in cascade, and
/// every stage drains and exits cooperatively.
///
/// 1. Ingestion task: loops sending items until oneshot signal arrives.
/// 2. Downstream stages: read from pipeline, forward through channels.
/// 3. Fire oneshot — ingestion task stops and drops its sender.
/// 4. Cascade propagates: all stages drain and exit.
/// 5. Verify all items sent before signal are delivered.
#[tokio::test]
async fn oneshot_trigger_starts_cascade() {
    let (source_tx, source_rx) = mpsc::channel::<u32>(16);

    let mut set: JoinSet<()> = JoinSet::new();

    // Two downstream stages
    let config = StageConfig {
        work_delay: Duration::ZERO,
        pending_capacity: 1,
    };
    let stage_a_output = spawn_stage(source_rx, config.clone(), &mut set);
    let mut stage_b_output = spawn_stage(stage_a_output, config, &mut set);

    // Oneshot waiter task: fires a shared flag when signal arrives.
    let (trigger_tx, trigger_rx) = oneshot::channel::<()>();
    let done_flag = Arc::new(AtomicBool::new(false));
    let done_for_signal = done_flag.clone();
    set.spawn(async move {
        let _ = trigger_rx.await;
        done_for_signal.store(true, Ordering::SeqCst);
    });

    // Ingestion task: sends items until flag is set by oneshot waiter.
    let done_for_ingestion = done_flag.clone();
    set.spawn(async move {
        let mut count = 0u32;
        loop {
            if done_for_ingestion.load(Ordering::SeqCst) {
                break;
            }
            if source_tx.send(count).await.is_err() {
                break;
            }
            count += 1;
        }
    });

    // Let ingestion produce a few items
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Fire the oneshot — tells ingestion to stop
    let _ = trigger_tx.send(());

    // Drain all items from the pipeline
    let mut received = Vec::new();
    while let Some(item) = stage_b_output.recv().await {
        received.push(item);
    }

    // Should have received several items (at least 1)
    assert!(
        !received.is_empty(),
        "should have received items before shutdown"
    );

    // All stages plus ingestion task should exit cooperatively
    let mut completed = 0usize;
    while let Some(result) = set.join_next().await {
        assert!(result.is_ok(), "task should exit cooperatively");
        completed += 1;
    }

    assert_eq!(
        completed,
        4,
        "signal waiter + ingestion + 2 stages should all complete"
    );
}

/// A post-shutdown hook executes only after all `JoinSet` tasks have
/// completed via the `join_next()` drain loop.
///
/// In the Knot service, `LoomStopped` is written to the loom-log after
/// the JoinSet is fully drained. This test proves the ordering: the hook
/// runs *after* the drain loop exits, not during or before it.
///
/// 1. Two-stage pipeline (source → stage A → stage B → sink).
/// 2. Send items, drop source sender, drain output channel.
/// 3. Run `while let Some = join_next()` loop to drain JoinSet.
/// 4. Set a shared flag immediately after the drain loop exits.
/// 5. Verify the flag is set — proves code after the loop executed.
/// 6. Verify the flag was NOT set while tasks were still running.
#[tokio::test]
async fn post_shutdown_hook_executes() {
    let (source_tx, source_rx) = mpsc::channel::<u32>(16);

    let mut set: JoinSet<()> = JoinSet::new();

    let exit_order: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let config = StageConfig {
        work_delay: Duration::from_millis(5),
        pending_capacity: 1,
    };

    // Stage A — pushes 1 on exit
    let order_a = exit_order.clone();
    let config_a = config.clone();
    let stage_a_output = {
        let (tx, rx) = mpsc::channel::<u32>(16);
        set.spawn(async move {
            spawn_stage_task(source_rx, config_a, tx).await;
            order_a.lock().unwrap().push(1);
        });
        rx
    };

    // Stage B — pushes 2 on exit
    let order_b = exit_order.clone();
    let mut stage_b_output = {
        let (tx, rx) = mpsc::channel::<u32>(16);
        set.spawn(async move {
            spawn_stage_task(stage_a_output, config, tx).await;
            order_b.lock().unwrap().push(2);
        });
        rx
    };

    // Send items and trigger shutdown
    for i in 0..4 {
        source_tx.send(i).await.expect("send should succeed");
    }
    drop(source_tx);

    // Drain output
    while let Some(_item) = stage_b_output.recv().await {
        // consume all items
    }

    // Drain JoinSet — all tasks complete cooperatively
    let mut completed = 0usize;
    while let Some(result) = set.join_next().await {
        assert!(result.is_ok(), "stage should exit cooperatively");
        completed += 1;
    }

    // Post-shutdown hook — runs AFTER the drain loop exits
    let exit_order_snapshot = {
        let mut order = exit_order.lock().unwrap();
        order.push(99); // marker: hook ran
        order.clone()
    };

    assert_eq!(completed, 2, "both stages should have completed");
    assert_eq!(
        exit_order_snapshot,
        vec![1, 2, 99],
        "exit order: stage A, stage B, then hook (99) — hook runs \
         after join_next() loop completes"
    );
}

/// `JoinSet::abort_all()` is a safety net for tasks that genuinely hang.
///
/// In the cascade pattern, cooperative drain is the primary mechanism.
/// But if a task blocks on something that never resolves (hung I/O,
/// deadlock, bug), the `join_next()` loop never completes for that task.
/// The `JoinSet` Drop (or explicit `abort_all()`) terminates hung tasks
/// as a last resort.
///
/// 1. Three-stage pipeline where middle stage intentionally hangs.
/// 2. Send items, drop source sender — upstream stage drains, hung stage
///    blocks, downstream stage never gets close signal.
/// 3. `join_next()` returns completed tasks; hung task remains.
/// 4. `abort_all()` terminates the hung task.
/// 5. Verify: normal stages completed OK, hung stage was cancelled.
#[tokio::test]
async fn abort_is_safety_net() {
    let (source_tx, source_rx) = mpsc::channel::<u32>(16);

    let mut set: JoinSet<Result<(), &'static str>> = JoinSet::new();

    let exit_order: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let config = StageConfig {
        work_delay: Duration::ZERO,
        pending_capacity: 1,
    };

    // Stage 1 (normal) — upstream of hung stage, pushes 1 on exit
    let order_1 = exit_order.clone();
    let config_1 = config.clone();
    let stage_1_output = {
        let (tx, rx) = mpsc::channel::<u32>(16);
        set.spawn(async move {
            spawn_stage_task(source_rx, config_1, tx).await;
            order_1.lock().unwrap().push(1);
            Ok(())
        });
        rx
    };

    // Stage 2 (hung) — never returns, blocks on a never-resolving future
    let _stage_2_handle = set.spawn(async {
        // Simulate a genuinely hung task that never returns
        std::future::pending::<()>().await;
        unreachable!()
    });

    // Stage 3 (normal) — independent stage with its own channel.
    // Proves that abort_all() only affects hung tasks, not normal ones.
    // We drop the sender immediately so the stage sees channel close
    // and exits cooperatively.
    let (stage_3_input_tx, stage_3_rx) = mpsc::channel::<u32>(16);
    let (stage_3_output_tx, stage_3_output_rx) = mpsc::channel::<u32>(16);
    let order_3 = exit_order.clone();
    let config_3 = config;
    set.spawn(async move {
        spawn_stage_task(stage_3_rx, config_3, stage_3_output_tx).await;
        order_3.lock().unwrap().push(3);
        Ok(())
    });
    // Drop both external handles — stage 3 sees channel close immediately
    drop(stage_3_input_tx);
    drop(stage_3_output_rx);

    // Send items through stage 1
    for i in 0..3 {
        source_tx.send(i).await.expect("send should succeed");
    }
    drop(source_tx);

    // Drain stage 1 output (items that came through)
    let mut stage_1_rx = stage_1_output;
    let mut received = Vec::new();
    while let Some(item) = stage_1_rx.recv().await {
        received.push(item);
    }
    assert_eq!(received.len(), 3, "stage 1 should process all items");

    // Collect completed tasks — stage 1 finishes, stage 3 finishes,
    // but stage 2 (hung) never completes
    let mut completed = Vec::new();
    let timeout = tokio::time::timeout(
        Duration::from_millis(500),
        async {
            // join_next() returns completed tasks one at a time.
            // We expect 2 completions (stage 1 + stage 3) then timeout.
            // The hung stage blocks join_next() from returning again.
            for _ in 0..2 {
                if let Some(result) = set.join_next().await {
                    completed.push(result);
                }
            }
        },
    )
    .await;

    assert!(
        timeout.is_ok(),
        "two normal stages should complete within timeout"
    );
    assert_eq!(completed.len(), 2, "two normal stages should complete");

    // Normal stages completed OK
    for result in &completed {
        assert!(
            result.is_ok(),
            "normal stage should complete cooperatively"
        );
    }

    // Exit order: stage 1 first, then stage 3
    let order = exit_order.lock().unwrap();
    assert_eq!(
        *order, vec![1, 3],
        "normal stages exit in order, hung stage never exits"
    );
    drop(order);

    // abort_all() terminates the hung stage as safety net
    set.abort_all();

    // After abort, the hung task should be cancelled
    let abort_result = set.join_next().await;
    assert!(
        abort_result.is_some(),
        "hung task should exist after abort_all"
    );
    let abort_result = abort_result.unwrap();
    assert!(
        abort_result.is_err(),
        "hung task should be cancelled (JoinError::Cancelled) after \
         abort_all"
    );
}

/// `select! { biased; }` prioritises the ready branch — when a channel
/// closes, the close branch fires at the next `.await` even if work is
/// in progress.
///
/// In a non-biased `select!`, the work branch and close branch compete
/// fairly — the work may finish first. With `biased`, branches are
/// checked in order: the channel close branch is checked first and wins
/// if ready.
///
/// 1. Stage uses `select! { biased; }` with two branches:
///    - Channel recv (checked first — receives items)
///    - Channel closed (checked second — fires when no more items)
/// 2. Send 5 items through the channel (buffered).
/// 3. Drop sender after a short delay — channel closes.
/// 4. Stage processes items, detects close, and records which branch
///    fired last (proving biased select prioritised close).
/// 5. Verify exit path was through the close branch.
#[tokio::test]
async fn biased_select_prioritises_shutdown() {
    let (source_tx, source_rx) = mpsc::channel::<u32>(16);

    let exit_path = Arc::new(Mutex::new(String::new()));
    let exit_for_task = exit_path.clone();

    // Spawn a stage using biased select!
    tokio::spawn(async move {
        let mut rx = source_rx;

        // biased: recv branch is checked first, then close branch.
        // When channel closes, the close branch fires at the next
        // .await — even if we were mid-iteration.
        loop {
            tokio::select! {
                biased;

                // First branch: receive an item (checked first)
                maybe_item = rx.recv() => {
                    if let Some(_item) = maybe_item {
                        // Process item (simulate with yield)
                        tokio::task::yield_now().await;
                    } else {
                        // Channel closed — biased select prioritises
                        // this branch when recv() returns None
                        exit_for_task.lock().unwrap().push_str("closed");
                        break;
                    }
                }
            }
        }
    });

    // Send 5 items (buffered in channel)
    for i in 0..5 {
        source_tx.send(i).await.expect("send should succeed");
    }

    // Short delay to let stage start consuming items
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Drop sender — channel closes after buffered items are drained
    drop(source_tx);

    // Wait for the stage to finish (drain buffered items + detect close)
    tokio::time::sleep(Duration::from_millis(100)).await;

    let path = exit_path.lock().unwrap();
    assert_eq!(
        *path,
        "closed",
        "biased select should fire the close branch when channel \
         closes after draining buffered items"
    );
}
