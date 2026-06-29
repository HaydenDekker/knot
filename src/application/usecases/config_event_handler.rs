//! Use case: handle configuration events for looms and knots.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adapters::outbound::event_source::WatchType;
use crate::adapters::logging;
use crate::application::ports::{
    EventSource, LoomLogPort, LoomRepository, PortError,
};
use crate::application::store::LoomStore;
use crate::domain::entities::{Knot, KnotId, Loom, LoomId};
use crate::domain::events::{ConfigEvent, LoomEvent};

// Re-export shared types from types module
use super::types::format_timestamp;

// ── ConfigEventHandler ───────────────────────────────────────────────

/// Use case: handle configuration events for looms and knots.
///
/// Receives `ConfigEvent`s from the file watcher (via outbound adapter)
/// and updates the in-memory `LoomStore`, starts/stops watchers, and
/// writes loom-log entries.
///
/// - `ConfigEvent::LoomAdded` — scan the loom directory via
///   `LoomRepository::scan()` and register the loom (same flow as
///   `RegisterLoom`).
/// - `ConfigEvent::KnotAdded` — add the knot to the loom in the store,
///   log `KnotRegistered`, start watcher for `strand_dir`.
/// - `ConfigEvent::KnotModified` — update the knot in the store, stop
///   old watcher, start new watcher if `strand_dir` changed.
/// - `ConfigEvent::KnotDeleted` — remove the knot from the loom in the
///   store, stop watcher, log `KnotDeregistered`.
pub struct ConfigEventHandler {
    repository: Arc<dyn LoomRepository>,
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
    /// Project root (parent of rig_dir), used to resolve relative paths.
    project_root: PathBuf,
}

