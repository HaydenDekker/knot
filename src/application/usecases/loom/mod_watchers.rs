//! Shared watcher helper for loom use cases.
//!
//! Provides a single unified `ensure_strand_source_watch()` function that
//! handles both `StrandSource` variants — filesystem paths and event URIs.

use std::path::Path;

use crate::adapters::logging;
use crate::application::ports::{EventSource, LoomLogPort, PortError};
use crate::domain::entities::{KnotId, LoomId};
use crate::domain::events::LoomEvent;
use crate::domain::value_objects::StrandSource;

use super::super::types::format_timestamp;

/// Ensure a knot's strand source directory exists on disk, then start
/// the file watcher.
///
/// Handles both strand source variants:
///
/// - **Filesystem**: creates the path if missing, starts a watcher.
/// - **EventUri**: derives the event dispatch directory
///   `{rig_dir}/tie-offs/{loom-id}/{event-id}/`, creates it if missing,
///   and starts a watcher so dispatched event files trigger
///   `StrandEvent::Created` for the consumer knot.
///
/// Logs `LoomEvent::DirectoryCreated` when a directory is auto-created,
/// and always logs a `watch-started` event after the watcher is
/// registered.
pub(crate) fn ensure_strand_source_watch(
    rig_dir: &Path,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_source: &StrandSource,
    log_port: &dyn LoomLogPort,
    event_source: &dyn EventSource,
) -> Result<(), PortError> {
    match strand_source {
        StrandSource::Filesystem(path) => {
            ensure_filesystem_watch(
                loom_id,
                knot_id,
                path,
                log_port,
                event_source,
            )
        }
        StrandSource::EventUri { event_id, .. } => {
            ensure_event_uri_watch(
                rig_dir,
                loom_id,
                knot_id,
                event_id,
                log_port,
                event_source,
            )
        }
    }
}

/// Ensure a filesystem strand source directory exists and is watched.
///
/// If the directory is missing, creates it (including any parent
/// directories), logs a `LoomEvent::DirectoryCreated` event, and
/// emits a log line. The watcher is always started regardless of
/// whether creation was needed.
fn ensure_filesystem_watch(
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_dir: &Path,
    log_port: &dyn LoomLogPort,
    event_source: &dyn EventSource,
) -> Result<(), PortError> {
    let dir_created = if !strand_dir.exists() {
        std::fs::create_dir_all(strand_dir).map_err(|e| {
            PortError::LoomSaveFailed(format!(
                "failed to create strand dir '{}': {}",
                strand_dir.display(),
                e,
            ))
        })?;
        log_port.append(LoomEvent::DirectoryCreated {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            directory: strand_dir.display().to_string(),
            timestamp: format_timestamp(),
        })?;
        logging::log_knot_event(
            "dir-created",
            &loom_id.0,
            &knot_id.0,
            &format!("auto-created strand dir: {}", strand_dir.display()),
        );
        true
    } else {
        false
    };

    event_source.set_loom_ids(strand_dir, loom_id, knot_id);
    event_source.watch(strand_dir).map_err(|e| {
        PortError::LoomSaveFailed(format!(
            "failed to watch '{}': {}",
            strand_dir.display(),
            e,
        ))
    })?;

    if dir_created {
        logging::log_knot_event(
            "watch-started",
            &loom_id.0,
            &knot_id.0,
            "watcher started on newly created dir",
        );
    }

    Ok(())
}

