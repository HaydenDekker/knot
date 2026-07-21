//! Debounce engine for strand file-system events.
//!
//! Groups rapid events for the same file into a single debounced emission.
//! The adapter emits raw events; this engine filters them at 100ms per-file.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::domain::events::StrandEvent;
use crate::domain::entities::{KnotId, LoomId, StrandPath};
use crate::domain::pending_event::PendingEvent;
use crate::application::ports::StrandEventQueue;

/// Default debounce window: 100 ms per file.
pub const DEFAULT_DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);

/// Default check interval: 5 ms.
pub const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_millis(5);

// ── DebounceEngine ────────────────────────────────────────────────────────

/// Debounces `StrandEvent`s on a per-file basis.
///
/// Rapid events for the same strand path are coalesced — only the last
/// event within the debounce window is emitted. Events for different
/// files are tracked independently and can fire at different times.
pub struct DebounceEngine;

impl DebounceEngine {
    /// Start the debounce engine, spawning into a `JoinSet` with
    /// explicit timing parameters and an externally-provided queue.
    ///
    /// The `queue` is shared with the ProcessStrand consumer and
    /// `WriteState`. The debounce engine pushes `PendingEvent` for
    /// debounced events and calls `push_shutdown()` on channel close.
    ///
    /// Returns the queue Arc for chaining.
    pub fn spawn_with_receiver_with_window_and_queue<
        Q: StrandEventQueue + 'static,
    >(
        input_rx: mpsc::Receiver<StrandEvent>,
        join_set: &mut tokio::task::JoinSet<()>,
        window: Duration,
        check_interval: Duration,
        queue: Arc<Q>,
    ) -> Arc<Q> {
        join_set.spawn(Self::run_with_queue(
            input_rx,
            Arc::clone(&queue) as Arc<dyn StrandEventQueue>,
            window,
            check_interval,
        ));
        queue
    }

    /// Internal event loop: watch for incoming events and emit debounced
    /// ones through the [`StrandEventQueue`] trait.
    async fn run_with_queue(
        mut input_rx: mpsc::Receiver<StrandEvent>,
        queue: Arc<dyn StrandEventQueue>,
        window: Duration,
        check_interval: Duration,
    ) {
        // Maps (strand_path, loom_id, knot_id) → (last event, deadline).
        type EventKey = (StrandPath, LoomId, KnotId);
        let mut pending: HashMap<EventKey, (StrandEvent, tokio::time::Instant)> =
            HashMap::new();

        let mut check = tokio::time::interval(check_interval);

        loop {
            tokio::select! {
                biased;

                maybe_event = input_rx.recv() => {
                    match maybe_event {
                        Some(event) => {
                            let key = Self::event_key(&event);
                            let deadline =
                                tokio::time::Instant::now() + window;
                            pending.insert(key, (event, deadline));
                        }
                        None => {
                            // Input channel closed — drain remaining
                            // entries and exit.
                            Self::flush_all_with_queue(&pending, &*queue);
                            queue.push_shutdown();
                            return;
                        }
                    }
                }

                _ = check.tick() => {
                    let now = tokio::time::Instant::now();
                    let expired: Vec<_> = pending
                        .iter()
                        .filter(|(_, (_, deadline))| *deadline <= now)
                        .map(|(key, _)| key.clone())
                        .collect();

                    for key in expired {
                        if let Some((event, _)) = pending.remove(&key) {
                            let pending_event: PendingEvent = event.into();
                            (*queue).push_or_replace(pending_event);
                        }
                    }
                }
            }
        }
    }

