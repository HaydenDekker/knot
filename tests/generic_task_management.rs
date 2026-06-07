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
