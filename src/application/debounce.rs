//! Debounce engine for strand file-system events.
//!
//! Groups rapid events for the same file into a single debounced emission.
//! The adapter emits raw events; this engine filters them at 100ms per-file.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::domain::events::StrandEvent;
use crate::domain::entities::{KnotId, LoomId, StrandPath};

/// Default debounce window: 100 ms per file.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(100);

/// How often the engine checks for expired entries.
const CHECK_INTERVAL: Duration = Duration::from_millis(5);

// ── StrandEventKind ────────────────────────────────────────────────────────

/// The variant kind of a `StrandEvent`.
///
/// Used as part of the deduplication key so that different event types
/// (Created/Modified/Deleted) for the same file always pass through,
/// while repeated events of the same type are collapsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrandEventKind {
    /// A strand was created.
    Created,
    /// A strand was modified.
    Modified,
    /// A strand was deleted.
    Deleted,
}

/// Extract the `StrandEventKind` from a `StrandEvent`.
pub fn event_kind(event: &StrandEvent) -> StrandEventKind {
    match event {
        StrandEvent::Created { .. } => StrandEventKind::Created,
        StrandEvent::Modified { .. } => StrandEventKind::Modified,
        StrandEvent::Deleted { .. } => StrandEventKind::Deleted,
    }
}

/// Build the deduplication key for a `StrandEvent`.
///
/// The key includes the event kind so that different event types for the
/// same file are treated as distinct entries.
pub fn dedup_key(
    event: &StrandEvent,
) -> (StrandPath, LoomId, KnotId, StrandEventKind) {
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
        } => {
            (
                strand_path.clone(),
                loom_id.clone(),
                knot_id.clone(),
                event_kind(event),
            )
        }
    }
}

// ── InspectQueue ───────────────────────────────────────────────────────────

/// A thread-safe FIFO queue that supports in-place replacement.
///
/// Wraps a `VecDeque<T>` behind a `Mutex` with a `tokio::sync::Notify`
/// for signaling the consumer. The `push_or_replace` method scans the
/// queue for an existing item with the same deduplication key and
/// replaces it in-place rather than queuing a duplicate.
///
/// This replaces the opaque `mpsc::channel` between the debounce engine
/// and the process-strand consumer, making the pipeline inspectable so
/// that duplicate events can be collapsed before they are emitted.
pub struct InspectQueue<T> {
    inner: Mutex<VecDeque<T>>,
    notify: tokio::sync::Notify,
}

impl<T> InspectQueue<T> {
    /// Create a new empty queue.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            notify: tokio::sync::Notify::new(),
        }
    }

    /// Push an item to the back of the queue and signal one waiter.
    pub fn push(&self, item: T) {
        self.inner.lock().unwrap().push_back(item);
        self.notify.notify_one();
    }

    /// Push an item, or replace an existing item with the same key.
    ///
    /// Scans the queue for an existing item whose key (derived by `key_fn`)
    /// matches the key of the new item. If found, replaces it in-place.
    /// Otherwise pushes to the back. Always signals one waiter.
    pub fn push_or_replace<F, K>(&self, item: T, key_fn: F)
    where
        F: Fn(&T) -> K,
        K: PartialEq,
    {
        let mut queue = self.inner.lock().unwrap();
        let new_key = key_fn(&item);

        if let Some(pos) = queue
            .iter()
            .position(|existing| key_fn(existing) == new_key)
        {
            queue[pos] = item;
        } else {
            queue.push_back(item);
        }

        self.notify.notify_one();
    }

    /// Pop the first item from the front of the queue.
    ///
    /// Returns `None` if the queue is empty.
    pub fn pop(&self) -> Option<T> {
        self.inner.lock().unwrap().pop_front()
    }

    /// Await a signal that an item was pushed.
    ///
    /// Suspends the current task until `notify_one()` is called by a
    /// producer. Used by the consumer loop to wait efficiently.
    pub async fn notified(&self) {
        self.notify.notified().await;
    }

    /// Return the current length of the queue (for tests).
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

// ── InspectQueue<StrandEvent> shutdown support ─────────────────────────────

/// A wrapper around `InspectQueue<StrandEvent>` that tracks a shutdown flag.
///
/// The debounce engine signals shutdown after flushing all pending events.
/// This allows `QueueReceiver::recv()` to stop blocking once the engine
/// has exited.
struct QueueWithShutdown {
    queue: InspectQueue<StrandEvent>,
    /// Set to true when the debounce engine has flushed and exited.
    /// Protected by the same mutex as the queue for atomic check-and-pop.
    shutdown: Mutex<bool>,
}

