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

use super::super::system_event_emitter::{
    EventScope, SystemEventEmitter,
};
use super::super::types::format_timestamp;

/// Emit a knot-scoped `DirectoryCreated` system event (plan 082),
/// best-effort — no-op without an emitter.
fn emit_directory_created(
    emitter: Option<&SystemEventEmitter>,
    loom_id: &LoomId,
    knot_id: &KnotId,
    directory: &str,
) {
    let Some(emitter) = emitter else { return };
    let mut payload = std::collections::HashMap::new();
    payload.insert("directory".to_string(), directory.to_string());
    let scope =
        EventScope::knot_no_strand(loom_id.clone(), knot_id.clone());
    let _ = emitter.emit(
        &scope,
        "DirectoryCreated",
        payload,
        Some(format!(
            "Strand directory '{}' auto-created (knot '{}')",
            directory, knot_id.0
        )),
    );
}

/// Ensure a knot's strand source directory exists on disk, then start
/// the file watcher.
///
/// Handles both strand source variants:
///
/// - **Filesystem**: creates the path if missing, starts a watcher.
/// - **EventUri**: **no directory and no watcher** (plan 099) — the
///   event is delivered straight into the consumer's queue, so the
///   dispatched-file machinery is retired.
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
    emitter: Option<&SystemEventEmitter>,
) -> Result<(), PortError> {
    match strand_source {
        StrandSource::Filesystem(path) => {
            ensure_filesystem_watch(
                loom_id,
                knot_id,
                path,
                log_port,
                event_source,
                emitter,
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
                emitter,
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
    emitter: Option<&SystemEventEmitter>,
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
        emit_directory_created(
            emitter,
            loom_id,
            knot_id,
            &strand_dir.display().to_string(),
        );
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
/// Derives the event dispatch directory from the rig's runtime root:
/// `tie-offs/<rig-basename>/{loom-id}/{event-id}/`. Creates it if missing
/// and starts a file watcher so dispatched event files trigger
/// `StrandEvent::Created` for the consumer knot.
/// Plan 099: an `EventUri` strand source needs **no directory and no
/// watcher**.
///
/// Delivery is a direct write into the consumer's queue
/// (`QueueEventDispatcher`), so the dispatched-file machinery this
/// function used to set up is retired:
///
/// - the `tie-offs/<rig>/<loom>/<EventId>/` dispatch directory is no
///   longer created,
/// - no filesystem watcher is registered for it,
/// - no `DirectoryCreated` record is emitted for it.
///
/// The subscription itself (`StrandSource::EventUri`) is unchanged —
/// only the delivery mechanism moved off the filesystem. The knot is
/// still recorded as event-driven so the producer-side matching
/// (`resolve_for_producer`) and the read-side views keep working.
fn ensure_event_uri_watch(
    _rig_dir: &Path,
    loom_id: &LoomId,
    knot_id: &KnotId,
    event_id: &str,
    _log_port: &dyn LoomLogPort,
    _event_source: &dyn EventSource,
    _emitter: Option<&SystemEventEmitter>,
) -> Result<(), PortError> {
    logging::log_knot_event(
        "watch-skipped",
        &loom_id.0,
        &knot_id.0,
        &format!(
            "event subscription {event_id} needs no dispatch dir — \
             plan 099 delivers straight to the queue"
        ),
    );
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
            None,
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
            None,
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

    // ── EventUri tests (plan 099: the dispatched-file mechanism is retired) ──

    /// Plan 099: an `EventUri` strand source creates **no** dispatch
    /// directory, registers **no** watcher, and calls **no**
    /// `set_loom_ids` — delivery is a direct queue write, so the
    /// filesystem machinery this replaced must stay untouched.
    #[test]
    fn event_uri_source_creates_no_directory_and_watches_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().join("rig");

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
            None,
        );
        assert!(result.is_ok(), "an event subscription must still register");

        // The retired dispatch directory must NOT be created.
        let retired_dir = tmp
            .path()
            .join("tie-offs")
            .join("rig")
            .join(&loom_id.0)
            .join(event_id);
        assert!(
            !retired_dir.exists(),
            "plan 099 must not create the dispatch dir {}",
            retired_dir.display()
        );
        assert!(
            !tmp.path().join("tie-offs").exists(),
            "plan 099 must not create the tie-offs runtime tree for a subscription"
        );

        // No watcher, no id mapping — nothing to watch.
        assert!(watch_calls.lock().unwrap().is_empty(), "no watcher for an event source");
        assert!(set_ids_calls.lock().unwrap().is_empty(), "no set_loom_ids for an event source");

        // No DirectoryCreated record.
        let events = logged_events.lock().unwrap();
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, LoomEvent::DirectoryCreated { .. })),
            "no DirectoryCreated for a retired mechanism"
        );
    }

    /// Plan 099: a pre-existing legacy dispatch directory (left by an
    /// older binary) is left alone and is **not** watched — the queue is
    /// the delivery mechanism, so a stale file dropped there no longer
    /// drives the consumer.
    #[test]
    fn event_uri_source_ignores_a_legacy_dispatch_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().join("rig");

        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("test-knot".to_string());

        let legacy_dir = tmp
            .path()
            .join("tie-offs")
            .join("rig")
            .join(&loom_id.0)
            .join("PreEvent");
        std::fs::create_dir_all(&legacy_dir).unwrap();

        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();

        let event_uri = StrandSource::EventUri {
            producer_knot: "producer".to_string(),
            event_id: "PreEvent".to_string(),
        };

        ensure_strand_source_watch(
            &rig_path,
            &loom_id,
            &knot_id,
            &event_uri,
            &log_port,
            &event_source,
            None,
        )
        .unwrap();

        assert!(
            watch_calls.lock().unwrap().is_empty(),
            "a legacy dispatch directory must not be watched under plan 099"
        );
        assert!(legacy_dir.exists(), "the legacy directory is left in place");
    }

    /// Plan 099: the retirement is scoped to event sources — a
    /// filesystem strand source still gets its directory and watcher.
    #[test]
    fn filesystem_source_still_creates_and_watches_after_the_retirement() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().join("rig");

        let loom_id = LoomId("work-loom".to_string());
        let knot_id = KnotId("worker".to_string());
        let strand_dir = tmp.path().join("strands");

        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();

        ensure_strand_source_watch(
            &rig_path,
            &loom_id,
            &knot_id,
            &StrandSource::Filesystem(strand_dir.clone()),
            &log_port,
            &event_source,
            None,
        )
        .unwrap();

        assert!(strand_dir.exists(), "filesystem strand dir is still created");
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], strand_dir);
    }
}
