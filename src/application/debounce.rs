//! Debounce engine for strand file-system events.
//!
//! Groups rapid events for the same file into a single debounced emission.
//! The adapter emits raw events; this engine filters them at 100ms per-file.

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::domain::events::StrandEvent;
use crate::domain::entities::{KnotId, LoomId, StrandPath};

/// Default debounce window: 100 ms per file.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);

/// How often the engine checks for expired entries.
const CHECK_INTERVAL: Duration = Duration::from_millis(5);

// ── DebounceEngine ────────────────────────────────────────────────────────

/// Debounces `StrandEvent`s on a per-file basis.
///
/// Rapid events for the same strand path are coalesced — only the last
/// event within the debounce window is emitted. Events for different
/// files are tracked independently and can fire at different times.
pub struct DebounceEngine;

impl DebounceEngine {
    /// Start the debounce engine as a background tokio task.
    ///
    /// Creates its own input channel. Returns:
    /// - `Sender<StrandEvent>` — feed raw events into the engine
    /// - `Receiver<StrandEvent>` — receive debounced events
    /// - `JoinHandle<()>` — handle for the background task
    pub fn start() -> (
        mpsc::Sender<StrandEvent>,
        mpsc::Receiver<StrandEvent>,
        JoinHandle<()>,
    ) {
        let (input_tx, input_rx) = mpsc::channel::<StrandEvent>(100);
        let (output_tx, output_rx) = mpsc::channel::<StrandEvent>(100);

        let handle = tokio::spawn(Self::run(input_rx, output_tx));

        (input_tx, output_rx, handle)
    }

    /// Start the debounce engine using an external input channel.
    ///
    /// The provided `input_rx` is the receiver from the channel that
    /// `NotifyEventSource` sends raw events into. The debounce engine
    /// reads from this receiver and emits debounced events to its own
    /// output channel.
    ///
    /// Returns:
    /// - `Receiver<StrandEvent>` — receive debounced events
    /// - `JoinHandle<()>` — handle for the background task
    pub fn start_with_receiver(
        input_rx: mpsc::Receiver<StrandEvent>,
    ) -> (mpsc::Receiver<StrandEvent>, JoinHandle<()>) {
        let (output_tx, output_rx) = mpsc::channel::<StrandEvent>(100);

        let handle = tokio::spawn(Self::run(input_rx, output_tx));

        (output_rx, handle)
    }

    /// Start the debounce engine, spawning into a `JoinSet`.
    ///
    /// This variant ties the debounce task's lifetime to the caller's
    /// `JoinSet`, so it is aborted when the set is dropped or aborted.
    /// Used by the server startup to ensure pipeline tasks are children
    /// of the server task.
    ///
    /// Returns the debounced output receiver.
    pub fn spawn_with_receiver(
        input_rx: mpsc::Receiver<StrandEvent>,
        join_set: &mut tokio::task::JoinSet<()>,

    ) -> mpsc::Receiver<StrandEvent> {
        let (output_tx, output_rx) = mpsc::channel::<StrandEvent>(100);
        join_set.spawn(Self::run(input_rx, output_tx));
        output_rx
    }

    /// Internal event loop: watch for incoming events and emit debounced ones.
    async fn run(
        mut input_rx: mpsc::Receiver<StrandEvent>,
        output_tx: mpsc::Sender<StrandEvent>,
    ) {
        // Maps (strand_path, loom_id, knot_id) → (last event, deadline for emission).
        // The composite key ensures events for the same file but different
        // knots are tracked independently — two knots watching the same
        // strand directory each get their own debounced event.
        type EventKey = (StrandPath, LoomId, KnotId);
        let mut pending: HashMap<EventKey, (StrandEvent, tokio::time::Instant)> =
            HashMap::new();

        let window = DEBOUNCE_WINDOW;
        let mut check = tokio::time::interval(CHECK_INTERVAL);

        loop {
            tokio::select! {
                biased;

                // New raw event arrives — update the pending entry for that (file, knot) pair.
                maybe_event = input_rx.recv() => {
                    match maybe_event {
                        Some(event) => {
                            let key = Self::event_key(&event);
                            let deadline = tokio::time::Instant::now() + window;
                            pending.insert(key, (event, deadline));
                        }
                        None => {
                            // Input channel closed — drain remaining entries and exit.
                            Self::flush_all(&pending, &output_tx).await;
                            return;
                        }
                    }
                }

                // Periodic check — emit any entries whose deadline has passed.
                _ = check.tick() => {
                    let now = tokio::time::Instant::now();
                    let expired: Vec<_> = pending
                        .iter()
                        .filter(|(_, (_, deadline))| *deadline <= now)
                        .map(|(key, _)| key.clone())
                        .collect();

                    for key in expired {
                        if let Some((event, _)) = pending.remove(&key) {
                            let _ = output_tx.send(event).await;
                        }
                    }
                }
            }
        }
    }