impl QueueWithShutdown {
    fn new() -> Self {
        Self {
            queue: InspectQueue::new(),
            shutdown: Mutex::new(false),
        }
    }

    /// Signal that the debounce engine has exited.
    fn signal_shutdown(&self) {
        *self.shutdown.lock().unwrap() = true;
        // Wake any waiters in recv()
        self.queue.notify.notify_one();
    }

    /// Pop an item from the queue.
    ///
    /// Returns `Some(event)` if an item is available, `None` if the queue
    /// is empty (and the caller should wait or check shutdown).
    fn pop(&self) -> Option<StrandEvent> {
        self.queue.pop()
    }

    /// Check if shutdown has been signalled.
    fn is_shutdown(&self) -> bool {
        *self.shutdown.lock().unwrap()
    }

    /// Push an item with dedup, signaling waiters.
    fn push_or_replace(&self, item: StrandEvent) {
        self.queue.push_or_replace(item, dedup_key);
    }

    /// Await notification from a producer.
    async fn notified(&self) {
        self.queue.notified().await;
    }

}

// ── QueueReceiver ──────────────────────────────────────────────────────────

/// Async receiver wrapping a `QueueWithShutdown`.
///
/// Provides a `recv()` method that blocks until the next debounced event
/// is available, or `None` when the debounce engine has exited (input
/// channel closed + flush complete).
pub struct QueueReceiver {
    inner: Arc<QueueWithShutdown>,
}

impl QueueReceiver {
    /// Create a new receiver from a queue-with-shutdown.
    fn new(inner: Arc<QueueWithShutdown>) -> Self {
        Self { inner }
    }

