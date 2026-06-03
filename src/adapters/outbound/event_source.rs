//! Notify-based file system event source adapter.
//!
//! Wraps `notify::RecommendedWatcher` and maps raw file system events
//! to `StrandEvent` domain types. Emits raw events to an mpsc channel —
//! the debounce engine (application layer) subscribes to this channel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::application::ports::{EventSource, PortError};
use crate::domain::entities::{KnotId, LoomId, StrandPath};
use crate::domain::events::StrandEvent;

// ── NotifyEventSource ──────────────────────────────────────────────────────

/// Shared mutable state between the watcher callback and the public API.
struct InnerState {
    sender: mpsc::Sender<StrandEvent>,
    watched_dirs: HashMap<PathBuf, (LoomId, KnotId)>,
}

impl InnerState {
    /// Map a raw notify event to a StrandEvent, filtering and enriching.
    ///
    /// Returns None for events that should be dropped:
    /// - Directory events (only files are watched)
    /// - Events outside watched directories
    /// - Non-create/modify/remove event kinds
    fn map_event(&self, event: &Event) -> Option<StrandEvent> {
        let path = event.paths.first()?;

        // Only process file events (skip directories)
        if path.is_dir() {
            return None;
        }

        // Only process events inside watched directories
        if !self.is_watched_path(path) {
            return None;
        }

        // Look up loom/knot IDs for this watched directory
        let (loom_id, knot_id) = self.find_ids_for_path(path)?;

        let strand_path = StrandPath(path.to_path_buf());

        match event.kind {
            EventKind::Create(_) => {
                Some(StrandEvent::Created {
                    loom_id,
                    knot_id,
                    strand_path,
                })
            }
            EventKind::Modify(_) => {
                Some(StrandEvent::Modified {
                    loom_id,
                    knot_id,
                    strand_path,
                })
            }
            EventKind::Remove(_) => {
                Some(StrandEvent::Deleted {
                    loom_id,
                    knot_id,
                    strand_path,
                })
            }
            // Access, Other, Any — not strand-relevant
            _ => None,
        }
    }

    /// Check if a path falls inside any watched directory.
    fn is_watched_path(&self, path: &Path) -> bool {
        self.watched_dirs
            .keys()
            .any(|dir| path.starts_with(dir))
    }

    /// Find the loom/knot IDs for a path's watched directory.
    fn find_ids_for_path(
        &self,
        path: &Path,
    ) -> Option<(LoomId, KnotId)> {
        self.watched_dirs
            .iter()
            .find(|(dir, _)| path.starts_with(dir))
            .map(|(_, ids)| ids.clone())
    }
}

/// File-system event source backed by the `notify` crate.
///
/// Wraps a `notify::RecommendedWatcher`, maps raw events to
/// `StrandEvent` domain types, and forwards them to the application
/// layer via an mpsc channel.
///
/// Only file-level events (not directories) are emitted.
/// Only events inside watched source directories are emitted.
pub struct NotifyEventSource {
    watcher: Mutex<RecommendedWatcher>,
    state: Arc<Mutex<InnerState>>,
}

impl NotifyEventSource {
    /// Create a new `NotifyEventSource` that sends events to `sender`.
    ///
    /// Uses `notify::Config` with a 50ms poll interval for consistent
    /// test behaviour across platforms.
    pub fn new(sender: mpsc::Sender<StrandEvent>) -> Self {
        let state = Arc::new(Mutex::new(InnerState {
            sender,
            watched_dirs: HashMap::new(),
        }));

        let state_clone = state.clone();

        let watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    let inner = state_clone.lock().unwrap();
                    if let Some(strand_event) = inner.map_event(&event) {
                        let _ = inner.sender.blocking_send(strand_event);
                    }
                }
            },
            notify::Config::default()
                .with_poll_interval(std::time::Duration::from_millis(50)),
        )
        .expect("failed to create notify watcher");

        Self {
            watcher: Mutex::new(watcher),
            state,
        }
    }

    /// Set the loom and knot IDs to attach to emitted events.
    ///
    /// Call this before `watch()` so events can carry the correct
    /// `loom_id` and `knot_id`. The IDs apply to all subsequently
    /// watched directories (used as a fallback).
    pub fn with_ids(self, loom_id: LoomId, knot_id: KnotId) -> Self {
        self.state
            .lock()
            .unwrap()
            .watched_dirs
            .insert(PathBuf::from("__default_ids__"), (loom_id, knot_id));
        self
    }

    /// Set the loom and knot IDs for a specific source directory.
    ///
    /// Call this for each loom's source directory before `watch()` so
    /// events carry the correct `loom_id` and `knot_id` per directory.
    /// This enables multiple looms to share a single event source.
    pub fn with_loom_ids(
        &self,
        source_dir: PathBuf,
        loom_id: LoomId,
        knot_id: KnotId,
    ) {
        self.state
            .lock()
            .unwrap()
            .watched_dirs
            .insert(source_dir, (loom_id, knot_id));
    }
}

impl EventSource for NotifyEventSource {
    fn watch(&self, path: &Path) -> Result<(), PortError> {
        {
            let mut inner = self.state.lock().unwrap();

            // Check if IDs were registered for this specific directory
            let ids = inner
                .watched_dirs
                .get(path)
                .cloned()
                // Fall back to default IDs if no per-directory mapping
                .or_else(|| {
                    inner.watched_dirs
                        .get(&PathBuf::from("__default_ids__"))
                        .cloned()
                })
                .unwrap_or_else(|| {
                    (
                        LoomId("unknown".to_string()),
                        KnotId("unknown".to_string()),
                    )
                });
            inner
                .watched_dirs
                .insert(path.to_path_buf(), ids);
        }

        self.watcher
            .lock()
            .unwrap()
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| PortError::EventWatchFailed(e.to_string()))?;