    /// Flush all pending entries to the output channel (used on shutdown).
    async fn flush_all(
        pending: &HashMap<(StrandPath, LoomId, KnotId), (StrandEvent, tokio::time::Instant)>,
        output_tx: &mpsc::Sender<StrandEvent>,
    ) {
        for (event, _) in pending.values() {
            let _ = output_tx.send(event.clone()).await;
        }
    }

    /// Extract the composite key (file, loom, knot) from a `StrandEvent`.
    ///
    /// Using all three fields ensures that the same file watched by
    /// different knots produces independent debounced events.
    fn event_key(event: &StrandEvent) -> (StrandPath, LoomId, KnotId) {
        match event {
            StrandEvent::Created {
                strand_path,
                loom_id,
                knot_id,
            }
            | StrandEvent::Modified {
                strand_path,
                loom_id,
                knot_id,
            }
            | StrandEvent::Deleted {
                strand_path,
                loom_id,
                knot_id,
            } => (strand_path.clone(), loom_id.clone(), knot_id.clone()),
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{KnotId, LoomId};
    use std::path::PathBuf;

    /// Build a `Created` event for testing.
    fn created(path: &str) -> StrandEvent {
        StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("test-knot".to_string()),
            strand_path: StrandPath(PathBuf::from(path)),
        }
    }

    /// Build a `Modified` event for testing.
    fn modified(path: &str) -> StrandEvent {
        StrandEvent::Modified {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("test-knot".to_string()),
            strand_path: StrandPath(PathBuf::from(path)),
        }
    }

    /// Build a `Deleted` event for testing.
    fn deleted(path: &str) -> StrandEvent {
        StrandEvent::Deleted {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("test-knot".to_string()),
            strand_path: StrandPath(PathBuf::from(path)),
        }
    }

    /// Extract the variant kind from a `StrandEvent`.
    fn event_kind(event: &StrandEvent) -> &'static str {
        match event {
            StrandEvent::Created { .. } => "Created",
            StrandEvent::Modified { .. } => "Modified",
            StrandEvent::Deleted { .. } => "Deleted",
        }
    }

    /// Extract the file path string from a `StrandEvent`.
    fn event_path(event: &StrandEvent) -> String {
        match event {
            StrandEvent::Created { strand_path, .. }
            | StrandEvent::Modified { strand_path, .. }
            | StrandEvent::Deleted { strand_path, .. } => {
                strand_path.0.to_string_lossy().into_owned()
            }
        }
    }

    /// Extract the knot ID from a `StrandEvent`.
    fn event_knot_id(event: &StrandEvent) -> String {
        match event {
            StrandEvent::Created { knot_id, .. }
            | StrandEvent::Modified { knot_id, .. }
            | StrandEvent::Deleted { knot_id, .. } => knot_id.0.clone(),
        }
    }

    /// Build a `Created` event with explicit loom/knot IDs.
    fn created_for(path: &str, loom: &str, knot: &str) -> StrandEvent {
        StrandEvent::Created {
            loom_id: LoomId(loom.to_string()),
            knot_id: KnotId(knot.to_string()),
            strand_path: StrandPath(PathBuf::from(path)),
        }
    }

    #[tokio::test]
    async fn single_event_emits_after_window() {
        let (tx, mut rx, _handle) = DebounceEngine::start();

        let event = created("file-a.md");
        tx.send(event.clone()).await.unwrap();

        // Before the debounce window, nothing should be emitted.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let immediate = tokio::time::timeout(
            Duration::from_millis(20),
            rx.recv(),
        )
        .await;
        assert!(
            immediate.is_err(),
            "event should not be emitted before debounce window"
        );

