//! Disk-backed implementation of [`StrandEventQueue`].
//!
//! The disk **is** the queue — there is no in-memory event index. Every
//! operation reads from `rig/events/*.json` and orders by filename sort
//! (timestamp prefix ensures FIFO). This eliminates any dual-source-of-
//! truth: the `.json` files are the queue.
//!
//! Internal state is minimal: a [`tokio::sync::Notify`] for signaling
//! and a flag for the shutdown sentinel (which is NOT persisted to disk).

use std::path::PathBuf;
use std::sync::Mutex;

use crate::application::ports::StrandEventQueue;
use crate::domain::events::StrandQueueAccessor;
use crate::domain::pending_event::{
    dedup_key, PendingEvent, PendingEventId, PendingEventOrShutdown,
};

use super::event_store::FileSystemEventStore;

/// Disk-backed implementation of [`StrandEventQueue`].
///
/// The disk **is** the queue. Every operation reads `rig/events/*.json`
/// from disk and orders by filename sort. No in-memory event storage —
/// only a [`tokio::sync::Notify`] for signaling and a shutdown sentinel
/// flag (not persisted).
pub struct DiskBackedEventQueue {
    store: FileSystemEventStore,
    notify: tokio::sync::Notify,
    shutdown: Mutex<bool>,
}

impl DiskBackedEventQueue {
    /// Create a new disk-backed queue pointing at `events_dir`.
    ///
    /// Creates the directory if it does not exist. The queue starts empty
    /// (no persisted events are loaded — call [`load_persisted`]
    /// explicitly at startup).
    pub fn new(events_dir: PathBuf) -> Self {
        Self {
            store: FileSystemEventStore::new(events_dir),
            notify: tokio::sync::Notify::new(),
            shutdown: Mutex::new(false),
        }
    }

    /// Scan persisted event files and push them into the queue.
    ///
    /// Reads all `.json` files from the events directory, deduplicates
    /// them via [`push_or_replace`], and returns the number of events
    /// loaded. Malformed files are skipped with a warning (handled by
    /// [`FileSystemEventStore::scan_events`]).
    ///
    /// This should be called once at startup, **before** the file-watcher
    /// begins emitting new events, to preserve FIFO ordering across
    /// restart boundaries.
    pub fn load_persisted(&self) -> usize {
        let events = self.store.scan_events().unwrap_or_default();
        let count = events.len();

        // Push each event through push_or_replace to deduplicate
        // (e.g. same strand modified multiple times before restart).
        for event in events {
            self.push_or_replace(event);
        }

        // Don't signal notify during load — no consumer is waiting yet.
        count
    }
}

impl Default for DiskBackedEventQueue {
    fn default() -> Self {
        // Default to a temp dir for tests that don't configure one.
        Self::new(std::env::temp_dir().join(format!(
            "knot-events-{:?}",
            std::process::id()
        )))
    }
}

impl StrandEventQueue for DiskBackedEventQueue {
    fn push(&self, event: PendingEvent) -> PendingEventId {
        let id = event.id.clone();
        self.store.write_event(&event).expect(
            "failed to write event file — check disk permissions",
        );
        self.notify.notify_one();
        id
    }

    fn push_or_replace(&self, event: PendingEvent) -> PendingEventId {
        // Scan all existing events and check dedup keys.
        let existing = self.store.scan_events().unwrap_or_default();
        for existing_event in &existing {
            if dedup_key(existing_event) == dedup_key(&event) {
                // Match found — remove the old file
                self.store
                    .remove_event(&existing_event.id)
                    .expect("failed to remove old event file during dedup");
            }
        }

        // Always write the new event (moves to back of queue with new timestamp)
        let id = event.id.clone();
        self.store.write_event(&event).expect(
            "failed to write event file — check disk permissions",
        );
        self.notify.notify_one();
        id
    }

    fn pop(&self) -> Option<PendingEventOrShutdown> {
        // Scan all events sorted by filename (FIFO order).
        let events = self.store.scan_events().unwrap_or_default();

        if let Some(first) = events.first() {
            // Read the file fresh from disk (honours any on-disk edits).
            let event = self
                .store
                .read_event(&first.id)
                .expect("failed to read event file on pop — file may have been removed by another process");

            // Remove the file from disk.
            self.store
                .remove_event(&event.id)
                .expect("failed to remove event file after pop");

            return Some(PendingEventOrShutdown::Event(event));
        }

        // No events — check shutdown flag.
        if *self.shutdown.lock().unwrap() {
            Some(PendingEventOrShutdown::Shutdown)
        } else {
            None
        }
    }

