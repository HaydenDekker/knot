//! In-memory implementation of [`StrandEventQueue`].
//!
//! Uses a `VecDeque<PendingEvent>` behind a `Mutex` with a
//! `tokio::sync::Notify` for signaling. No disk I/O.
//!
//! This is the Phase-1 compat shim: the entire pipeline speaks
//! `StrandEventQueue` but the backing store is purely in-memory.
//! Phase 2 introduces `DiskBackedEventQueue` as the production
//! implementation; Phase 3b swaps it in.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::domain::events::StrandQueueAccessor;
use crate::domain::pending_event::{
    dedup_key, PendingEvent, PendingEventId, PendingEventOrShutdown,
};

/// In-memory implementation of [`super::ports::StrandEventQueue`].
///
/// Internal state: `Mutex<VecDeque<PendingEvent>>` + `Mutex<bool>` for
/// shutdown sentinel + `tokio::sync::Notify`.
pub struct InMemoryEventQueue {
    events: Mutex<VecDeque<PendingEvent>>,
    shutdown: Mutex<bool>,
    notify: tokio::sync::Notify,
}

impl InMemoryEventQueue {
    pub fn new() -> Self {
        Self {
            events: Mutex::new(VecDeque::new()),
            shutdown: Mutex::new(false),
            notify: tokio::sync::Notify::new(),
        }
    }
}

impl Default for InMemoryEventQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl super::ports::StrandEventQueue for InMemoryEventQueue {
    fn push(
        &self,
        event: PendingEvent,
    ) -> PendingEventId {
        let id = event.id.clone();
        self.events.lock().unwrap().push_back(event);
        self.notify.notify_one();
        id
    }

    fn push_or_replace(
        &self,
        event: PendingEvent,
    ) -> PendingEventId {
        let mut queue = self.events.lock().unwrap();
        let key = dedup_key(&event);

        if let Some(pos) = queue
            .iter()
            .position(|existing| dedup_key(existing) == key)
        {
            queue[pos] = event;
        } else {
            queue.push_back(event);
        }

        let id = queue.back().unwrap().id.clone();
        self.notify.notify_one();
        id
    }

    fn pop(&self) -> Option<PendingEventOrShutdown> {
        let event = {
            let mut queue = self.events.lock().unwrap();
            queue.pop_front()
        };

        if let Some(event) = event {
            return Some(PendingEventOrShutdown::Event(event));
        }

        // Queue is empty — check shutdown flag.
        if *self.shutdown.lock().unwrap() {
            Some(PendingEventOrShutdown::Shutdown)
        } else {
            None
        }
    }

    fn snapshot(&self) -> Vec<PendingEvent> {
        self.events.lock().unwrap().iter().cloned().collect()
    }

    fn delete(&self, id: &PendingEventId) -> bool {
        let mut queue = self.events.lock().unwrap();
        if let Some(pos) = queue.iter().position(|e| &e.id == id) {
            queue.remove(pos);
            true
        } else {
            false
        }
    }

    fn pending_event(
        &self,
        id: &PendingEventId,
    ) -> Option<PendingEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .find(|e| &e.id == id)
            .cloned()
    }

    fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    fn is_empty(&self) -> bool {
        self.events.lock().unwrap().is_empty()
    }

    fn push_shutdown(&self) {
        *self.shutdown.lock().unwrap() = true;
        self.notify.notify_one();
    }

    fn notified(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        let notify = &self.notify;
        Box::pin(async move {
            notify.notified().await;
        })
    }
}

impl std::fmt::Debug for InMemoryEventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.events.lock().unwrap();
        f.debug_struct("InMemoryEventQueue")
            .field("len", &guard.len())
            .finish_non_exhaustive()
    }
}