impl ConfigEventHandler {
    /// Create a new `ConfigEventHandler`.
    pub fn new(
        repository: Arc<dyn LoomRepository>,
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
        event_source: Arc<dyn EventSource>,
        rig_path: PathBuf,
    ) -> Self {
        // Derive project root from rig_path parent (same as FileSystemLoomRepository::scan).
        // Falls back to rig_path itself if no parent exists.
        let project_root = rig_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| rig_path.clone());
        Self {
            repository,
            log_port,
            store,
            event_source,
            project_root,
        }
    }

    /// Handle a single configuration event.
    pub fn execute(&self, event: ConfigEvent) -> Result<(), PortError> {
        match event {
            ConfigEvent::LoomAdded {
                ref loom_id,
                ref loom_dir,
            } => {
                logging::log_config_event(
                    "LoomAdded",
                    &format!("loom={} dir={}", loom_id.0, loom_dir),
                );
                self.handle_loom_added(loom_id, Path::new(loom_dir))
            }
            ConfigEvent::KnotAdded { ref loom_id, ref knot } => {
                logging::log_config_event(
                    "KnotAdded",
                    &format!("loom={} knot={}", loom_id.0, knot.id.0),
                );
                self.handle_knot_added(loom_id, knot.clone())
            }
            ConfigEvent::KnotModified { ref loom_id, ref knot } => {
                logging::log_config_event(
                    "KnotModified",
                    &format!("loom={} knot={}", loom_id.0, knot.id.0),
                );
                self.handle_knot_modified(loom_id, knot.clone())
            }
            ConfigEvent::KnotDeleted { ref loom_id, ref knot_id } => {
                logging::log_config_event(
                    "KnotDeleted",
                    &format!("loom={} knot={}", loom_id.0, knot_id.0),
                );
                self.handle_knot_deleted(loom_id, knot_id)
            }
        }
    }

    /// Handle `ConfigEvent::LoomAdded`.
    ///
    /// Scans only the new loom directory via `LoomRepository::scan_knot_files()`
    /// (not the full rig), resolves knot paths relative to the project root,
    /// builds a `Loom` directly, and registers it using the same flow as
    /// `RegisterLoom`.
    fn handle_loom_added(
        &self,
        loom_id: &LoomId,
        loom_dir: &Path,
    ) -> Result<(), PortError> {
        // Skip if already registered
        if self.store.get(loom_id).is_some() {
            logging::log_config_event(
                "LoomAdded",
                &format!("loom={} already registered (skip)", loom_id.0),
            );
            return Ok(());
        }

        // Scan only the new loom directory (not the full rig)
        let (mut knots, warnings) =
            self.repository.scan_knot_files(loom_dir)?;

        // Resolve per-knot paths relative to the project root
        for knot in &mut knots {
            knot.strand_dir =
                crate::adapters::outbound::loom_repository::FileSystemLoomRepository::resolve_path(
                    &self.project_root,
                    &knot.strand_dir,
                );
        }

        let loom = Loom {
            id: loom_id.clone(),
            knots,
        };

        self.register_loom(&loom, &warnings)?;
        logging::log_loom_event(
            "registered",
            &loom_id.0,
            &format!("{} knots", loom.knots.len()),
        );
        Ok(())
    }

    /// Handle `ConfigEvent::KnotAdded`.
    ///
    /// Adds the knot to the loom in the store, logs `KnotRegistered`,
    /// and starts a watcher for its `strand_dir`.
    fn handle_knot_added(
        &self,
        loom_id: &LoomId,
        knot: Knot,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        // Check for duplicate knot ID
        if loom.knots.iter().any(|k| k.id == knot.id) {
            // Idempotent — knot already present, skip
            logging::log_knot_event(
                "added",
                &loom_id.0,
                &knot.id.0,
                "already present (skip)",
            );
            return Ok(());
        }

        let knot_strand_dir = knot.strand_dir.clone();
        let knot_id = knot.id.clone();
        loom.knots.push(knot);
        self.store.register(loom);

        // Log KnotRegistered
        self.log_port.append(LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            timestamp: format_timestamp(),
        })?;

        // Start watcher (auto-creates strand_dir if missing)
        super::loom::ensure_strand_dir_and_watch(
            loom_id,
            &knot_id,
            &knot_strand_dir,
            &*self.log_port,
            &*self.event_source,
        )?;

        logging::log_knot_event(
            "added",
            &loom_id.0,
            &knot_id.0,
            "registered + watcher started",
        );
        Ok(())
    }

    /// Handle `ConfigEvent::KnotModified`.
    ///
    /// Updates the knot in the store, stops the old watcher if
    /// `strand_dir` changed, and starts a new watcher for the
    /// updated `strand_dir`.
    ///
    /// If the knot is not found in the loom (e.g., due to a race between
    /// `LoomAdded` scanning a partially-written directory and a later
    /// `KnotModified` with valid data), treats it as a new registration:
    /// appends the knot, logs `KnotRegistered`, starts a watcher, and
    /// emits a warning.
    fn handle_knot_modified(
        &self,
        loom_id: &LoomId,
        knot: Knot,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        let pos = loom.knots.iter().position(|k| k.id == knot.id);

        match pos {
            Some(index) => {
                // Existing knot — update in place
                let old_strand_dir = loom.knots[index].strand_dir.clone();
                let new_strand_dir = knot.strand_dir.clone();
                let knot_id = knot.id.clone();
                loom.knots[index] = knot;
                self.store.register(loom);

                // If strand_dir changed, stop old watcher and start new one
                if old_strand_dir != new_strand_dir {
                    self.event_source.unwatch_with_type(
                        &old_strand_dir,
                        WatchType::Strand(loom_id.clone(), knot_id.clone()),
                    )
                    .map_err(|e| {
                        PortError::EventUnwatchFailed(format!(
                            "failed to unwatch '{}': {}",
                            old_strand_dir.display(),
                            e
                        ))
                    })?;

                    super::loom::ensure_strand_dir_and_watch(
                        loom_id,
                        &knot_id,
                        &new_strand_dir,
                        &*self.log_port,
                        &*self.event_source,
                    )?;

                    logging::log_knot_event(
                        "modified",
                        &loom_id.0,
                        &knot_id.0,
                        "strand_dir changed, watcher updated",
                    );
                } else {
                    logging::log_knot_event(
                        "modified",
                        &loom_id.0,
                        &knot_id.0,
                        "config updated (strand_dir unchanged)",
                    );
                }
            }
            None => {
                // Knot not found — recover by registering as new.
                // This handles the race where LoomAdded scanned before
                // the knot file was fully written.
                let knot_strand_dir = knot.strand_dir.clone();
                let knot_id = knot.id.clone();
                loom.knots.push(knot);
                self.store.register(loom);

                logging::log_knot_event(
                    "warn:modified",  
                    &loom_id.0,
                    &knot_id.0,
                    "knot not found, recovered by registering",
                );

                // Log KnotRegistered
                self.log_port.append(LoomEvent::KnotRegistered {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    timestamp: format_timestamp(),
                })?;

                // Start watcher (auto-creates strand_dir if missing)
                super::loom::ensure_strand_dir_and_watch(
                    loom_id,
                    &knot_id,
                    &knot_strand_dir,
                    &*self.log_port,
                    &*self.event_source,
                )?;

                logging::log_knot_event(
                    "added",
                    &loom_id.0,
                    &knot_id.0,
                    "registered + watcher started (recovered from KnotModified)",
                );
            }
        }

        Ok(())
    }

    /// Handle `ConfigEvent::KnotDeleted`.
    ///
    /// Removes the knot from the loom in the store, stops its
    /// `strand_dir` watcher, and logs `KnotDeregistered`.
    fn handle_knot_deleted(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) -> Result<(), PortError> {
        let loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        let pos = loom.knots.iter()
            .position(|k| k.id == *knot_id)
            .ok_or_else(|| PortError::LoomSaveFailed(format!(
                "knot '{}' not found in loom '{}'",
                knot_id.0,
                loom_id.0
            )))?;

        let knot = &loom.knots[pos];
        let strand_dir = knot.strand_dir.clone();

        // Remove knot from loom
        let mut updated_loom = loom;
        updated_loom.knots.remove(pos);
        self.store.register(updated_loom);

        // Stop watcher for the knot's strand directory
        self.event_source.unwatch_with_type(
            &strand_dir,
            WatchType::Strand(loom_id.clone(), knot_id.clone()),
        )
        .map_err(|e| {
            PortError::EventUnwatchFailed(format!(
                "failed to unwatch '{}': {}",
                strand_dir.display(),
                e
            ))
        })?;

        // Log KnotDeregistered
        self.log_port.append(LoomEvent::KnotDeregistered {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            timestamp: format_timestamp(),
        })?;

        logging::log_knot_event(
            "deleted",
            &loom_id.0,
            &knot_id.0,
            "removed + watcher stopped",
        );
        Ok(())
    }

    /// Register a loom: log events, store, and start watchers.
    fn register_loom(
        &self,
        loom: &Loom,
        warnings: &[String],
    ) -> Result<(), PortError> {
        // Open the loom activity log
        self.log_port.open(&loom.id)?;

        // Append KnotRegistered for each knot
        for knot in &loom.knots {
            self.log_port.append(LoomEvent::KnotRegistered {
                loom_id: loom.id.clone(),
                knot_id: knot.id.clone(),
                timestamp: format_timestamp(),
            })?;
        }

        // Append KnotParseWarning for each unknown property warning
        for warning in warnings {
            self.log_port.append(LoomEvent::KnotParseWarning {
                loom_id: loom.id.clone(),
                knot_file_name: String::new(),
                message: warning.clone(),
                timestamp: format_timestamp(),
            })?;
        }

        // Append LoomStarted event
        self.log_port.append(LoomEvent::LoomStarted {
            loom_id: loom.id.clone(),
            timestamp: format_timestamp(),
        })?;

        // Store the loom
        self.store.register(loom.clone());

        // Start file watchers for each knot's strand directory
        for knot in &loom.knots {
            super::loom::ensure_strand_dir_and_watch(
                &loom.id,
                &knot.id,
                &knot.strand_dir,
                &*self.log_port,
                &*self.event_source,
            )?;
        }

        Ok(())
    }
}