/// Ensure an event URI strand source directory exists and is watched.
///
/// Derives the event dispatch directory from the rig directory:
/// `{rig_dir}/tie-offs/{loom-id}/{event-id}/`. Creates it if missing
/// and starts a file watcher so dispatched event files trigger
/// `StrandEvent::Created` for the consumer knot.
fn ensure_event_uri_watch(
    rig_dir: &Path,
    loom_id: &LoomId,
    knot_id: &KnotId,
    event_id: &str,
    log_port: &dyn LoomLogPort,
    event_source: &dyn EventSource,
) -> Result<(), PortError> {
    let event_dir = rig_dir.join("tie-offs").join(&loom_id.0).join(event_id);

    let dir_created = if !event_dir.exists() {
        std::fs::create_dir_all(&event_dir).map_err(|e| {
            PortError::LoomSaveFailed(format!(
                "failed to create event dir '{}': {}",
                event_dir.display(),
                e,
            ))
        })?;
        log_port.append(LoomEvent::DirectoryCreated {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            directory: event_dir.display().to_string(),
            timestamp: format_timestamp(),
        })?;
        logging::log_knot_event(
            "dir-created",
            &loom_id.0,
            &knot_id.0,
            &format!("auto-created event dir: {} (event={})", event_dir.display(), event_id),
        );
        true
    } else {
        false
    };

    event_source.set_loom_ids(&event_dir, loom_id, knot_id);
    event_source.watch(&event_dir).map_err(|e| {
        PortError::LoomSaveFailed(format!(
            "failed to watch event dir '{}': {}",
            event_dir.display(),
            e,
        ))
    })?;

    if dir_created {
        logging::log_knot_event(
            "watch-started",
            &loom_id.0,
            &knot_id.0,
            &format!("event watcher started on {} (event={})", event_dir.display(), event_id),
        );
    }

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod mod_watchers_tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use crate::application::ports::{EventSource, LoomLogPort, PortError};
    use crate::application::usecases::test_fixtures::{
        MockLoomLogPort, TrackingEventSource,
    };
    use crate::domain::entities::LoomId;
    use crate::domain::events::LoomEvent;
    use crate::domain::value_objects::StrandSource;

    use super::*;

    // ── Filesystem tests ───────────────────────────────────────────────

    /// `ensure_strand_source_watch` with `StrandSource::Filesystem`:
    /// when the directory does not exist, it is created (including
    /// parent directories), `DirectoryCreated` is logged, and the
    /// watcher is started.
    #[test]
    fn filesystem_source_creates_and_watches_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().to_path_buf();
        let non_existent = rig_path.join("subdir").join("strands");
        assert!(!non_existent.exists());

        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("test-knot".to_string());

        let (log_port, logged_events) = MockLoomLogPort::new();
        let (event_source, watch_calls, _, set_ids_calls) =
            TrackingEventSource::new();

        let result = ensure_strand_source_watch(
            &rig_path,
            &loom_id,
            &knot_id,
            &StrandSource::Filesystem(non_existent.clone()),
            &log_port,
            &event_source,
        );

        // Should succeed
        assert!(result.is_ok());

        // Directory was created
        assert!(non_existent.exists());

        // DirectoryCreated event logged
        let events = logged_events.lock().unwrap();
        assert!(
            events.iter().any(|e| matches!(e, LoomEvent::DirectoryCreated { .. })),
            "DirectoryCreated should be logged"
        );

        // Watcher started
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], non_existent);

        // set_loom_ids called
        let set_ids = set_ids_calls.lock().unwrap();
        assert_eq!(set_ids.len(), 1);
        assert_eq!(set_ids[0].0, non_existent);
        assert_eq!(set_ids[0].1, loom_id);
        assert_eq!(set_ids[0].2, knot_id);
    }

    /// `ensure_strand_source_watch` with `StrandSource::Filesystem`:
    /// when the directory already exists, no creation event is logged,
    /// but the watcher is still started.
    #[test]
    fn filesystem_source_existing_directory_starts_watcher() {
        let tmp = tempfile::tempdir().unwrap();
        let existing = tmp.path().join("existing-strands");
        std::fs::create_dir_all(&existing).unwrap();

        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("test-knot".to_string());

        let (log_port, logged_events) = MockLoomLogPort::new();
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();

        let result = ensure_strand_source_watch(
            tmp.path(),
            &loom_id,
            &knot_id,
            &StrandSource::Filesystem(existing.clone()),
            &log_port,
            &event_source,
        );

        assert!(result.is_ok());

        // No DirectoryCreated event (dir already existed)
        let events = logged_events.lock().unwrap();
        let has_dir_created = events.iter().any(|e| {
            matches!(e, LoomEvent::DirectoryCreated { .. })
        });
        assert!(
            !has_dir_created,
            "no DirectoryCreated when dir already exists"
        );

        // Watcher still started
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
    }

    // ── EventUri tests ─────────────────────────────────────────────────

    /// `ensure_strand_source_watch` with `StrandSource::EventUri`:
    /// derives the event dispatch directory
    /// `{rig_dir}/tie-offs/{loom-id}/{event-id}/`, creates it if missing,
    /// and starts the watcher.
    #[test]
    fn event_uri_source_derives_and_creates_event_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().to_path_buf();

        let loom_id = LoomId("validation-loom".to_string());
        let knot_id = KnotId("validator-knot".to_string());
        let event_id = "NonConformance";

        let (log_port, logged_events) = MockLoomLogPort::new();
        let (event_source, watch_calls, _, set_ids_calls) =
            TrackingEventSource::new();

        let event_uri = StrandSource::EventUri {
            producer_knot: "plan-creator".to_string(),
            event_id: event_id.to_string(),
        };

        let result = ensure_strand_source_watch(
            &rig_path,
            &loom_id,
            &knot_id,
            &event_uri,
            &log_port,
            &event_source,
        );

        // Should succeed
        assert!(result.is_ok());

        // Event directory was created
        let expected_dir = rig_path
            .join("tie-offs")
            .join(&loom_id.0)
            .join(event_id);
        assert!(
            expected_dir.exists(),
            "event directory should have been created"
        );

        // DirectoryCreated event logged
        let events = logged_events.lock().unwrap();
        assert!(
            events.iter().any(|e| matches!(e, LoomEvent::DirectoryCreated { .. })),
            "DirectoryCreated should be logged for event dir"
        );

        // Verify DirectoryCreated event fields
        let dir_created_event = events.iter().find(|e| {
            matches!(e, LoomEvent::DirectoryCreated { .. })
        });
        assert!(dir_created_event.is_some());
        match dir_created_event.unwrap() {
            LoomEvent::DirectoryCreated {
                directory,
                knot_id: event_knot_id,
                ..
            } => {
                assert_eq!(
                    directory.as_str(),
                    expected_dir.display().to_string()
                );
                assert_eq!(event_knot_id.0, knot_id.0);
            }
            other => panic!("expected DirectoryCreated, got {other:?}"),
        }

        // Watcher started on event directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], expected_dir);

        // set_loom_ids called with event directory
        let set_ids = set_ids_calls.lock().unwrap();
        assert_eq!(set_ids.len(), 1);
        assert_eq!(set_ids[0].0, expected_dir);
        assert_eq!(set_ids[0].1, loom_id);
        assert_eq!(set_ids[0].2, knot_id);
    }

    /// `ensure_strand_source_watch` with `StrandSource::EventUri`:
    /// the event dispatch directory is watched (verified via
    /// TrackingEventSource).
    #[test]
    fn event_uri_source_watches_derived_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().to_path_buf();

        let loom_id = LoomId("event-loom".to_string());
        let knot_id = KnotId("event-knot".to_string());

        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();

        let event_uri = StrandSource::EventUri {
            producer_knot: "producer".to_string(),
            event_id: "TestEvent".to_string(),
        };

        ensure_strand_source_watch(
            &rig_path,
            &loom_id,
            &knot_id,
            &event_uri,
            &log_port,
            &event_source,
        )
        .unwrap();

        // Watcher started on the derived event directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);

        let expected = rig_path
            .join("tie-offs")
            .join(&loom_id.0)
            .join("TestEvent");
        assert_eq!(watches[0], expected);
    }

    /// `ensure_strand_source_watch` with `StrandSource::EventUri`:
    /// when the event directory already exists, no creation event is
    /// logged, but the watcher is still started.
    #[test]
    fn event_uri_source_existing_directory_starts_watcher() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().to_path_buf();

        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("test-knot".to_string());

        // Pre-create the event directory
        let event_dir = rig_path
            .join("tie-offs")
            .join(&loom_id.0)
            .join("PreEvent");
        std::fs::create_dir_all(&event_dir).unwrap();

        let (log_port, logged_events) = MockLoomLogPort::new();
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();

        let event_uri = StrandSource::EventUri {
            producer_knot: "producer".to_string(),
            event_id: "PreEvent".to_string(),
        };

        let result = ensure_strand_source_watch(
            &rig_path,
            &loom_id,
            &knot_id,
            &event_uri,
            &log_port,
            &event_source,
        );

        assert!(result.is_ok());

        // No DirectoryCreated event
        let events = logged_events.lock().unwrap();
        assert!(
            events.iter().all(|e| !matches!(e, LoomEvent::DirectoryCreated { .. })),
            "no DirectoryCreated when event dir already exists"
        );

        // Watcher still started
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], event_dir);
    }

    /// `ensure_strand_source_watch` with `StrandSource::EventUri`:
    /// missing `rig/tie-offs/` parent directories are created
    /// automatically.
    #[test]
    fn event_uri_creates_missing_rig_tieoffs_parents() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().to_path_buf();
        // Do NOT create rig/tie-offs — it should be created automatically.
        assert!(!rig_path.join("tie-offs").exists());

        let loom_id = LoomId("parent-loom".to_string());
        let knot_id = KnotId("parent-knot".to_string());

        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();

        let event_uri = StrandSource::EventUri {
            producer_knot: "producer".to_string(),
            event_id: "ParentTest".to_string(),
        };

        ensure_strand_source_watch(
            &rig_path,
            &loom_id,
            &knot_id,
            &event_uri,
            &log_port,
            &event_source,
        )
        .unwrap();

        // Parent directories were created
        let expected = rig_path
            .join("tie-offs")
            .join(&loom_id.0)
            .join("ParentTest");
        assert!(
            expected.exists(),
            "event dir with parents should exist"
        );
        assert!(
            rig_path.join("tie-offs").exists(),
            "tie-offs parent should exist"
        );

        // Watcher started on the final directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], expected);
    }
}
