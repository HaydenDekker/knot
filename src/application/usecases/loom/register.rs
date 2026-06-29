//! `RegisterLoom` use case — register a single loom.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::adapters::logging;
use crate::application::ports::{EventSource, LoomLogPort, PortError};
use crate::application::store::LoomStore;
use crate::domain::entities::{KnotId, Loom, LoomId};
use crate::domain::events::LoomEvent;

use super::super::types::format_timestamp;

// ── RegisterLoom ───────────────────────────────────────────────────────────

/// Use case: register a single loom.
///
/// 1. Opens the loom activity log via `LoomLogPort::open()`
/// 2. Appends `KnotRegistered` for each knot via `LoomLogPort::append()`
/// 3. Appends `LoomStarted` event via `LoomLogPort::append()`
/// 4. Stores the loom in `LoomStore`
/// 5. Starts file watchers for each knot's effective source directory
///
/// Returns an error if a loom with the same ID already exists.
pub struct RegisterLoom {
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
}

impl RegisterLoom {
    /// Create a new `RegisterLoom` use case.
    pub fn new(
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
        event_source: Arc<dyn EventSource>,
    ) -> Self {
        Self {
            log_port,
            store,
            event_source,
        }
    }

    /// Register the given loom.
    ///
    /// Idempotent — if the loom is already registered (e.g., by the
    /// ConfigEventHandler auto-discovering the loom directory), this is
    /// a no-op. Returns `PortError::LoomSaveFailed` only for write errors.
    pub fn execute(&self, loom: Loom) -> Result<(), PortError> {
        // If already registered (e.g., auto-discovered), skip — idempotent
        if self.store.get(&loom.id).is_some() {
            logging::log_loom_event(
                "register",
                &loom.id.0,
                "already registered (idempotent, skip)",
            );
            return Ok(());
        }

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

        // Append LoomStarted event
        self.log_port.append(LoomEvent::LoomStarted {
            loom_id: loom.id.clone(),
            timestamp: format_timestamp(),
        })?;

        // Store the loom
        self.store.register(loom.clone());

        // Start file watchers for each knot's strand directory
        for knot in &loom.knots {
            super::ensure_strand_dir_and_watch(
                &loom.id,
                &knot.id,
                &knot.strand_dir,
                &*self.log_port,
                &*self.event_source,
            )?;
        }

        logging::log_loom_event(
            "registered",
            &loom.id.0,
            &format!("{} knots, watchers started", loom.knots.len()),
        );
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod register_tests {
    use super::*;

    use super::super::super::test_fixtures::{
        build_knot, build_loom, MockLoomLogPort, TrackingEventSource,
    };

    /// `RegisterLoom` with mock `EventSource`: after registration,
    /// `watch()` is called for each knot's strand directory.
    #[test]
    fn register_loom_starts_watchers() {
        let loom = build_loom(
            "watch-loom",
            vec![
                build_knot("k1"),
                build_knot("k2"),
            ],
        );
        let loom_id = loom.id.clone();

        let (event_source, watch_calls, _, _) = TrackingEventSource::new();
        let store = LoomStore::new();

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(loom);

        // Should succeed
        assert!(result.is_ok());

        // watch() called for each knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 2);

        // Both knots use their strand_dir ("strands")
        let watched: HashSet<_> =
            watches.iter().map(|p| p.as_path()).collect();
        assert!(watched.contains(Path::new("strands")));

        // Loom is in the store
        assert!(store.get(&loom_id).is_some());
    }

    /// `RegisterLoom` with no knots registers the loom without watchers.
    #[test]
    fn register_loom_starts_watcher_empty_knots() {
        let loom = build_loom("empty-loom", vec![]);
        let loom_id = loom.id.clone();

        let (event_source, watch_calls, _, _) = TrackingEventSource::new();
        let store = LoomStore::new();

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(loom);

        assert!(result.is_ok());

        // No knots means no watches
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 0);

        assert!(store.get(&loom_id).is_some());
    }

    /// `RegisterLoom` duplicate ID is idempotent — returns Ok without starting
    /// new watchers or modifying the existing entry. This is necessary because
    /// auto-discovery may pre-register a loom before the POST /looms arrives.
    #[test]
    fn register_loom_duplicate_is_idempotent() {
        let loom1 = build_loom("dup", vec![build_knot("k1")]);
        let loom2 = build_loom("dup", vec![build_knot("k2")]);

        let (event_source, watch_calls, _, _) = TrackingEventSource::new();
        let store = LoomStore::new();

        // Register first loom
        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            Arc::clone(&es),
        );
        assert!(use_case.execute(loom1).is_ok());

        // Verify first registration started a watcher
        {
            let watches = watch_calls.lock().unwrap();
            assert_eq!(watches.len(), 1);
        }

        // Duplicate registration — should succeed (idempotent)
        let (event_source2, watch_calls2, _, _) = TrackingEventSource::new();
        let es2: Arc<dyn EventSource> = Arc::new(event_source2);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es2,
        );
        let result = use_case.execute(loom2);

        // Idempotent: returns Ok(()) instead of Err
        assert!(result.is_ok());

        // No new watchers started for the duplicate
        let watches = watch_calls2.lock().unwrap();
        assert!(watches.is_empty());

        // Original store unchanged (k2 was not added)
        let stored = store.get(&LoomId("dup".to_string())).unwrap();
        assert_eq!(stored.knots.len(), 1);
        assert_eq!(stored.knots[0].id, KnotId("k1".to_string()));
    }
}
