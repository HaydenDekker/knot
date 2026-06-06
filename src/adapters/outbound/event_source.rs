//! Notify-based file system event source adapter.
//!
//! Wraps `notify::RecommendedWatcher` and maps raw file system events
//! to `StrandEvent` and `ConfigEvent` domain types. Emits events to
//! two mpsc channels — the debounce engine (application layer)
//! subscribes to the strand channel, and the `ConfigEventHandler`
//! subscribes to the config channel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::application::ports::{EventSource, PortError};
use crate::domain::entities::{Knot, KnotId, LoomId, StrandPath};
use crate::domain::events::{ConfigEvent, StrandEvent};
use crate::domain::knot_file;

// ── WatchType ──────────────────────────────────────────────────────────────

/// The type of watch being registered, determining how events are mapped.
#[derive(Debug, Clone)]
pub enum WatchType {
    /// Strand directory watch — maps file events to `StrandEvent`.
    /// Carries the loom and knot IDs for the watched directory.
    Strand(LoomId, KnotId),
    /// Rig directory watch — maps new `*-loom` directory creation
    /// to `ConfigEvent::LoomAdded`.
    Rig,
    /// Loom directory watch — maps `.md` file create/modify/delete
    /// to `ConfigEvent::Knot*`. Carries the loom ID.
    Loom(LoomId),
}

// ── NotifyEventSource ──────────────────────────────────────────────────────

/// Shared mutable state between the watcher callback and the public API.
struct InnerState {
    strand_sender: mpsc::Sender<StrandEvent>,
    config_sender: mpsc::Sender<ConfigEvent>,
    watched_dirs: HashMap<PathBuf, WatchType>,
}

impl InnerState {
    /// Map a raw notify event to a `StrandEvent` or `ConfigEvent`,
    /// filtering and enriching based on the watch type.
    ///
    /// Returns a tuple of (Option<StrandEvent>, Option<ConfigEvent>).
    /// At most one of the two options will be `Some`.
    fn map_event(
        &self,
        event: &Event,
    ) -> (Option<StrandEvent>, Option<ConfigEvent>) {
        let path = match event.paths.first() {
            Some(p) => p,
            None => return (None, None),
        };

        // Find the watch type for this path
        let watch_type = match self.find_watch_type(path) {
            Some(wt) => wt,
            None => return (None, None),
        };

        match &watch_type {
            WatchType::Strand(loom_id, knot_id) => {
                self.map_strand_event(event, path, loom_id, knot_id)
            }
            WatchType::Rig => {
                self.map_rig_event(event, path)
            }
            WatchType::Loom(loom_id) => {
                self.map_loom_event(event, path, loom_id)
            }
        }
    }

    /// Map events for strand directory watches.
    fn map_strand_event(
        &self,
        event: &Event,
        path: &Path,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) -> (Option<StrandEvent>, Option<ConfigEvent>) {
        // Only process file events (skip directories)
        if path.is_dir() {
            return (None, None);
        }

        // Only process `.md` files
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            return (None, None);
        }

        let strand_path = StrandPath(path.to_path_buf());

        let strand_event = match event.kind {
            EventKind::Create(_) => Some(StrandEvent::Created {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path,
            }),
            EventKind::Modify(_) => Some(StrandEvent::Modified {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path,
            }),
            EventKind::Remove(_) => Some(StrandEvent::Deleted {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path,
            }),
            _ => None,
        };