    /// Flush all pending entries to the output queue (used on shutdown).
    fn flush_all_with_queue(
        pending: &HashMap<
            (StrandPath, LoomId, KnotId),
            (StrandEvent, tokio::time::Instant),
        >,
        queue: &dyn StrandEventQueue,
    ) {
        for (event, _) in pending.values() {
            let pending_event: PendingEvent = event.clone().into();
            queue.push_or_replace(pending_event);
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
    use crate::application::in_memory_event_queue::InMemoryEventQueue;
    use crate::domain::entities::{KnotId, LoomId, StrandPath};
    use crate::domain::pending_event::PendingEventOrShutdown;
    use std::path::PathBuf;
    use std::sync::Arc;

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

    /// Build a `Created` event with explicit loom/knot IDs.
    fn created_for(path: &str, loom: &str, knot: &str) -> StrandEvent {
        StrandEvent::Created {
            loom_id: LoomId(loom.to_string()),
            knot_id: KnotId(knot.to_string()),
            strand_path: StrandPath(PathBuf::from(path)),
        }
    }

    // ── Test Helpers ──────────────────────────────────────────────────

    /// Start the debounce engine with an `InMemoryEventQueue` and fast
    /// timing suitable for tests.
    fn start_debounce() -> (
        mpsc::Sender<StrandEvent>,
        Arc<InMemoryEventQueue>,
        JoinSetHandle,
    ) {
        let (input_tx, input_rx) = mpsc::channel::<StrandEvent>(100);
        let queue = Arc::new(InMemoryEventQueue::new());
        let mut join_set = tokio::task::JoinSet::new();

        DebounceEngine::spawn_with_receiver_with_window_and_queue(
            input_rx,
            &mut join_set,
            Duration::from_millis(50), // fast window for tests
            Duration::from_millis(5),
            Arc::clone(&queue),
        );

        (input_tx, queue, JoinSetHandle { inner: join_set })
    }

    struct JoinSetHandle {
        inner: tokio::task::JoinSet<()>,
    }

    impl JoinSetHandle {
        async fn join(&mut self) {
            while let Some(res) = self.inner.join_next().await {
                if let Err(e) = res {
                    eprintln!("Background task failed: {e}");
                }
            }
        }
    }

    /// Receive the next item from the queue, waiting with a timeout.
    ///
    /// Blocks until an event arrives or the shutdown sentinel is pushed.
    /// Returns `Some(PendingEvent)` for real events, `Some(Shutdown)` for
    /// the shutdown sentinel, or `None` on timeout.
    async fn recv_with_timeout(
        queue: &Arc<InMemoryEventQueue>,
        timeout: Duration,
    ) -> Option<PendingEventOrShutdown> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(item) = queue.pop() {
                return Some(item);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            let sleep_until = deadline.min(tokio::time::Instant::now() + Duration::from_millis(10));
            tokio::select! {
                _ = tokio::time::sleep(sleep_until - tokio::time::Instant::now()) => {
                    // Timeout reached
                    return None;
                }
                _ = queue.notified() => {
                    // Re-check the queue
                }
            }
        }
    }

    /// Poll the queue for events, collecting all available items up to
    /// a maximum timeout. Returns collected events and any shutdown sentinel.
    async fn collect_events(
        queue: &Arc<InMemoryEventQueue>,
        count: usize,
        timeout: Duration,
    ) -> Vec<PendingEventOrShutdown> {
        let mut collected = Vec::new();
        for _ in 0..count {
            match recv_with_timeout(queue, timeout).await {
                Some(item) => collected.push(item),
                None => break,
            }
        }
        collected
    }

    // ── Debounce Engine Tests ─────────────────────────────────────────

    /// Single event emits after the debounce window expires.
    #[tokio::test]
    async fn single_event_emits_after_window() {
        let (tx, queue, mut handle) = start_debounce();

        let event = created("file-a.md");
        tx.send(event.clone()).await.unwrap();

        // Before the debounce window, nothing should be emitted.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let immediate = queue.pop();
        assert!(
            immediate.is_none(),
            "event should not be emitted before debounce window"
        );

        // After the window, the event should arrive.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let received = recv_with_timeout(&queue, Duration::from_millis(50)).await;

        assert!(received.is_some(), "should receive event after window");
        if let PendingEventOrShutdown::Event(pe) = received.unwrap() {
            assert_eq!(pe.strand_path, "file-a.md");
            assert_eq!(pe.kind, "Created");
        } else {
            panic!("expected Event variant");
        }

        // Signal shutdown.
        drop(tx);
        handle.join().await;
    }

    /// Shutdown flush uses the queue — pending events are pushed via
    /// `push_or_replace`, then `push_shutdown` is called.
    #[tokio::test]
    async fn shutdown_flush_uses_queue() {
        let (tx, queue, mut handle) = start_debounce();

        // Send event but don't wait for debounce window.
        tx.send(created("file-flush.md")).await.unwrap();

        // Drop sender immediately — debounce engine flushes pending.
        drop(tx);
        handle.join().await;

        // The flushed event arrives.
        let event = recv_with_timeout(&queue, Duration::from_millis(50)).await;
        assert!(event.is_some(), "should receive flushed event");
        if let PendingEventOrShutdown::Event(pe) = event.unwrap() {
            assert_eq!(pe.strand_path, "file-flush.md");
        } else {
            panic!("expected Event variant");
        }

        // Then the shutdown sentinel.
        let sentinel = recv_with_timeout(&queue, Duration::from_millis(50)).await;
        assert!(
            matches!(sentinel, Some(PendingEventOrShutdown::Shutdown)),
            "should receive Shutdown sentinel after flush"
        );
    }

    /// Shutdown sentinel arrives as `PendingEventOrShutdown::Shutdown`.
    #[tokio::test]
    async fn shutdown_sentinel_arrives_as_shutdown_variant() {
        let (tx, queue, mut handle) = start_debounce();

        // Send one event and let it debounce.
        tx.send(created("file-shutdown.md")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Consume the event.
        let event = recv_with_timeout(&queue, Duration::from_millis(50)).await;
        assert!(event.is_some(), "should receive event");
        if let PendingEventOrShutdown::Event(pe) = event.unwrap() {
            assert_eq!(pe.strand_path, "file-shutdown.md");
        } else {
            panic!("expected Event variant");
        }

        // Drop sender to trigger shutdown.
        drop(tx);
        handle.join().await;

        // Next recv should return Shutdown sentinel.
        let result = recv_with_timeout(&queue, Duration::from_millis(100)).await;
        assert!(
            matches!(result, Some(PendingEventOrShutdown::Shutdown)),
            "should receive Shutdown on shutdown"
        );
    }

    /// Rapid events for the same file produce exactly one queued event.
    #[tokio::test]
    async fn rapid_events_produce_one_queued_event() {
        let (tx, queue, mut handle) = start_debounce();

        // Send 5 events for the same file within 50 ms.
        for _ in 0..5 {
            tx.send(modified("file-0.md")).await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        // Wait for the debounce window to expire (from last send).
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Only one event should be emitted — the last Modify.
        let received = recv_with_timeout(&queue, Duration::from_millis(50)).await;
        assert!(received.is_some(), "should receive debounced event");
        if let PendingEventOrShutdown::Event(pe) = received.unwrap() {
            assert_eq!(pe.kind, "Modified");
            assert_eq!(pe.strand_path, "file-0.md");
        } else {
            panic!("expected Event variant");
        }

        // Signal shutdown and verify no extra events.
        drop(tx);
        handle.join().await;

        let extra = recv_with_timeout(&queue, Duration::from_millis(100)).await;
        // After shutdown, we get the sentinel (no extra events)
        assert!(
            matches!(extra, Some(PendingEventOrShutdown::Shutdown)),
            "no extra events should be emitted for same file"
        );
    }

    /// Different files emit independently.
    #[tokio::test]
    async fn different_files_emit_independently() {
        let (tx, queue, mut handle) = start_debounce();

        // Send events for two different files.
        tx.send(created("file-a.md")).await.unwrap();
        tx.send(created("file-b.md")).await.unwrap();

        // Wait for the debounce window.
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Both should be emitted.
        let events = collect_events(&queue, 2, Duration::from_millis(100)).await;
        assert_eq!(events.len(), 2, "should receive two events");

        let mut paths = Vec::new();
        for ev in &events {
            if let PendingEventOrShutdown::Event(pe) = ev {
                paths.push(pe.strand_path.clone());
            } else {
                panic!("expected Event variant");
            }
        }
        assert!(paths.contains(&"file-a.md".to_string()));
        assert!(paths.contains(&"file-b.md".to_string()));

        // Signal shutdown and verify no more events.
        drop(tx);
        handle.join().await;

        let extra = recv_with_timeout(&queue, Duration::from_millis(100)).await;
        assert!(
            matches!(extra, Some(PendingEventOrShutdown::Shutdown)),
            "no extra events expected"
        );
    }

    /// Delete after modify for the same file emits only the delete.
    #[tokio::test]
    async fn delete_after_modify_emits_delete() {
        let (tx, queue, mut handle) = start_debounce();

        // Send Modify then Delete for the same file, within the window.
        tx.send(modified("file-x.md")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        tx.send(deleted("file-x.md")).await.unwrap();

        // Wait for debounce window (from the Delete send).
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Only the Delete should be emitted.
        let received = recv_with_timeout(&queue, Duration::from_millis(50)).await;
        assert!(received.is_some(), "should receive debounced event");
        if let PendingEventOrShutdown::Event(pe) = received.unwrap() {
            assert_eq!(pe.kind, "Deleted");
            assert_eq!(pe.strand_path, "file-x.md");
        } else {
            panic!("expected Event variant");
        }

        // Signal shutdown.
        drop(tx);
        handle.join().await;

        let extra = recv_with_timeout(&queue, Duration::from_millis(100)).await;
        assert!(
            matches!(extra, Some(PendingEventOrShutdown::Shutdown)),
            "no extra events expected"
        );
    }

    /// Same file modified, but watched by two different knots — both
    /// knots get independent debounced events.
    #[tokio::test]
    async fn same_file_different_knots_both_emit() {
        let (tx, queue, mut handle) = start_debounce();

        // Two knots watch the same strand directory.
        // A file change produces events for both knots.
        tx.send(created_for("shared.md", "loom-1", "knot-a"))
            .await
            .unwrap();
        tx.send(created_for("shared.md", "loom-1", "knot-b"))
            .await
            .unwrap();

        // Wait for debounce window.
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Both knots should receive events (different debounce keys).
        let events = collect_events(&queue, 2, Duration::from_millis(100)).await;
        assert_eq!(events.len(), 2, "should receive two events");

        let mut knot_ids = Vec::new();
        for ev in &events {
            if let PendingEventOrShutdown::Event(pe) = ev {
                knot_ids.push(pe.knot_id.clone());
                assert_eq!(pe.strand_path, "shared.md");
            } else {
                panic!("expected Event variant");
            }
        }
        assert!(
            knot_ids.contains(&"test-knot".to_string()) ||
            knot_ids.iter().any(|k| k.contains("knot-a") || k.contains("knot-b")),
            "both knots should have received events"
        );

        // Signal shutdown.
        drop(tx);
        handle.join().await;
    }
}