    fn snapshot(&self) -> Vec<PendingEvent> {
        // Read fresh from disk — always reflects on-disk state.
        self.store.scan_events().unwrap_or_default()
    }

    fn delete(&self, id: &PendingEventId) -> bool {
        let path = self.store.event_path(id);
        if path.exists() {
            self.store.remove_event(id).expect(
                "failed to remove event file during delete",
            );
            true
        } else {
            false
        }
    }

    fn pending_event(&self, id: &PendingEventId) -> Option<PendingEvent> {
        self.store.read_event(id).ok()
    }

    fn len(&self) -> usize {
        self.store.event_count()
    }

    fn is_empty(&self) -> bool {
        self.store.event_count() == 0
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

impl std::fmt::Debug for DiskBackedEventQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskBackedEventQueue")
            .field("events_dir", &self.store.events_dir())
            .field("len", &self.store.event_count())
            .finish_non_exhaustive()
    }
}

impl StrandQueueAccessor for DiskBackedEventQueue {
    fn pending_strand_paths(&self) -> Vec<PathBuf> {
        self.store
            .scan_events()
            .unwrap_or_default()
            .iter()
            .map(|e| PathBuf::from(&e.strand_path))
            .collect()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{KnotId, LoomId, StrandPath};
    use crate::domain::events::StrandEvent;
    use std::path::PathBuf;

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

    // ── push / snapshot ─────────────────────────────────────────────

    /// `push` writes a file to disk; `snapshot` returns it.
    #[test]
    fn push_writes_file_and_snapshot_returns_it() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        let event = make_pending(created("/file-a.md"));
        let id = queue.push(event.clone());
        assert_eq!(id.0, event.id.0);

        let snap = queue.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].strand_path, "/file-a.md");
    }

    // ── pop ─────────────────────────────────────────────────────────

    /// `pop` reads the first file (by filename sort), removes it from
    /// disk, and returns the content.
    #[test]
    fn pop_reads_first_file_and_removes_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        // Use fixed IDs so we control FIFO order
        let mut e1 = make_pending(created("/file-a.md"));
        e1.id = PendingEventId("1001-aaa".to_string());
        let mut e2 = make_pending(created("/file-b.md"));
        e2.id = PendingEventId("1002-bbb".to_string());
        queue.push(e1);
        queue.push(e2);

        assert_eq!(queue.len(), 2);

        let result = queue.pop().unwrap();
        match result {
            PendingEventOrShutdown::Event(e) => {
                assert_eq!(e.strand_path, "/file-a.md");
            }
            PendingEventOrShutdown::Shutdown => {
                panic!("expected Event, got Shutdown");
            }
        }

        assert_eq!(queue.len(), 1);

        let second = queue.pop().unwrap();
        match second {
            PendingEventOrShutdown::Event(e) => {
                assert_eq!(e.strand_path, "/file-b.md");
            }
            PendingEventOrShutdown::Shutdown => {
                panic!("expected Event, got Shutdown");
            }
        }

        assert!(queue.is_empty());
    }

    /// `pop` returns `None` when no files exist and shutdown not signalled.
    #[test]
    fn pop_returns_none_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        assert!(queue.pop().is_none());
    }

    // ── push_or_replace ─────────────────────────────────────────────

    /// `push_or_replace` replaces existing event with same dedup key:
    /// old file removed, new file written.
    #[test]
    fn push_or_replace_replaces_existing() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        let mut e1 = make_pending(created("/file-a.md"));
        e1.id = PendingEventId("1001-aaa".to_string());
        let e2 = make_pending(created("/file-b.md"));
        queue.push_or_replace(e1.clone());
        queue.push_or_replace(e2);

        // Now push a replacement for e1 (same dedup key)
        let mut e1_replaced = make_pending(created("/file-a.md"));
        e1_replaced.id = PendingEventId("1003-ccc".to_string());
        let e1_new_id = e1_replaced.id.clone();
        queue.push_or_replace(e1_replaced);

        // Queue should still have 2 events (one replaced, one unchanged)
        assert_eq!(queue.len(), 2);

        // The old file should be gone (push_or_replace already removed it)
        let old_path = dir.path().join("1001-aaa.json");
        assert!(!old_path.exists(), "old event file should be removed by push_or_replace");

        // Snapshot should contain the new ID for file-a
        let snap = queue.snapshot();
        let file_a = snap.iter().find(|e| e.strand_path == "/file-a.md").unwrap();
        assert_eq!(file_a.id, e1_new_id);
    }

    /// `push_or_replace` pushes a new event when no matching dedup key.
    #[test]
    fn push_or_replace_pushes_new_when_different_key() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        queue.push_or_replace(make_pending(created("/file-a.md")));
        queue.push_or_replace(make_pending(modified("/file-a.md"))); // different kind
        queue.push_or_replace(make_pending(created("/file-b.md")));  // different path

        assert_eq!(queue.len(), 3);
    }

    // ── delete ──────────────────────────────────────────────────────

    /// `delete` removes the file from disk; returns `true` if file existed.
    #[test]
    fn delete_removes_file_and_returns_true() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        let event = make_pending(created("/file-a.md"));
        let id = queue.push(event);

        assert_eq!(queue.len(), 1);
        assert!(queue.delete(&id));
        assert_eq!(queue.len(), 0);
    }

    /// `delete` returns `false` for non-existent ID.
    #[test]
    fn delete_returns_false_for_nonexistent_id() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());
        let id = PendingEventId("9999-zzzz".to_string());

        assert!(!queue.delete(&id));
    }

    // ── pending_event ───────────────────────────────────────────────

    /// `pending_event` reads the file from disk and returns the event.
    #[test]
    fn pending_event_reads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        let event = make_pending(created("/file-a.md"));
        let id = queue.push(event);

        let found = queue.pending_event(&id).unwrap();
        assert_eq!(found.strand_path, "/file-a.md");
        assert_eq!(found.kind, "Created");
    }

    /// `pending_event` returns `None` for non-existent ID.
    #[test]
    fn pending_event_returns_none_for_nonexistent_id() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());
        let id = PendingEventId("9999-zzzz".to_string());

        assert!(queue.pending_event(&id).is_none());
    }

    // ── snapshot ────────────────────────────────────────────────────

    /// `snapshot` returns all events sorted by filename.
    #[test]
    fn snapshot_returns_all_events_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        // Use fixed IDs so we control sort order
        let mut e1 = make_pending(created("/file-a.md"));
        e1.id = PendingEventId("1001-aaa".to_string());
        let mut e2 = make_pending(created("/file-b.md"));
        e2.id = PendingEventId("1002-bbb".to_string());
        let mut e3 = make_pending(created("/file-c.md"));
        e3.id = PendingEventId("1003-ccc".to_string());

        // Push out of order — snapshot should still return sorted by filename
        queue.push(e3);
        queue.push(e1);
        queue.push(e2);

        let snap = queue.snapshot();
        assert_eq!(snap.len(), 3);
        assert_eq!(snap[0].id.0, "1001-aaa");
        assert_eq!(snap[0].strand_path, "/file-a.md");
        assert_eq!(snap[1].id.0, "1002-bbb");
        assert_eq!(snap[1].strand_path, "/file-b.md");
        assert_eq!(snap[2].id.0, "1003-ccc");
        assert_eq!(snap[2].strand_path, "/file-c.md");
    }

    // ── len / is_empty ──────────────────────────────────────────────

    #[test]
    fn len_and_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());

        queue.push(make_pending(created("/file-a.md")));
        assert_eq!(queue.len(), 1);
        assert!(!queue.is_empty());

        queue.pop();
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }

    // ── shutdown sentinel ───────────────────────────────────────────

    /// `push_shutdown` sets the sentinel flag; after last event popped,
    /// `pop` returns `Shutdown`.
    #[test]
    fn shutdown_sentinel_after_last_event() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        queue.push(make_pending(created("/file-a.md")));
        queue.push_shutdown();

        // Event comes first
        let e = queue.pop().unwrap();
        match e {
            PendingEventOrShutdown::Event(_) => {}
            PendingEventOrShutdown::Shutdown => panic!("expected Event first"),
        }

        // Then shutdown
        let shutdown = queue.pop().unwrap();
        match shutdown {
            PendingEventOrShutdown::Shutdown => {}
            PendingEventOrShutdown::Event(_) => panic!("expected Shutdown"),
        }
    }

    /// `push_shutdown` on empty queue returns `Shutdown` immediately.
    #[test]
    fn shutdown_sentinel_on_empty_queue() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        queue.push_shutdown();
        let result = queue.pop();
        match result {
            Some(PendingEventOrShutdown::Shutdown) => {}
            _ => panic!("expected Shutdown"),
        }
    }

    // ── notified ────────────────────────────────────────────────────

    /// `notified` unblocks after `push`.
    #[tokio::test]
    async fn notified_unblocks_after_push() {
        let dir = tempfile::tempdir().unwrap();
        let queue = std::sync::Arc::new(
            DiskBackedEventQueue::new(dir.path().to_path_buf()),
        );
        let queue_clone = std::sync::Arc::clone(&queue);

        let handle = tokio::spawn(async move {
            queue_clone.notified().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        queue.push(make_pending(created("/file-a.md")));

        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            handle,
        )
        .await
        .expect("notified() should unblock after push")
        .unwrap();
    }

    // ── round-trip ──────────────────────────────────────────────────

    /// Push 3 events, pop all 3, disk is empty.
    #[test]
    fn roundtrip_push_three_pop_all_disk_empty() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        queue.push(make_pending(created("/a.md")));
        queue.push(make_pending(created("/b.md")));
        queue.push(make_pending(created("/c.md")));

        assert_eq!(queue.len(), 3);

        for _ in 0..3 {
            queue.pop();
        }

        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }

    // ── crash survival ──────────────────────────────────────────────

    /// Push events, don't pop (simulate crash), create new queue —
    /// verify events are present on disk and loadable.
    #[test]
    fn events_survive_on_disk_across_queue_recreation() {
        let dir = tempfile::tempdir().unwrap();
        let events_dir = dir.path().to_path_buf();

        // First queue: push events but don't pop
        {
            let queue = DiskBackedEventQueue::new(events_dir.clone());
            queue.push(make_pending(created("/file-a.md")));
            queue.push(make_pending(created("/file-b.md")));
            assert_eq!(queue.len(), 2);
            // Drop the queue (simulates process death)
        }

        // Second queue: recreate pointing at same directory
        {
            let queue = DiskBackedEventQueue::new(events_dir.clone());

            // Events should still be on disk
            assert_eq!(queue.len(), 2);

            // load_persisted should find them
            let count = queue.load_persisted();
            // load_persisted pushes through push_or_replace which dedupes,
            // but since files are already there and we scan them, the count
            // reflects what was found on disk.
            assert_eq!(count, 2, "should have loaded 2 persisted events");
        }
    }

    // ── on-disk modification ────────────────────────────────────────

    /// Modify an event file on disk, then `pop` returns the updated content.
    #[test]
    fn pop_returns_updated_content_after_on_disk_modification() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        let event = make_pending(created("/file-a.md"));
        queue.push(event.clone());

        // Modify the file on disk: change the kind to "Modified"
        let mut modified_event = event.clone();
        modified_event.kind = "Modified".to_string();

        let event_path = dir.path().join(format!("{}.json", event.id.0));
        std::fs::write(
            &event_path,
            serde_json::to_string_pretty(&modified_event).unwrap(),
        )
        .unwrap();

        // Pop should return the updated content
        let result = queue.pop().unwrap();
        match result {
            PendingEventOrShutdown::Event(e) => {
                assert_eq!(e.kind, "Modified", "should reflect on-disk edit");
            }
            PendingEventOrShutdown::Shutdown => {
                panic!("expected Event");
            }
        }
    }

    /// `snapshot` after on-disk modification returns updated content.
    #[test]
    fn snapshot_after_on_disk_modification_returns_updated_content() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        let event = make_pending(created("/file-a.md"));
        queue.push(event.clone());

        // Modify the file on disk
        let mut modified_event = event.clone();
        modified_event.kind = "Deleted".to_string();

        let event_path = dir.path().join(format!("{}.json", event.id.0));
        std::fs::write(
            &event_path,
            serde_json::to_string_pretty(&modified_event).unwrap(),
        )
        .unwrap();

        // Snapshot should reflect the change
        let snap = queue.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].kind, "Deleted", "snapshot should reflect on-disk edit");
    }

    // ── StrandQueueAccessor ─────────────────────────────────────────

    /// `pending_strand_paths` returns paths from snapshot.
    #[test]
    fn pending_strand_paths_returns_paths() {
        let dir = tempfile::tempdir().unwrap();
        let queue = DiskBackedEventQueue::new(dir.path().to_path_buf());

        queue.push(make_pending(created("/file-a.md")));
        queue.push(make_pending(created("/file-b.md")));

        let paths = queue.pending_strand_paths();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&PathBuf::from("/file-a.md")));
        assert!(paths.contains(&PathBuf::from("/file-b.md")));
    }
}