        (strand_event, None)
    }

    /// Map events for rig directory watches.
    ///
    /// Detects new `*-loom` directories and emits `ConfigEvent::LoomAdded`.
    fn map_rig_event(
        &self,
        event: &Event,
        path: &Path,
    ) -> (Option<StrandEvent>, Option<ConfigEvent>) {
        // Only process directory creation events
        if !path.is_dir() {
            return (None, None);
        }

        let config_event = if matches!(event.kind, EventKind::Create(_)) {
            // Check if directory name ends with `-loom`
            if let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
            {
                if name.ends_with("-loom") {
                    let loom_id = LoomId(name.to_string());
                    return (
                        None,
                        Some(ConfigEvent::LoomAdded { loom_id }),
                    );
                }
            }
            None
        } else {
            None
        };

        (None, config_event)
    }

    /// Map events for loom directory watches.
    ///
    /// Maps `.md` file create/modify/delete to `ConfigEvent::Knot*`.
    fn map_loom_event(
        &self,
        event: &Event,
        path: &Path,
        loom_id: &LoomId,
    ) -> (Option<StrandEvent>, Option<ConfigEvent>) {
        // Only process `.md` files (skip directories and other files)
        if path.is_dir() {
            return (None, None);
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            return (None, None);
        }

        let config_event = match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                // Parse the knot file to get the full Knot entity
                match std::fs::read_to_string(path) {
                    Ok(content) => match knot_file::parse(&content) {
                        Ok(knot_file) => {
                            let knot = Knot {
                                id: KnotId(knot_file.name.clone()),
                                agent_config: knot_file.agent_config,
                                prompt_template: knot_file.prompt_template,
                                strand_dir: knot_file.strand_dir,
                                tie_off_dir: knot_file.tie_off_dir,
                            };
                            if matches!(event.kind, EventKind::Create(_)) {
                                Some(ConfigEvent::KnotAdded {
                                    loom_id: loom_id.clone(),
                                    knot,
                                })
                            } else {
                                Some(ConfigEvent::KnotModified {
                                    loom_id: loom_id.clone(),
                                    knot,
                                })
                            }
                        }
                        Err(_) => {
                            // File parse failed — skip silently
                            // The file may be incomplete during write
                            None
                        }
                    },
                    Err(_) => None,
                }
            }
            EventKind::Remove(_) => {
                // Extract knot name from filename (remove .md extension)
                if let Some(name) = path
                    .file_stem()
                    .and_then(|n| n.to_str())
                {
                    Some(ConfigEvent::KnotDeleted {
                        loom_id: loom_id.clone(),
                        knot_id: KnotId(name.to_string()),
                    })
                } else {
                    None
                }
            }
            _ => None,
        };

        (None, config_event)
    }

    /// Find the watch type for a path by checking watched directories.
    fn find_watch_type(&self, path: &Path) -> Option<WatchType> {
        self.watched_dirs
            .iter()
            .find(|(dir, _)| path.starts_with(dir))
            .map(|(_, wt)| wt.clone())
    }
}

/// File-system event source backed by the `notify` crate.
///
/// Wraps a `notify::RecommendedWatcher`, maps raw events to
/// `StrandEvent` and `ConfigEvent` domain types, and forwards them
/// to the application layer via mpsc channels.
pub struct NotifyEventSource {
    watcher: Mutex<RecommendedWatcher>,
    state: Arc<Mutex<InnerState>>,
}

