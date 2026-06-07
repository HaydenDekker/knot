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

use std::time::Duration;

use tokio::sync::mpsc;
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