impl StrandQueueAccessor for InMemoryEventQueue {
    fn pending_strand_paths(&self) -> Vec<std::path::PathBuf> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| std::path::PathBuf::from(&e.strand_path))
            .collect()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::StrandEventQueue;
    use crate::domain::entities::{KnotId, LoomId, StrandPath};
    use crate::domain::events::StrandEvent;
    use std::path::PathBuf;
    use std::sync::Arc;

    /// Build a `PendingEvent` from a `StrandEvent` for testing.
    fn make_pending(event: StrandEvent) -> PendingEvent {
        event.into()
    }

    /// Build a `Created` StrandEvent for testing.
    fn created(path: &str) -> StrandEvent {
        StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("test-knot".to_string()),
            strand_path: StrandPath(PathBuf::from(path)),
        }
    }

    /// Build a `Modified` StrandEvent for testing.
    fn modified(path: &str) -> StrandEvent {
        StrandEvent::Modified {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("test-knot".to_string()),
            strand_path: StrandPath(PathBuf::from(path)),
        }
    }

    // ── Trait Object-Safety ─────────────────────────────────────────

    /// Trait is object-safe — `&dyn StrandEventQueue` compiles.
    #[test]
    fn trait_is_object_safe() {
        let queue = InMemoryEventQueue::new();
        let _obj: &dyn StrandEventQueue = &queue;
    }

    // ── push / pop FIFO ───────────────────────────────────────────

    /// `push` adds to the back; `pop` removes from the front (FIFO).
    #[test]
    fn push_pop_fifo() {
        let queue = InMemoryEventQueue::new();

        let e1 = make_pending(created("file-a.md"));
        let e2 = make_pending(created("file-b.md"));
        let e3 = make_pending(created("file-c.md"));

        queue.push(e1);
        queue.push(e2);
        queue.push(e3);

        assert_eq!(queue.len(), 3);

        let first = queue.pop().unwrap();
        if let PendingEventOrShutdown::Event(e) = first {
            assert_eq!(e.strand_path, "file-a.md");
        } else {
            panic!("expected Event variant");
        }

        let second = queue.pop().unwrap();
        if let PendingEventOrShutdown::Event(e) = second {
            assert_eq!(e.strand_path, "file-b.md");
        } else {
            panic!("expected Event variant");
        }

        let third = queue.pop().unwrap();
        if let PendingEventOrShutdown::Event(e) = third {
            assert_eq!(e.strand_path, "file-c.md");
        } else {
            panic!("expected Event variant");
        }

        assert!(queue.is_empty());
        // No shutdown — empty pop returns None
        assert!(queue.pop().is_none());
    }

    // ── push_or_replace ───────────────────────────────────────────

    /// `push_or_replace` replaces an existing event with the same
    /// dedup key in-place, preserving queue order.
    #[test]
    fn push_or_replace_existing() {
        let queue = InMemoryEventQueue::new();

        let e1 = make_pending(created("file-a.md"));
        let e2 = make_pending(created("file-b.md"));
        let e1_replaced = make_pending(created("file-a.md")); // same dedup key
        let e1_replaced_id = e1_replaced.id.clone();

        queue.push_or_replace(e1);
        queue.push_or_replace(e2);
        queue.push_or_replace(e1_replaced);

        // Item 1 was replaced in-place; queue length is still 2.
        assert_eq!(queue.len(), 2);

        let first = queue.pop().unwrap();
        if let PendingEventOrShutdown::Event(e) = first {
            assert_eq!(e.strand_path, "file-a.md");
            assert_eq!(e.id, e1_replaced_id); // replaced with new ID
        } else {
            panic!("expected Event variant");
        }

        let second = queue.pop().unwrap();
        if let PendingEventOrShutdown::Event(e) = second {
            assert_eq!(e.strand_path, "file-b.md");
        } else {
            panic!("expected Event variant");
        }
    }

    /// `push_or_replace` pushes a new event when no matching dedup key.
    #[test]
    fn push_or_replace_different_key() {
        let queue = InMemoryEventQueue::new();

        let e1 = make_pending(created("file-a.md"));
        let e2 = make_pending(modified("file-a.md")); // different kind
        let e3 = make_pending(created("file-b.md")); // different path

        queue.push_or_replace(e1);
        queue.push_or_replace(e2);
        queue.push_or_replace(e3);

        assert_eq!(queue.len(), 3);
    }

    // ── snapshot ──────────────────────────────────────────────────

    /// `snapshot()` returns all events in FIFO order.
    #[test]
    fn snapshot_returns_items_in_fifo_order() {
        let queue = InMemoryEventQueue::new();

        queue.push(make_pending(created("file-a.md")));
        queue.push(make_pending(created("file-b.md")));

        let snap = queue.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].strand_path, "file-a.md");
        assert_eq!(snap[1].strand_path, "file-b.md");

        // Snapshot is a copy — popping after doesn't change it.
        queue.pop();
        assert_eq!(queue.snapshot().len(), 1);
    }

    /// `snapshot()` excludes shutdown sentinel.
    #[test]
    fn snapshot_excludes_shutdown() {
        let queue = InMemoryEventQueue::new();

        queue.push(make_pending(created("file-a.md")));
        queue.push_shutdown();

        // snapshot() returns only events (never the shutdown sentinel)
        let snap = queue.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].strand_path, "file-a.md");
    }

    // ── delete / pending_event ────────────────────────────────────

    /// `delete` removes the event; returns `true` if it existed.
    #[test]
    fn delete_existing() {
        let queue = InMemoryEventQueue::new();

        let e1 = make_pending(created("file-a.md"));
        let id = e1.id.clone();
        queue.push(e1);
        queue.push(make_pending(created("file-b.md")));

        assert_eq!(queue.len(), 2);
        assert!(queue.delete(&id));
        assert_eq!(queue.len(), 1);
    }

    /// `delete` returns `false` for non-existent ID.
    #[test]
    fn delete_nonexistent() {
        let queue = InMemoryEventQueue::new();
        let id = PendingEventId("999-zzzz".to_string());
        assert!(!queue.delete(&id));
    }

    /// `pending_event` returns the event by ID.
    #[test]
    fn pending_event_existing() {
        let queue = InMemoryEventQueue::new();

        let e1 = make_pending(created("file-a.md"));
        let id = e1.id.clone();
        queue.push(e1);

        let found = queue.pending_event(&id).unwrap();
        assert_eq!(found.strand_path, "file-a.md");
    }

    /// `pending_event` returns `None` for non-existent ID.
    #[test]
    fn pending_event_nonexistent() {
        let queue = InMemoryEventQueue::new();
        let id = PendingEventId("999-zzzz".to_string());
        assert!(queue.pending_event(&id).is_none());
    }

    // ── shutdown sentinel ─────────────────────────────────────────

    /// `push_shutdown` + empty pop returns `Shutdown`.
    #[test]
    fn shutdown_sentinel() {
        let queue = InMemoryEventQueue::new();
        queue.push_shutdown();

        let result = queue.pop();
        assert!(matches!(
            result,
            Some(PendingEventOrShutdown::Shutdown)
        ));
    }

    /// Events are popped before shutdown sentinel.
    #[test]
    fn events_before_shutdown() {
        let queue = InMemoryEventQueue::new();

        queue.push(make_pending(created("file-a.md")));
        queue.push(make_pending(created("file-b.md")));
        queue.push_shutdown();

        // Events come first.
        let e1 = queue.pop().unwrap();
        if let PendingEventOrShutdown::Event(e) = e1 {
            assert_eq!(e.strand_path, "file-a.md");
        } else {
            panic!("expected Event");
        }

        let e2 = queue.pop().unwrap();
        if let PendingEventOrShutdown::Event(e) = e2 {
            assert_eq!(e.strand_path, "file-b.md");
        } else {
            panic!("expected Event");
        }

        // Then the sentinel.
        let shutdown = queue.pop().unwrap();
        assert!(matches!(
            shutdown,
            PendingEventOrShutdown::Shutdown
        ));
    }

    // ── notified ──────────────────────────────────────────────────

    /// `notified` suspends until a producer signals.
    #[tokio::test]
    async fn notified_waits_for_push() {
        let queue = Arc::new(InMemoryEventQueue::new());
        let queue_clone = Arc::clone(&queue);

        let handle = tokio::spawn(async move {
            queue_clone.notified().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        queue.push(make_pending(created("file-a.md")));

        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            handle,
        )
        .await
        .expect("notified() should unblock after push")
        .unwrap();
    }

    /// `notified` unblocks after `push_shutdown`.
    #[tokio::test]
    async fn notified_waits_for_shutdown() {
        let queue = Arc::new(InMemoryEventQueue::new());
        let queue_clone = Arc::clone(&queue);

        let handle = tokio::spawn(async move {
            queue_clone.notified().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        queue.push_shutdown();

        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            handle,
        )
        .await
        .expect("notified() should unblock after shutdown")
        .unwrap();
    }

    // ── len / is_empty ────────────────────────────────────────────

    #[test]
    fn len_and_is_empty() {
        let queue = InMemoryEventQueue::new();
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());

        queue.push(make_pending(created("file-a.md")));
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());

        queue.pop();
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }
}