impl NotifyEventSource {
    /// Create a new `NotifyEventSource` with two event channels.
    ///
    /// `strand_sender` receives `StrandEvent` from strand directory
    /// watches. `config_sender` receives `ConfigEvent` from rig and
    /// loom directory watches.
    ///
    /// Uses `notify::Config` with a 50ms poll interval for consistent
    /// test behaviour across platforms.
    pub fn new(
        strand_sender: mpsc::Sender<StrandEvent>,
        config_sender: mpsc::Sender<ConfigEvent>,
    ) -> Self {
        let state = Arc::new(Mutex::new(InnerState {
            strand_sender,
            config_sender,
            watched_dirs: HashMap::new(),
        }));

        let state_clone = state.clone();

        let watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| {
                if let Ok(event) = result {
                    let inner = state_clone.lock().unwrap();
                    let (strand_event, config_event) =
                        inner.map_event(&event);
                    if let Some(se) = strand_event {
                        let _ = inner.strand_sender.blocking_send(se);
                    }
                    if let Some(ce) = config_event {
                        let _ = inner.config_sender.blocking_send(ce);
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

    /// Register a watch type for a specific directory.
    ///
    /// Call this before `watch()` so events carry the correct metadata
    /// and map to the right event type.
    pub fn register_watch(&self, path: PathBuf, watch_type: WatchType) {
        self.state
            .lock()
            .unwrap()
            .watched_dirs
            .insert(path, watch_type);
    }

    /// Set the loom and knot IDs for a strand source directory.
    ///
    /// Convenience method equivalent to
    /// `register_watch(path, WatchType::Strand(loom_id, knot_id))`.
    /// Maintains backward compatibility with existing callers.
    pub fn with_loom_ids(
        &self,
        source_dir: PathBuf,
        loom_id: LoomId,
        knot_id: KnotId,
    ) {
        self.register_watch(
            source_dir,
            WatchType::Strand(loom_id, knot_id),
        );
    }

    /// Set default strand IDs (used as fallback).
    ///
    /// Maintains backward compatibility with builder-style `with_ids`.
    pub fn with_ids(self, loom_id: LoomId, knot_id: KnotId) -> Self {
        self.register_watch(
            PathBuf::from("__default_ids__"),
            WatchType::Strand(loom_id, knot_id),
        );
        self
    }
}

impl EventSource for NotifyEventSource {
    fn set_loom_ids(
        &self,
        source_dir: &Path,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) {
        self.with_loom_ids(
            source_dir.to_path_buf(),
            loom_id.clone(),
            knot_id.clone(),
        );
    }

    fn watch(&self, path: &Path) -> Result<(), PortError> {
        // Check if a watch type was registered for this path
        let watch_type = {
            let inner = self.state.lock().unwrap();
            inner
                .watched_dirs
                .get(path)
                .cloned()
                // Fall back to default IDs if no per-directory mapping
                .or_else(|| {
                    inner
                        .watched_dirs
                        .get(&PathBuf::from("__default_ids__"))
                        .cloned()
                })
        };

        if let Some(ref wt) = watch_type {
            // Ensure the path is in the map for event lookup
            self.state
                .lock()
                .unwrap()
                .watched_dirs
                .insert(path.to_path_buf(), wt.clone());
        } else {
            // Default: treat as strand watch with unknown IDs
            let default = WatchType::Strand(
                LoomId("unknown".to_string()),
                KnotId("unknown".to_string()),
            );
            self.state
                .lock()
                .unwrap()
                .watched_dirs
                .insert(path.to_path_buf(), default);
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

    /// Poll the strand channel for an event, retrying briefly.
    fn recv_strand_event(
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

    /// Poll the config channel for an event, retrying briefly.
    fn recv_config_event(
        rx: &mut mpsc::Receiver<ConfigEvent>,
        timeout: Duration,
    ) -> Option<ConfigEvent> {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if let Ok(event) = rx.try_recv() {
                return Some(event);
            }
            thread::sleep(Duration::from_millis(10));
        }
        None
    }

    /// Build a `NotifyEventSource` for strand event tests (backward compat).
    fn create_source(
        loom_id: &str,
        knot_id: &str,
    ) -> (
        NotifyEventSource,
        mpsc::Receiver<StrandEvent>,
        mpsc::Receiver<ConfigEvent>,
    ) {
        let (strand_tx, strand_rx) = mpsc::channel(100);
        let (config_tx, config_rx) = mpsc::channel(100);
        let source = NotifyEventSource::new(strand_tx, config_tx).with_ids(
            LoomId(loom_id.to_string()),
            KnotId(knot_id.to_string()),
        );
        (source, strand_rx, config_rx)
    }

    /// Build a `NotifyEventSource` with fresh channels (no default IDs).
    fn create_source_fresh(
    ) -> (
        NotifyEventSource,
        mpsc::Receiver<StrandEvent>,
        mpsc::Receiver<ConfigEvent>,
    ) {
        let (strand_tx, strand_rx) = mpsc::channel(100);
        let (config_tx, config_rx) = mpsc::channel(100);
        let source = NotifyEventSource::new(strand_tx, config_tx);
        (source, strand_rx, config_rx)
    }

    // ── Strand Event Tests (backward compatibility) ─────────────────────

    #[test]
    fn watcher_starts() {
        let (source, _strand_rx, _config_rx) =
            create_source("test-loom", "test-knot");
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
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let file_path = dir.path().join("new-strand.md");
        fs::write(&file_path, "content").unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
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
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        let file_path = dir.path().join("existing.md");
        fs::write(&file_path, "initial").unwrap();

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "updated content").unwrap();
        drop(file);

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
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
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        let file_path = dir.path().join("to-delete.md");
        fs::write(&file_path, "will be deleted").unwrap();

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        fs::remove_file(&file_path).unwrap();

        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
            .expect("should receive a Deleted event");
        match event {
            StrandEvent::Deleted {
                loom_id,
                knot_id,
                strand_path,
            } => {
                assert_eq!(loom_id.0, "loom-1");
                assert_eq!(knot_id.0, "knot-1");
                assert_eq!(
                    strand_path.0.file_name().unwrap(),
                    "to-delete.md"
                );
            }
            other => panic!("Expected Deleted event, got: {:?}", other),
        }
    }

    #[test]
    fn directory_events_filtered() {
        let dir = TempDir::new().unwrap();
        let (source, mut strand_rx, mut config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let subdir = dir.path().join("new-subdir");
        fs::create_dir(&subdir).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "directory creation should not emit strand events"
        );
        // Config channel: no events (this is a strand watch, not a rig watch)
        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "directory creation should not emit config events on strand watch"
        );
    }

    #[test]
    fn event_outside_source_dir_filtered() {
        let watched_dir = TempDir::new().unwrap();
        let outside_dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(watched_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        let outside_file = outside_dir.path().join("outside.md");
        fs::write(&outside_file, "outside content").unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_strand_event(&mut rx, Duration::from_millis(200)).is_none(),
            "events outside watched dir should not be emitted"
        );
    }

    #[test]
    fn event_mapping_correct_types() {
        let dir = TempDir::new().unwrap();
        let (source, mut rx, _config_rx) =
            create_source("loom-1", "knot-1");

        source.watch(dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // 1. Create → StrandEvent::Created
        let file_path = dir.path().join("mapping-test.md");
        fs::write(&file_path, "initial").unwrap();
        thread::sleep(POLL_DELAY);

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
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

        let event = recv_strand_event(&mut rx, Duration::from_millis(500))
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
        while let Some(event) =
            recv_strand_event(&mut rx, Duration::from_millis(100))
        {
            last_event = Some(event);
        }
        let event = last_event
            .expect("should receive at least one event for remove");
        assert!(
            matches!(event, StrandEvent::Deleted { .. }),
            "Remove should map to StrandEvent::Deleted, got: {:?}", event
        );
    }

    // ── Config Event Tests (Phase 3: Rig/Loom Watching) ────────────────

    /// Watch rig directory with `WatchType::Rig`; create a `*-loom`
    /// directory → `ConfigEvent::LoomAdded` emitted on config channel.
    #[test]
    fn rig_dir_new_loom_emits_config_event() {
        let rig_dir = TempDir::new().unwrap();
        let (source, mut strand_rx, mut config_rx) = create_source_fresh();

        // Register rig watch
        source.register_watch(
            rig_dir.path().to_path_buf(),
            WatchType::Rig,
        );
        source.watch(rig_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a new loom directory
        let loom_dir = rig_dir.path().join("my-loom");
        fs::create_dir(&loom_dir).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "rig watch should not emit strand events"
        );

        // Config channel: should receive LoomAdded
        let event =
            recv_config_event(&mut config_rx, Duration::from_millis(500))
                .expect("should receive a LoomAdded config event");
        match event {
            ConfigEvent::LoomAdded { loom_id } => {
                assert_eq!(loom_id.0, "my-loom");
            }
            other => panic!(
                "Expected ConfigEvent::LoomAdded, got: {:?}",
                other
            ),
        }
    }

    /// Watch loom directory with `WatchType::Loom(id)`; create a `.md`
    /// file with valid knot frontmatter → `ConfigEvent::KnotAdded`
    /// emitted on config channel.
    #[test]
    fn loom_dir_new_knot_emits_config_event() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, mut strand_rx, mut config_rx) = create_source_fresh();

        // Register loom watch
        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id.clone()),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a new knot .md file with valid frontmatter
        let knot_content = "---
name: new-knot
agent-config:
  goal: \"Review documents\"
  provider: \"openai\"
  model: \"gpt-4o\"
strand-dir: \"strands\"
tie-off-dir: \"tie-offs\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review the document\"
---

# New Knot
";
        let knot_path = loom_dir.path().join("new-knot.md");
        fs::write(&knot_path, knot_content).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "loom watch should not emit strand events"
        );

        // Config channel: should receive KnotAdded
        let event =
            recv_config_event(&mut config_rx, Duration::from_millis(500))
                .expect("should receive a KnotAdded config event");
        match event {
            ConfigEvent::KnotAdded {
                loom_id: lid,
                knot,
            } => {
                assert_eq!(lid.0, "test-loom");
                assert_eq!(knot.id.0, "new-knot");
                assert_eq!(knot.agent_config.provider, "openai");
            }
            other => panic!(
                "Expected ConfigEvent::KnotAdded, got: {:?}",
                other
            ),
        }
    }

    /// Watch loom directory with `WatchType::Loom(id)`; edit an existing
    /// `.md` knot file → `ConfigEvent::KnotModified` emitted on config
    /// channel.
    #[test]
    fn loom_dir_edit_knot_emits_config_event() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, mut strand_rx, mut config_rx) = create_source_fresh();