// ── ConfigEventHandler Tests ──────────────────────────────────────────

#[cfg(test)]
mod config_handler_tests {
    use super::*;
    use crate::domain::value_objects::PromptTemplate;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    #[allow(unused_imports)]
    use super::super::test_fixtures::{
        build_knot, build_knot_with_strand_dir, build_loom,
        MockLoomLogPort, MockLoomRepository, TrackingEventSource,
    };

    // ── Tests ──────────────────────────────────────────────────────────

    /// `ConfigEventHandler` with `ConfigEvent::LoomAdded`: scans the
    /// loom dir via repository `scan_knot_files()`, registers loom in
    /// store, logs events, and starts watchers for each knot's strand
    /// directory.
    #[test]
    fn config_handler_loom_added() {
        let loom_id = LoomId("new-loom".to_string());
        // Use a temp directory so strand_dirs can be auto-created
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().to_path_buf();
        let strand_dir = rig_path.join("strands");
        let knots = vec![
            build_knot_with_strand_dir("k1", strand_dir.clone()),
            build_knot_with_strand_dir("k2", strand_dir.clone()),
        ];

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(knots)),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let store = LoomStore::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            rig_path.clone(),
        );

        let result = handler.execute(ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: rig_path.join("new-loom").display().to_string(),
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom is in the store
        let stored = store.get(&loom_id);
        assert!(stored.is_some(), "loom should be in store");
        let stored = stored.unwrap();
        assert_eq!(stored.id, loom_id);
        assert_eq!(stored.knots.len(), 2);

        // Log events: 2x KnotRegistered, LoomStarted, DirectoryCreated
        let events = logged_events.lock().unwrap();
        assert_eq!(
            events.len(),
            4,
            "should log KnotRegistered x2 + LoomStarted + DirectoryCreated"
        );
        match &events[0] {
            LoomEvent::KnotRegistered { loom_id: lid, knot_id, .. } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k1");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }
        match &events[1] {
            LoomEvent::KnotRegistered { loom_id: lid, knot_id, .. } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k2");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }
        match &events[2] {
            LoomEvent::LoomStarted { loom_id: lid, .. } => {
                assert_eq!(*lid, loom_id);
            }
            other => panic!("Expected LoomStarted, got {other:?}"),
        }
        // DirectoryCreated logged for first knot (second shares same dir)
        match &events[3] {
            LoomEvent::DirectoryCreated {
                loom_id: lid,
                knot_id,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k1");
            }
            other => panic!("Expected DirectoryCreated, got {other:?}"),
        }

        // Watchers started for each knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(
            watches.len(),
            2,
            "should watch 2 knot strand directories"
        );
        let watched: HashSet<_> =
            watches.iter().map(|p| p.as_path()).collect();
        assert!(
            watched.contains(strand_dir.as_path()),
            "watcher should be started for strand_dir"
        );
    }

    /// `ConfigEventHandler` with `ConfigEvent::LoomAdded` for an
    /// already-registered loom is idempotent (no-op).
    #[test]
    fn config_handler_loom_added_already_registered() {
        let loom = build_loom("existing-loom", vec![build_knot("k1")]);
        let loom_id = loom.id.clone();

        let store = LoomStore::new();
        store.register(loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: "/rig/existing-loom".to_string(),
        });

        // Should succeed (idempotent)
        assert!(result.is_ok(), "should succeed for existing loom");

        // No watchers started
        let watches = watch_calls.lock().unwrap();
        assert!(
            watches.is_empty(),
            "no watchers should be started for existing loom"
        );
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotAdded`: adds the
    /// knot to the loom in the store, logs `KnotRegistered`, and
    /// starts a watcher for the knot's strand directory.
    #[test]
    fn config_handler_knot_added() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_loom =
            build_loom("test-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let new_knot = build_knot("k2");
        let result = handler.execute(ConfigEvent::KnotAdded {
            loom_id: loom_id.clone(),
            knot: new_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom now has 2 knots
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 2);
        let k2 = loom.knots.iter()
            .find(|k| k.id == KnotId("k2".to_string()))
            .unwrap();
        assert_eq!(k2.id, KnotId("k2".to_string()));

        // Log: KnotRegistered for k2 (may also have DirectoryCreated
        // if strand_dir was auto-created, depending on CWD state)
        let events = logged_events.lock().unwrap();
        assert!(events.len() >= 1);
        match &events[0] {
            LoomEvent::KnotRegistered {
                loom_id: lid,
                knot_id,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k2");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }

        // Watcher started for new knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotAdded` for a
    /// duplicate knot is idempotent (no-op).
    #[test]
    fn config_handler_knot_added_duplicate() {
        let loom_id = LoomId("test-loom".to_string());
        let knot = build_knot("k1");
        let existing_loom =
            build_loom("test-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotAdded {
            loom_id: loom_id.clone(),
            knot,
        });

        // Should succeed (idempotent)
        assert!(result.is_ok(), "should succeed for duplicate knot");

        // Loom still has 1 knot
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 1);

        // No new log entries
        let events = logged_events.lock().unwrap();
        assert!(events.is_empty());

        // No new watchers
        let watches = watch_calls.lock().unwrap();
        assert!(watches.is_empty());
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotAdded` returns
    /// error when loom does not exist.
    #[test]
    fn config_handler_knot_added_loom_not_found() {
        let store = LoomStore::new();
        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, _, _, _) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotAdded {
            loom_id: LoomId("nonexistent".to_string()),
            knot: build_knot("k1"),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("nonexistent".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified`: updates
    /// the knot in the store, stops the old watcher, and starts a
    /// new watcher if `strand_dir` changed.
    #[test]
    fn config_handler_knot_modified() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_knot = build_knot("k1");
        let existing_loom =
            build_loom("test-loom", vec![existing_knot]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            _watch_calls,
            unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        // Update knot with different strand_dir
        let updated_knot =
            build_knot_with_strand_dir("k1", PathBuf::from("new-strands"));

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: updated_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Knot is updated in store
        let loom = store.get(&loom_id).unwrap();
        let k1 = loom.knots.iter()
            .find(|k| k.id == KnotId("k1".to_string()))
            .unwrap();
        assert_eq!(
            k1.strand_dir,
            PathBuf::from("new-strands")
        );

        // Old watcher stopped
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 1);
        assert_eq!(unwatches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` with
    /// same `strand_dir`: updates knot without watcher changes.
    #[test]
    fn config_handler_knot_modified_same_strand_dir() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_knot = build_knot("k1");
        let existing_loom =
            build_loom("test-loom", vec![existing_knot]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _) = MockLoomLogPort::new();
        let (
            event_source,
            _watch_calls,
            unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        // Update knot with same strand_dir (only profile ref changed)
        let mut updated_knot = build_knot("k1");
        updated_knot.agent_profile_ref = "slow".to_string();

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: updated_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // No watcher changes
        let unwatches = unwatch_calls.lock().unwrap();
        assert!(
            unwatches.is_empty(),
            "no unwatch when strand_dir unchanged"
        );
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` recovers
    /// by registering the knot when it does not exist in the loom.
    /// This handles the race where LoomAdded scanned before the knot
    /// file was fully written, resulting in 0 knots registered.
    #[test]
    fn config_handler_knot_modified_not_found() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_loom =
            build_loom("test-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: build_knot("k_new"),
        });

        // Should succeed — recovers by registering the knot
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom now has 2 knots (k1 + recovered k_new)
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 2);
        let ids: Vec<_> = loom.knots.iter().map(|k| k.id.0.as_str()).collect();
        assert!(ids.contains(&"k1"));
        assert!(ids.contains(&"k_new"));

        // Log: KnotRegistered for recovered knot
        let events = logged_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::KnotRegistered {
                loom_id: lid,
                knot_id,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k_new");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }

        // Watcher started for recovered knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` for a
    /// loom with 0 knots: recovers by registering the knot, logs
    /// `KnotRegistered`, and starts a watcher.
    #[test]
    fn config_handler_knot_modified_new_knot_registers() {
        let loom_id = LoomId("empty-loom".to_string());
        // Loom registered with 0 knots (simulates race condition)
        let empty_loom = build_loom("empty-loom", vec![]);

        let store = LoomStore::new();
        store.register(empty_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let new_knot = build_knot("k1");
        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: new_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom now has 1 knot
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 1);
        assert_eq!(loom.knots[0].id, KnotId("k1".to_string()));

        // Log: KnotRegistered for the knot
        let events = logged_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::KnotRegistered {
                loom_id: lid,
                knot_id,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k1");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }

        // Watcher started for knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` for a
    /// missing knot emits a warning log and recovers. The warning is
    /// verified by checking that the recovery path side-effects
    /// (`KnotRegistered` log event, watcher started) are present.
    #[test]
    fn config_handler_knot_modified_warns_on_recovery() {
        let loom_id = LoomId("warn-loom".to_string());
        let empty_loom = build_loom("warn-loom", vec![]);

        let store = LoomStore::new();
        store.register(empty_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: build_knot("k_warn"),
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Verify the warning was emitted by checking the log events:
        // the recovery path logs KnotRegistered, proving the recovery
        // branch was taken (which also emits the warning via eprintln)
        let events = logged_events.lock().unwrap();
        let has_registered = events.iter().any(|e| matches!(
            e,
            LoomEvent::KnotRegistered { .. }
        ));
        assert!(
            has_registered,
            "recovery path should emit KnotRegistered (warning logged to stderr)"
        );

        // Watcher started confirms full recovery path executed
        let watches = watch_calls.lock().unwrap();
        assert_eq!(
            watches.len(),
            1,
            "recovery path should start a watcher"
        );
        assert_eq!(watches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotDeleted`: removes
    /// the knot from the loom in the store, stops its watcher, and
    /// logs `KnotDeregistered`.
    #[test]
    fn config_handler_knot_deleted() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_loom = build_loom(
            "test-loom",
            vec![
                build_knot("k1"),
                build_knot("k2"),
                build_knot("k3"),
            ],
        );

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            _watch_calls,
            unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let knot_id = KnotId("k2".to_string());
        let result = handler.execute(ConfigEvent::KnotDeleted {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Knot removed from loom
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 2);
        let ids: Vec<_> = loom.knots.iter()
            .map(|k| k.id.0.as_str())
            .collect();
        assert!(ids.contains(&"k1"));
        assert!(ids.contains(&"k3"));
        assert!(!ids.contains(&"k2"));

        // Watcher stopped for the deleted knot's strand directory
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 1);
        assert_eq!(unwatches[0], PathBuf::from("strands"));

        // Log: KnotDeregistered
        let events = logged_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::KnotDeregistered {
                loom_id: lid,
                knot_id: kid,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
            }
            other => panic!("Expected KnotDeregistered, got {other:?}"),
        }
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotDeleted` returns
    /// error when knot does not exist in the loom.
    #[test]
    fn config_handler_knot_deleted_not_found() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_loom =
            build_loom("test-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, _, _, _) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotDeleted {
            loom_id: loom_id.clone(),
            knot_id: KnotId("k_unknown".to_string()),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomSaveFailed(msg) => {
                assert!(msg.contains("not found"));
            }
            other => panic!("Expected LoomSaveFailed, got {other:?}"),
        }
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotDeleted` returns
    /// error when loom does not exist.
    #[test]
    fn config_handler_knot_deleted_loom_not_found() {
        let store = LoomStore::new();
        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, _, _, _) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotDeleted {
            loom_id: LoomId("nonexistent".to_string()),
            knot_id: KnotId("k1".to_string()),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("nonexistent".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotAdded` when
    /// `strand_dir` does not exist: auto-creates the directory,
    /// logs `DirectoryCreated`, and starts the watcher.
    #[test]
    fn config_handler_knot_added_missing_strand_dir() {
        let loom_id = LoomId("auto-dir-loom".to_string());
        let existing_loom =
            build_loom("auto-dir-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        // Create a temp dir for the nonexistent strand_dir
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent_dir = tmp.path().join("nonexistent-strands");
        assert!(
            !nonexistent_dir.exists(),
            "strand_dir must not exist before test"
        );

        let new_knot = build_knot_with_strand_dir("k2", nonexistent_dir.clone());
        let result = handler.execute(ConfigEvent::KnotAdded {
            loom_id: loom_id.clone(),
            knot: new_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // strand_dir was auto-created
        assert!(
            nonexistent_dir.exists(),
            "strand_dir should have been auto-created"
        );

        // Log contains KnotRegistered + DirectoryCreated
        let events = logged_events.lock().unwrap();
        let event_names: Vec<_> = events.iter().map(|e| match e {
            LoomEvent::KnotRegistered { .. } => "KnotRegistered",
            LoomEvent::DirectoryCreated { .. } => "DirectoryCreated",
            other => panic!("unexpected event variant: {:?}", other),
        }).collect();
        assert!(
            event_names.contains(&"KnotRegistered"),
            "should log KnotRegistered"
        );
        assert!(
            event_names.contains(&"DirectoryCreated"),
            "should log DirectoryCreated"
        );

        // Verify DirectoryCreated event fields
        let dir_created_event = events.iter().find(|e| {
            matches!(e, LoomEvent::DirectoryCreated { .. })
        });
        assert!(
            dir_created_event.is_some(),
            "DirectoryCreated event should be present"
        );
        match dir_created_event.unwrap() {
            LoomEvent::DirectoryCreated {
                loom_id: lid,
                knot_id: kid,
                directory: dir,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(kid.0, "k2");
                assert_eq!(
                    dir.as_str(),
                    nonexistent_dir.display().to_string(),
                    "DirectoryCreated should record the path"
                );
            }
            _ => unreachable!(),
        }

        // Watcher was started
        let watches = watch_calls.lock().unwrap();
        assert_eq!(
            watches.len(),
            1,
            "watcher should be started after dir creation"
        );
        assert_eq!(watches[0], nonexistent_dir);

        // Knot is in store
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 2);
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` when the
    /// new `strand_dir` does not exist: auto-creates the directory,
    /// logs `DirectoryCreated`, stops the old watcher, and starts a
    /// new watcher.
    #[test]
    fn config_handler_knot_modified_missing_strand_dir() {
        let loom_id = LoomId("auto-mod-loom".to_string());
        let existing_knot = build_knot("k1");
        let existing_loom =
            build_loom("auto-mod-loom", vec![existing_knot]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        // Create a temp dir for the nonexistent strand_dir
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent_dir = tmp.path().join("new-strands");
        assert!(
            !nonexistent_dir.exists(),
            "new strand_dir must not exist before test"
        );

        // Update knot to point to the nonexistent directory
        let updated_knot =
            build_knot_with_strand_dir("k1", nonexistent_dir.clone());

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: updated_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // strand_dir was auto-created
        assert!(
            nonexistent_dir.exists(),
            "new strand_dir should have been auto-created"
        );

        // Log contains DirectoryCreated
        let events = logged_events.lock().unwrap();
        let dir_created_event = events.iter().find(|e| {
            matches!(e, LoomEvent::DirectoryCreated { .. })
        });
        assert!(
            dir_created_event.is_some(),
            "DirectoryCreated event should be present"
        );
        match dir_created_event.unwrap() {
            LoomEvent::DirectoryCreated {
                loom_id: lid,
                knot_id: kid,
                directory: dir,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(kid.0, "k1");
                assert_eq!(
                    dir.as_str(),
                    nonexistent_dir.display().to_string(),
                    "DirectoryCreated should record the new path"
                );
            }
            _ => unreachable!(),
        }

        // Old watcher stopped
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 1);
        assert_eq!(unwatches[0], PathBuf::from("strands"));

        // New watcher started on the created directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], nonexistent_dir);

        // Knot is updated in store
        let loom = store.get(&loom_id).unwrap();
        let k1 = loom.knots.iter()
            .find(|k| k.id == KnotId("k1".to_string()))
            .unwrap();
        assert_eq!(k1.strand_dir, nonexistent_dir);
    }
}

// ── Phase 2 Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod phase2_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::PromptTemplate;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[allow(unused_imports)]
    use super::super::test_fixtures::{
        build_knot, build_knot_with_strand_dir, build_loom,
        MockLoomLogPort, TrackingEventSource,
    };

    // ── Mock LoomRepository for ConfigEventHandler tests ───────────────

    struct MockLoomRepository {
        scan_knots: Arc<Mutex<Vec<Knot>>>,
        scan_error: Arc<Mutex<Option<String>>>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<(Vec<Loom>, Vec<String>), PortError> {
            Ok((vec![], vec![]))
        }

        fn scan_knot_files(
            &self,
            _loom_dir: &std::path::Path,
        ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
            if let Some(ref err) = *self.scan_error.lock().unwrap() {
                return Err(PortError::RigScanFailed(err.clone()));
            }
            Ok((self.scan_knots.lock().unwrap().clone(), vec![]))
        }

        fn get(
            &self,
            _id: &LoomId,
        ) -> Result<Option<Loom>, PortError> {
            Ok(None)
        }

        fn list(&self) -> Result<Vec<Loom>, PortError> {
            Ok(vec![])
        }

        fn save(&self, _loom: Loom) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── ConfigEventHandler: targeted loom scan tests ───────────────────

    /// `handle_loom_added` uses `scan_knot_files(loom_dir)` to scan only
    /// the new loom directory (not the full rig), then resolves paths and
    /// builds a `Loom` directly.
    #[test]
    fn config_handler_loom_added_scans_specific_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().to_path_buf();
        let rig_path = workspace.join("rig");
        fs::create_dir_all(&rig_path).unwrap();
        let strand_dir = workspace.join("strands");

        let loom_id = LoomId("new-loom".to_string());
        let knots = vec![
            build_knot_with_strand_dir("k1", strand_dir.clone()),
            build_knot_with_strand_dir("k2", strand_dir.clone()),
        ];

        let repo = Arc::new(MockLoomRepository {
            scan_knots: Arc::new(Mutex::new(knots)),
            scan_error: Arc::new(Mutex::new(None)),
        });
        let store = LoomStore::new();
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            Arc::new(event_source),
            rig_path.clone(),
        );

        let result = handler.execute(ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: rig_path.join("new-loom").display().to_string(),
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom is in the store with correct knots
        let stored = store.get(&loom_id).unwrap();
        assert_eq!(stored.id, loom_id);
        assert_eq!(stored.knots.len(), 2);
        let knot_ids: Vec<_> =
            stored.knots.iter().map(|k| k.id.0.as_str()).collect();
        assert!(knot_ids.contains(&"k1"));
        assert!(knot_ids.contains(&"k2"));

        // Watchers started for each knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 2);
        let watched: HashSet<_> =
            watches.iter().map(|p| p.as_path()).collect();
        assert!(
            watched.contains(strand_dir.as_path()),
            "expected strand_dir in watches, got {:?}",
            watches
        );
    }

    /// `handle_loom_added` returns an error when the loom directory
    /// scan fails (e.g. directory does not exist).
    #[test]
    fn config_handler_loom_added_dir_missing() {
        let loom_id = LoomId("missing-loom".to_string());

        let repo = Arc::new(MockLoomRepository {
            scan_knots: Arc::new(Mutex::new(vec![])),
            scan_error: Arc::new(Mutex::new(Some(
                "No such file or directory".to_string(),
            ))),
        });
        let store = LoomStore::new();
        let (event_source, _watch_calls, _, _) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/workspace/rig"),
        );

        let result = handler.execute(ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: "/workspace/rig/missing-loom".to_string(),
        });

        // Should fail with RigScanFailed
        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::RigScanFailed(msg) => {
                assert!(
                    msg.contains("No such file or directory"),
                    "expected scan error, got: {}",
                    msg
                );
            }
            other => {
                panic!("Expected RigScanFailed, got {other:?}");
            }
        }

        // Loom should NOT be in the store
        assert!(
            store.get(&loom_id).is_none(),
            "loom should not be registered after scan failure"
        );
    }
}

