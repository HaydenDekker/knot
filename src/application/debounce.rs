//! Debounce engine for strand file-system events.
//!
//! Groups rapid events for the same file into a single debounced emission.
//! The adapter emits raw events; this engine filters them at 100ms per-file.

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::domain::events::StrandEvent;
use crate::domain::entities::StrandPath;

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
    /// Returns:
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

    /// Internal event loop: watch for incoming events and emit debounced ones.
    async fn run(
        mut input_rx: mpsc::Receiver<StrandEvent>,
        output_tx: mpsc::Sender<StrandEvent>,
    ) {
        // Maps strand path → (last event, deadline for emission)
        let mut pending: HashMap<StrandPath, (StrandEvent, tokio::time::Instant)> =
            HashMap::new();

        let window = DEBOUNCE_WINDOW;
        let mut check = tokio::time::interval(CHECK_INTERVAL);

        loop {
            tokio::select! {
                biased;

                // New raw event arrives — update the pending entry for that file.
                maybe_event = input_rx.recv() => {
                    match maybe_event {
                        Some(event) => {
                            let key = Self::file_key(&event);
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
                        .map(|(path, _)| path.clone())
                        .collect();

                    for path in expired {
                        if let Some((event, _)) = pending.remove(&path) {
                            let _ = output_tx.send(event).await;
                        }
                    }
                }
            }
        }
    }

    /// Flush all pending entries to the output channel (used on shutdown).
    async fn flush_all(
        pending: &HashMap<StrandPath, (StrandEvent, tokio::time::Instant)>,
        output_tx: &mpsc::Sender<StrandEvent>,
    ) {
        for (_, (event, _)) in pending {
            let _ = output_tx.send(event.clone()).await;
        }
    }

    /// Extract the strand path (file key) from a `StrandEvent`.
    fn file_key(event: &StrandEvent) -> StrandPath {
        match event {
            StrandEvent::Created { strand_path, .. } => strand_path.clone(),
            StrandEvent::Modified { strand_path, .. } => strand_path.clone(),
            StrandEvent::Deleted { strand_path, .. } => strand_path.clone(),
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
        for i in 0..5 {
            let path = format!("file-{}.md", i % 1); // all "file-0.md"
            tx.send(modified(&path)).await.unwrap();
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
        let paths: Vec<_> = received.iter().map(|e| event_path(e)).collect();
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
}