        // Create the knot file before watching
        let knot_content = "---
name: existing-knot
agent-config:
  goal: \"Initial goal\"
  provider: \"openai\"
  model: \"gpt-4o\"
strand-dir: \"strands\"
tie-off-dir: \"tie-offs\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Initial instructions\"
---

# Existing Knot
";
        let knot_path = loom_dir.path().join("existing-knot.md");
        fs::write(&knot_path, knot_content).unwrap();

        // Register loom watch
        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id.clone()),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Drain any events from file creation before watch
        while recv_config_event(&mut config_rx, Duration::from_millis(50))
            .is_some()
        {}

        // Edit the knot file (change model)
        let updated_content = "---
name: existing-knot
agent-config:
  goal: \"Updated goal\"
  provider: \"anthropic\"
  model: \"claude-sonnet\"
strand-dir: \"strands\"
tie-off-dir: \"tie-offs\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Updated instructions\"
---

# Existing Knot (Updated)
";
        fs::write(&knot_path, updated_content).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "loom watch should not emit strand events on edit"
        );

        // Config channel: should receive KnotModified
        let event =
            recv_config_event(&mut config_rx, Duration::from_millis(500))
                .expect("should receive a KnotModified config event");
        match event {
            ConfigEvent::KnotModified {
                loom_id: lid,
                knot,
            } => {
                assert_eq!(lid.0, "test-loom");
                assert_eq!(knot.id.0, "existing-knot");
                assert_eq!(knot.agent_config.model, "claude-sonnet");
            }
            other => panic!(
                "Expected ConfigEvent::KnotModified, got: {:?}",
                other
            ),
        }
    }

    /// Watch loom directory with `WatchType::Loom(id)`; delete a `.md`
    /// knot file → `ConfigEvent::KnotDeleted` emitted on config channel.
    #[test]
    fn loom_dir_delete_knot_emits_config_event() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, mut strand_rx, mut config_rx) = create_source_fresh();

        // Create the knot file before watching
        let knot_content = "---