    /// Receive the next debounced event, or `None` if the debounce engine
    /// has exited and the queue is drained.
    ///
    /// Blocks until an event is available or shutdown is signalled.
    pub async fn recv(&self) -> Option<StrandEvent> {
        // Fast path: item already in queue
        if let Some(event) = self.inner.pop() {
            return Some(event);
        }

        // Queue is empty — wait for notification (new item or shutdown).
        // After waking, check queue first, then shutdown.
        loop {
            self.inner.notified().await;

            // Check queue after notification.
            if let Some(event) = self.inner.pop() {
                return Some(event);
            }

            // Queue still empty — check if shutdown was signalled.
            if self.inner.is_shutdown() {
                return None;
            }

            // Spurious wakeup — loop back and wait again.
        }
    }
}

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
    /// - `QueueReceiver` — receive debounced events (async `recv()`)
    /// - `JoinHandle<()>` — handle for the background task
    pub fn start() -> (
        mpsc::Sender<StrandEvent>,
        QueueReceiver,
        JoinHandle<()>,
    ) {
        let (input_tx, input_rx) = mpsc::channel::<StrandEvent>(100);
        let inner = Arc::new(QueueWithShutdown::new());
        let receiver = QueueReceiver::new(Arc::clone(&inner));

        let handle = tokio::spawn(Self::run(input_rx, inner));

        (input_tx, receiver, handle)
    }

    /// Start the debounce engine using an external input channel.
    ///
    /// The provided `input_rx` is the receiver from the channel that
    /// `NotifyEventSource` sends raw events into. The debounce engine
    /// reads from this receiver and emits debounced events to its own
    /// output queue.
    ///
    /// Returns:
    /// - `QueueReceiver` — receive debounced events (async `recv()`)
    /// - `JoinHandle<()>` — handle for the background task
    pub fn start_with_receiver(
        input_rx: mpsc::Receiver<StrandEvent>,
    ) -> (QueueReceiver, JoinHandle<()>) {
        let inner = Arc::new(QueueWithShutdown::new());
        let receiver = QueueReceiver::new(Arc::clone(&inner));

        let handle = tokio::spawn(Self::run(input_rx, inner));

        (receiver, handle)
    }

    /// Start the debounce engine, spawning into a `JoinSet`.
    ///
    /// This variant ties the debounce task's lifetime to the caller's
    /// `JoinSet`, so it is aborted when the set is dropped or aborted.
    /// Used by the server startup to ensure pipeline tasks are children
    /// of the server task.
    ///
    /// Returns a `QueueReceiver` for consuming debounced events.
    pub fn spawn_with_receiver(
        input_rx: mpsc::Receiver<StrandEvent>,
        join_set: &mut tokio::task::JoinSet<()>,
    ) -> QueueReceiver {
        let inner = Arc::new(QueueWithShutdown::new());
        let receiver = QueueReceiver::new(Arc::clone(&inner));
        join_set.spawn(Self::run(input_rx, inner));
        receiver
    }

    /// Internal event loop: watch for incoming events and emit debounced ones.
    async fn run(
        mut input_rx: mpsc::Receiver<StrandEvent>,
        queue: Arc<QueueWithShutdown>,
    ) {
        // Maps (strand_path, loom_id, knot_id) → (last event, deadline).
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

                // New raw event arrives — update the pending entry for
                // that (file, knot) pair.
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
                            Self::flush_all(&pending, &queue).await;
                            // Signal shutdown so recv() returns None
                            // after the queue is drained.
                            queue.signal_shutdown();
                            return;
                        }
                    }
                }

                // Periodic check — emit any entries whose deadline has
                // passed.
                _ = check.tick() => {
                    let now = tokio::time::Instant::now();
                    let expired: Vec<_> = pending
                        .iter()
                        .filter(|(_, (_, deadline))| *deadline <= now)
                        .map(|(key, _)| key.clone())
                        .collect();

                    for key in expired {
                        if let Some((event, _)) = pending.remove(&key) {
                            queue.push_or_replace(event);
                        }
                    }
                }
            }
        }
    }

    /// Flush all pending entries to the output queue (used on shutdown).
    async fn flush_all(
        pending: &HashMap<(StrandPath, LoomId, KnotId), (StrandEvent, tokio::time::Instant)>,
        queue: &QueueWithShutdown,
    ) {
        for (event, _) in pending.values() {
            queue.push_or_replace(event.clone());
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
        let (tx, rx, _handle) = DebounceEngine::start();

        let event = created("file-a.md");
        tx.send(event.clone()).await.unwrap();

        // Before the debounce window, nothing should be emitted.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let immediate =
            tokio::time::timeout(Duration::from_millis(20), rx.recv()).await;
        assert!(
            immediate.is_err(),
            "event should not be emitted before debounce window"
        );

        // After the window, the event should arrive.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let received =
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .expect("should receive event after window")
                .expect("queue should not be closed");

        assert_eq!(event_kind(&received), "Created");
        assert_eq!(event_path(&received), "file-a.md");
    }

    #[tokio::test]
    async fn rapid_events_emit_only_last() {
        let (tx, rx, handle) = DebounceEngine::start();

        // Send 5 events for the same file within 50 ms.
        for _i in 0..5 {
            let path = "file-0.md"; // all events target the same file
            tx.send(modified(path)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        // Wait for the debounce window to expire (from last send).
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Only one event should be emitted — the last Modify.
        let received =
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .expect("should receive debounced event")
                .expect("queue should not be closed");

        assert_eq!(event_kind(&received), "Modified");
        assert_eq!(event_path(&received), "file-0.md");

        // Signal shutdown and verify no extra events.
        drop(tx);
        handle.await.unwrap();
        let extra =
            tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(
            extra.is_ok(),
            "recv() should return after shutdown"
        );
        assert!(
            extra.unwrap().is_none(),
            "no extra events should be emitted for same file"
        );
    }

    #[tokio::test]
    async fn different_files_emit_independently() {
        let (tx, rx, handle) = DebounceEngine::start();

        // Send events for two different files.
        tx.send(created("file-a.md")).await.unwrap();
        tx.send(created("file-b.md")).await.unwrap();

        // Wait for the debounce window.
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Both should be emitted.
        let mut received = Vec::new();
        for _ in 0..2 {
            let event =
                tokio::time::timeout(Duration::from_millis(50), rx.recv())
                    .await
                    .expect("should receive event")
                    .expect("queue should not be closed");
            received.push(event);
        }

        // Verify both files are present (order may vary).
        let paths: Vec<_> = received.iter().map(event_path).collect();
        assert!(paths.contains(&"file-a.md".to_string()));
        assert!(paths.contains(&"file-b.md".to_string()));

        // Signal shutdown and verify no more events.
        drop(tx);
        handle.await.unwrap();
        let extra =
            tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(extra.is_ok(), "recv() should return after shutdown");
        assert!(
            extra.unwrap().is_none(),
            "no extra events expected"
        );
    }

    #[tokio::test]
    async fn delete_after_modify_emits_delete() {
        let (tx, rx, handle) = DebounceEngine::start();

        // Send Modify then Delete for the same file, within the window.
        tx.send(modified("file-x.md")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        tx.send(deleted("file-x.md")).await.unwrap();

        // Wait for debounce window (from the Delete send).
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Only the Delete should be emitted.
        let received =
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .expect("should receive debounced event")
                .expect("queue should not be closed");

        assert_eq!(event_kind(&received), "Deleted");
        assert_eq!(event_path(&received), "file-x.md");

        // Signal shutdown and verify no additional events.
        drop(tx);
        handle.await.unwrap();
        let extra =
            tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(extra.is_ok(), "recv() should return after shutdown");
        assert!(
            extra.unwrap().is_none(),
            "no extra events expected"
        );
    }

    /// Same file modified, but watched by two different knots — both
    /// knots get independent debounced events.
    #[tokio::test]
    async fn same_file_different_knots_both_emit() {
        let (tx, rx, handle) = DebounceEngine::start();

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
            let event =
                tokio::time::timeout(Duration::from_millis(50), rx.recv())
                    .await
                    .expect("should receive event")
                    .expect("queue should not be closed");
            received.push(event);
        }

        let knot_ids: Vec<_> =
            received.iter().map(event_knot_id).collect();
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

        // Signal shutdown and verify no extra events.
        drop(tx);
        handle.await.unwrap();
        let extra =
            tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(extra.is_ok(), "recv() should return after shutdown");
        assert!(extra.unwrap().is_none(), "no extra events expected");
    }

    // ── StrandEventKind ─────────────────────────────────────────────────

    #[test]
    fn strand_event_kind_maps_created() {
        let event = created("file-a.md");
        assert_eq!(super::event_kind(&event), StrandEventKind::Created);
    }

    #[test]
    fn strand_event_kind_maps_modified() {
        let event = modified("file-a.md");
        assert_eq!(super::event_kind(&event), StrandEventKind::Modified);
    }

    #[test]
    fn strand_event_kind_maps_deleted() {
        let event = deleted("file-a.md");
        assert_eq!(super::event_kind(&event), StrandEventKind::Deleted);
    }

    // ── InspectQueue ────────────────────────────────────────────────────

    /// `push` adds to the back; `pop` removes from the front (FIFO).
    #[test]
    fn inspect_queue_push_pop_fifo() {
        let queue = InspectQueue::new();

        queue.push(1);
        queue.push(2);
        queue.push(3);

        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.pop(), Some(3));
        assert_eq!(queue.pop(), None);
    }

    /// `push_or_replace` replaces an existing item with the same key
    /// in-place, preserving queue order.
    #[test]
    fn inspect_queue_push_or_replace_existing() {
        let queue = InspectQueue::new();

        // Use (id, value) tuples — key is the id.
        queue.push_or_replace((1, "first"), |item| item.0);
        queue.push_or_replace((2, "second"), |item| item.0);
        queue.push_or_replace((1, "replaced"), |item| item.0);

        // Item 1 was replaced in-place; queue length is still 2.
        assert_eq!(queue.len(), 2);

        let first = queue.pop().unwrap();
        assert_eq!(first.0, 1);
        assert_eq!(first.1, "replaced");

        let second = queue.pop().unwrap();
        assert_eq!(second.0, 2);
        assert_eq!(second.1, "second");

        assert_eq!(queue.pop(), None);
    }

    /// `push_or_replace` pushes a new item when no matching key exists,
    /// behaving like `push`.
    #[test]
    fn inspect_queue_push_or_replace_different_key() {
        let queue = InspectQueue::new();

        queue.push_or_replace((1, "a"), |item| item.0);
        queue.push_or_replace((2, "b"), |item| item.0);
        queue.push_or_replace((3, "c"), |item| item.0);

        assert_eq!(queue.len(), 3);

        assert_eq!(queue.pop().unwrap().1, "a");
        assert_eq!(queue.pop().unwrap().1, "b");
        assert_eq!(queue.pop().unwrap().1, "c");
    }

    /// `push_or_replace` with `StrandEvent` using `dedup_key`: same file
    /// + same kind replaces; same file + different kind does not.
    #[test]
    fn inspect_queue_strand_event_same_kind_replaces() {
        let queue = InspectQueue::new();

        let e1 = created("file-a.md");
        let e2 = created("file-a.md");

        queue.push_or_replace(e1, dedup_key);
        queue.push_or_replace(e2, dedup_key);

        // Same (path, loom, knot, kind) — should replace in-place.
        assert_eq!(queue.len(), 1);
    }

    /// Same file, different event kinds — both stay in the queue.
    #[test]
    fn inspect_queue_strand_event_different_kind_no_replace() {
        let queue = InspectQueue::new();

        let created_ev = created("file-a.md");
        let modified_ev = modified("file-a.md");

        queue.push_or_replace(created_ev, dedup_key);
        queue.push_or_replace(modified_ev, dedup_key);

        // Different kinds — both should be queued.
        assert_eq!(queue.len(), 2);
    }

    /// Same file, different knot — both stay in the queue.
    #[test]
    fn inspect_queue_strand_event_different_knot_no_replace() {
        let queue = InspectQueue::new();

        let e_a = created_for("file-a.md", "loom-1", "knot-a");
        let e_b = created_for("file-a.md", "loom-1", "knot-b");

        queue.push_or_replace(e_a, dedup_key);
        queue.push_or_replace(e_b, dedup_key);

        // Different knots — both should be queued.
        assert_eq!(queue.len(), 2);
    }

    /// `notified` suspends until a producer signals.
    #[tokio::test]
    async fn inspect_queue_notified_waits_for_push() {
        let queue = Arc::new(InspectQueue::<i32>::new());
        let queue_clone = Arc::clone(&queue);

        let handle = tokio::spawn(async move {
            queue_clone.notified().await;
        });

        // Give the task a moment to enter the notified() call.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Push signals the waiter.
        queue.push(42);

        // The task should complete.
        tokio::time::timeout(Duration::from_millis(200), handle)
            .await
            .expect("notified() should unblock after push")
            .unwrap();
    }

    // ── Queue Dedup Integration Tests ───────────────────────────────────

    /// Rapid events for the same dedup key produce exactly one queued
    /// event. Events sent well after the debounce window (so they pass
    /// through the debounce engine independently) still get deduped by
    /// the InspectQueue if they share the same key.
    #[tokio::test]
    async fn rapid_same_key_produces_one_queued_event() {
        let (tx, rx, handle) = DebounceEngine::start();

        // Send events with gaps > 100ms so each passes the debounce
        // window independently. They share the same dedup key
        // (same path, loom, knot, kind), so the InspectQueue should
        // replace in-place.
        for _i in 0..5 {
            tx.send(modified("file-dedup.md")).await.unwrap();
            // Wait past debounce window so each event fires independently.
            tokio::time::sleep(Duration::from_millis(110)).await;
        }

        // Receive the one event that remains in the queue.
        let received_event =
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .expect("should receive debounced event")
                .expect("queue should not be closed");

        assert_eq!(event_kind(&received_event), "Modified");
        assert_eq!(event_path(&received_event), "file-dedup.md");

        // Signal shutdown and verify no extra events.
        drop(tx);
        handle.await.unwrap();
        let extra =
            tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(extra.is_ok(), "recv() should return after shutdown");
        assert!(
            extra.unwrap().is_none(),
            "no extra events should be in queue after dedup"
        );
    }

    /// Different event types (Created + Modified) for the same file
    /// both appear in the queue — they have different dedup keys.
    #[tokio::test]
    async fn different_event_types_both_appear_in_queue() {
        let (tx, rx, handle) = DebounceEngine::start();

        // Send a Created event, wait for it to debounce and fire.
        tx.send(created("file-multi.md")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Send a Modified event, wait for it to debounce and fire.
        tx.send(modified("file-multi.md")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;

        // Both should be in the queue (different kinds = different keys).
        let event1 =
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .expect("should receive first event")
                .expect("queue should not be closed");
        let event2 =
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .expect("should receive second event")
                .expect("queue should not be closed");

        // Verify both events are present with different kinds.
        let kinds: Vec<_> =
            vec![event_kind(&event1), event_kind(&event2)];
        assert!(
            kinds.contains(&"Created"),
            "Created event should be in queue"
        );
        assert!(
            kinds.contains(&"Modified"),
            "Modified event should be in queue"
        );

        // Both target the same file.
        assert_eq!(event_path(&event1), "file-multi.md");
        assert_eq!(event_path(&event2), "file-multi.md");

        // Signal shutdown and verify no more events.
        drop(tx);
        handle.await.unwrap();
        let extra =
            tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(extra.is_ok(), "recv() should return after shutdown");
        assert!(extra.unwrap().is_none(), "no extra events expected");
    }
}