        Ok(())
    }

    fn unwatch(&self, path: &Path) -> Result<(), PortError> {
        self.state
            .lock()
            .unwrap()
            .watched_dirs
            .remove(path);

        self.watcher
            .lock()
            .unwrap()
            .unwatch(path)
            .map_err(|e| PortError::EventUnwatchFailed(e.to_string()))?;

        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Poll watcher interval plus buffer for reliable test timing.
    const POLL_DELAY: Duration = Duration::from_millis(300);

    /// Poll the channel for an event, retrying briefly.
    fn recv_event(
        rx: &mut mpsc::Receiver<StrandEvent>,
        timeout: Duration,
    ) -> Option<StrandEvent> {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if let Ok(event) = rx.try_recv() {
                return Some(event);
            }
            thread::sleep(Duration::from_millis(10));
        }
        None
    }

    fn create_source(
        loom_id: &str,
        knot_id: &str,
    ) -> (NotifyEventSource, mpsc::Receiver<StrandEvent>) {
        let (tx, rx) = mpsc::channel(100);
        let source = NotifyEventSource::new(tx).with_ids(
            LoomId(loom_id.to_string()),
            KnotId(knot_id.to_string()),
        );
        (source, rx)
    }

    #[test]
    fn watcher_starts() {
        let (source, _rx) = create_source("test-loom", "test-knot");
        let dir = TempDir::new().unwrap();

        assert!(
            source.watch(dir.path()).is_ok(),
            "watch() should succeed"
        );

        // unwatch() succeeds proves watcher was active
        assert!(
            source.unwatch(dir.path()).is_ok(),
            "unwatch() should succeed"
        );
    }

    #[test]
    fn create_event_emitted() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx) = create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let file_path = dir.path().join("new-strand.md");
        fs::write(&file_path, "content").unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Created event");
        match event {
            StrandEvent::Created {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(strand_path.0.file_name().unwrap(), "new-strand.md");
            }
            other => panic!("Expected Created event, got: {:?}", other),
        }
    }

    #[test]
    fn modify_event_emitted() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx) = create_source("loom-1", "knot-1");

        let file_path = dir.path().join("existing.md");
        fs::write(&file_path, "initial").unwrap();

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "updated content").unwrap();
        drop(file);

        thread::sleep(POLL_DELAY);

        let event = recv_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Modified event");
        match event {
            StrandEvent::Modified {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(strand_path.0.file_name().unwrap(), "existing.md");
            }
            other => panic!("Expected Modified event, got: {:?}", other),
        }
    }

    #[test]
    fn delete_event_emitted() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx) = create_source("loom-1", "knot-1");

        let file_path = dir.path().join("to-delete.md");
        fs::write(&file_path, "will be deleted").unwrap();

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        fs::remove_file(&file_path).unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Deleted event");
        match event {
            StrandEvent::Deleted {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(strand_path.0.file_name().unwrap(), "to-delete.md");
            }
            other => panic!("Expected Deleted event, got: {:?}", other),
        }
    }

    #[test]
    fn directory_events_filtered() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx) = create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let subdir = dir.path().join("new-subdir");
        fs::create_dir(&subdir).unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_event(&mut rx, Duration::from_millis(200)).is_none(),
            "directory creation should not emit events"
        );
    }

    #[test]
    fn event_outside_source_dir_filtered() {
        let watched_dir = TempDir::new().unwrap();
        let outside_dir = TempDir::new().unwrap();
        let (source, mut rx) = create_source("loom-1", "knot-1");

        source.watch(watched_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let outside_file = outside_dir.path().join("outside.md");
        fs::write(&outside_file, "outside content").unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_event(&mut rx, Duration::from_millis(200)).is_none(),
            "events outside watched dir should not be emitted"
        );
    }

    #[test]
    fn event_mapping_correct_types() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx) = create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // 1. Create → StrandEvent::Created
        let file_path = dir.path().join("mapping-test.md");
        fs::write(&file_path, "initial").unwrap();
        thread::sleep(POLL_DELAY);

        let event = recv_event(&mut rx, Duration::from_millis(500))
            .expect("should receive Create event");
        assert!(
            matches!(event, StrandEvent::Created { .. }),
            "Create should map to StrandEvent::Created, got: {:?}", event
        );

        // 2. Modify → StrandEvent::Modified
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "modified").unwrap();
        drop(file);
        thread::sleep(POLL_DELAY);

        let event = recv_event(&mut rx, Duration::from_millis(500))
            .expect("should receive Modify event");
        assert!(
            matches!(event, StrandEvent::Modified { .. }),
            "Modify should map to StrandEvent::Modified, got: {:?}", event
        );

        // 3. Remove → StrandEvent::Deleted
        fs::remove_file(&file_path).unwrap();
        thread::sleep(POLL_DELAY);

        // PollWatcher may emit Modify before Remove; drain and check last
        let mut last_event: Option<StrandEvent> = None;
        while let Some(event) = recv_event(&mut rx, Duration::from_millis(100)) {
            last_event = Some(event);
        }
        let event = last_event.expect("should receive at least one event for remove");
        assert!(
            matches!(event, StrandEvent::Deleted { .. }),
            "Remove should map to StrandEvent::Deleted, got: {:?}", event
        );
    }
}