name: to-delete
agent-config:
  goal: \"Goal\"
  provider: \"openai\"
  model: \"gpt-4o\"
strand-dir: \"strands\"
tie-off-dir: \"tie-offs\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Instructions\"
---

# To Delete
";
        let knot_path = loom_dir.path().join("to-delete.md");
        fs::write(&knot_path, knot_content).unwrap();

        // Register loom watch
        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id.clone()),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Drain any events from file creation before watch
        while recv_config_event(&mut config_rx, Duration::from_millis(50))
            .is_some()
        {}

        // Delete the knot file
        fs::remove_file(&knot_path).unwrap();

        thread::sleep(POLL_DELAY);

        // Strand channel: no events
        assert!(
            recv_strand_event(&mut strand_rx, Duration::from_millis(200))
                .is_none(),
            "loom watch should not emit strand events on delete"
        );

        // Config channel: should receive KnotDeleted
        let event =
            recv_config_event(&mut config_rx, Duration::from_millis(500))
                .expect("should receive a KnotDeleted config event");
        match event {
            ConfigEvent::KnotDeleted {
                loom_id: lid,
                knot_id,
            } => {
                assert_eq!(lid.0, "test-loom");
                assert_eq!(knot_id.0, "to-delete");
            }
            other => panic!(
                "Expected ConfigEvent::KnotDeleted, got: {:?}",
                other
            ),
        }
    }

    /// Non-`-loom` directory creation in rig watch is ignored.
    #[test]
    fn rig_dir_non_loom_directory_ignored() {
        let rig_dir = TempDir::new().unwrap();
        let (source, _strand_rx, mut config_rx) = create_source_fresh();

        source.register_watch(
            rig_dir.path().to_path_buf(),
            WatchType::Rig,
        );
        source.watch(rig_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a non-loom directory
        let subdir = rig_dir.path().join("random-dir");
        fs::create_dir(&subdir).unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "non-loom directory should not emit events"
        );
    }

    /// Non-`.md` file creation in loom watch is ignored.
    #[test]
    fn loom_dir_non_md_file_ignored() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, _strand_rx, mut config_rx) = create_source_fresh();

        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a non-md file
        let file_path = loom_dir.path().join("config.json");
        fs::write(&file_path, "{}").unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "non-.md file should not emit config events"
        );
    }

    /// Directory creation in loom watch is ignored.
    #[test]
    fn loom_dir_subdirectory_creation_ignored() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, _strand_rx, mut config_rx) = create_source_fresh();

        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a subdirectory
        let subdir = loom_dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "subdirectory creation in loom watch should not emit events"
        );
    }

    /// Malformed knot file in loom watch is silently skipped.
    #[test]
    fn loom_dir_malformed_knot_ignored() {
        let loom_dir = TempDir::new().unwrap();
        let loom_id = LoomId("test-loom".to_string());
        let (source, _strand_rx, mut config_rx) = create_source_fresh();

        source.register_watch(
            loom_dir.path().to_path_buf(),
            WatchType::Loom(loom_id),
        );
        source.watch(loom_dir.path()).unwrap();
        thread::sleep(Duration::from_millis(100));

        // Create a file with invalid frontmatter
        let bad_content = "# Not a valid knot file

Just some markdown.
";
        let bad_path = loom_dir.path().join("bad-knot.md");
        fs::write(&bad_path, bad_content).unwrap();

        thread::sleep(POLL_DELAY);

        assert!(
            recv_config_event(&mut config_rx, Duration::from_millis(200))
                .is_none(),
            "malformed knot file should not emit events"
        );
    }
}