        // After the window, the event should arrive.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let received = tokio::time::timeout(
            Duration::from_millis(50),
            rx.recv(),
        )
        .await
        .expect("should receive event after window")
        .expect("channel should not be closed");

        assert_eq!(event_kind(&received), "Created");
        assert_eq!(event_path(&received), "file-a.md");
    }

    #[tokio::test]
    async fn rapid_events_emit_only_last() {
        let (tx, mut rx, _handle) = DebounceEngine::start();

        // Send 5 events for the same file within 50 ms.
        for _i in 0..5 {
            let path = "file-0.md"; // all events target the same file
            tx.send(modified(path)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        // Wait for the debounce window to expire (from last send).
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Only one event should be emitted — the last Modify.
        let received = tokio::time::timeout(
            Duration::from_millis(50),
            rx.recv(),
        )
        .await
        .expect("should receive debounced event")
        .expect("channel should not be closed");

        assert_eq!(event_kind(&received), "Modified");
        assert_eq!(event_path(&received), "file-0.md");

        // No additional events should follow.
        let extra = tokio::time::timeout(
            Duration::from_millis(50),
            rx.recv(),
        )
        .await;
        assert!(
            extra.is_err(),
            "no extra events should be emitted for same file"
        );
    }

    #[tokio::test]
    async fn different_files_emit_independently() {
        let (tx, mut rx, _handle) = DebounceEngine::start();

        // Send events for two different files.
        tx.send(created("file-a.md")).await.unwrap();
        tx.send(created("file-b.md")).await.unwrap();

        // Wait for the debounce window.
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Both should be emitted.
        let mut received = Vec::new();
        for _ in 0..2 {
            let event = tokio::time::timeout(
                Duration::from_millis(50),
                rx.recv(),
            )
            .await
            .expect("should receive event")
            .expect("channel should not be closed");
            received.push(event);
        }

        // Verify both files are present (order may vary).
        let paths: Vec<_> = received.iter().map(event_path).collect();
        assert!(paths.contains(&"file-a.md".to_string()));
        assert!(paths.contains(&"file-b.md".to_string()));

        // No more events.
        let extra = tokio::time::timeout(
            Duration::from_millis(50),
            rx.recv(),
        )
        .await;
        assert!(extra.is_err(), "no extra events expected");
    }

    #[tokio::test]
    async fn delete_after_modify_emits_delete() {
        let (tx, mut rx, _handle) = DebounceEngine::start();

        // Send Modify then Delete for the same file, within the window.
        tx.send(modified("file-x.md")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        tx.send(deleted("file-x.md")).await.unwrap();

        // Wait for debounce window (from the Delete send).
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Only the Delete should be emitted.
        let received = tokio::time::timeout(
            Duration::from_millis(50),
            rx.recv(),
        )
        .await
        .expect("should receive debounced event")
        .expect("channel should not be closed");

        assert_eq!(event_kind(&received), "Deleted");
        assert_eq!(event_path(&received), "file-x.md");

        // No additional events.
        let extra = tokio::time::timeout(
            Duration::from_millis(50),
            rx.recv(),
        )
        .await;
        assert!(extra.is_err(), "no extra events expected");
    }

    /// Same file modified, but watched by two different knots — both
    /// knots get independent debounced events.
    #[tokio::test]
    async fn same_file_different_knots_both_emit() {
        let (tx, mut rx, _handle) = DebounceEngine::start();

        // Two knots watch the same strand directory.
        // A file change produces events for both knots.
        tx.send(created_for("shared.md", "loom-1", "knot-a"))
            .await
            .unwrap();
        tx.send(created_for("shared.md", "loom-1", "knot-b"))
            .await
            .unwrap();

        // Wait for debounce window.
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Both knots should receive events (different debounce keys).
        let mut received = Vec::new();
        for _ in 0..2 {
            let event = tokio::time::timeout(
                Duration::from_millis(50),
                rx.recv(),
            )
            .await
            .expect("should receive event")
            .expect("channel should not be closed");
            received.push(event);
        }

        let knot_ids: Vec<_> = received.iter().map(event_knot_id).collect();
        assert!(
            knot_ids.contains(&"knot-a".to_string()),
            "knot-a should have received an event"
        );
        assert!(
            knot_ids.contains(&"knot-b".to_string()),
            "knot-b should have received an event"
        );

        // Both events target the same file.
        for event in &received {
            assert_eq!(event_path(event), "shared.md");
        }

        // No extra events.
        let extra = tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await;
        assert!(extra.is_err(), "no extra events expected");
    }
}
