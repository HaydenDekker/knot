//! `UnregisterLoom` use case — unregister a loom.

use std::collections::HashSet;
use std::sync::Arc;

use crate::adapters::logging;
use crate::application::ports::{EventSource, LoomLogPort, PortError};
use crate::application::store::LoomStore;
use crate::domain::entities::{LoomId};
use crate::domain::events::LoomEvent;

use super::super::types::format_timestamp;

// ── UnregisterLoom ─────────────────────────────────────────────────────────

/// Use case: unregister a loom.
///
/// 1. Looks up the loom in `LoomStore`
/// 2. Calls `EventSource::unwatch()` for each effective source directory
/// 3. Appends `LoomStopped` event via `LoomLogPort::append()`
/// 4. Removes the loom from `LoomStore`
///
/// Returns an error if the loom was not found.
pub struct UnregisterLoom {
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
}

impl UnregisterLoom {
    /// Create a new `UnregisterLoom` use case.
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

    /// Unregister the loom with the given ID.
    ///
    /// Returns `PortError::LoomNotFound` if the loom is not in the store.
    pub fn execute(&self, id: &LoomId) -> Result<(), PortError> {
        // Check loom exists before any side effects
        let loom = self.store.get(id)
            .ok_or_else(|| PortError::LoomNotFound(id.clone()))?;

        // Stop watching strand directories for each knot
        for knot in &loom.knots {
            self.event_source.unwatch(&knot.strand_dir)
                .map_err(|e| {
                    PortError::EventUnwatchFailed(format!(
                        "failed to unwatch '{}': {}",
                        knot.strand_dir.display(),
                        e
                    ))
                })?;
        }

        // Append LoomStopped event
        self.log_port.append(LoomEvent::LoomStopped {
            loom_id: id.clone(),
            timestamp: format_timestamp(),
        })?;

        // Remove from store
        self.store.unregister(id);

        logging::log_loom_event(
            "unregistered",
            &id.0,
            &format!("{} watchers stopped", loom.knots.len()),
        );
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod unregister_tests {
    use super::*;

    use super::super::super::test_fixtures::{
        build_knot, build_loom, MockLoomLogPort, TrackingEventSource,
    };

    /// `UnregisterLoom` with mock `EventSource`: after unregistration,
    /// `unwatch()` is called for each knot's strand directory.
    #[test]
    fn unregister_loom_stops_watchers() {
        let loom = build_loom(
            "unwatch-loom",
            vec![
                build_knot("k1"),
                build_knot("k2"),
            ],
        );
        let loom_id = loom.id.clone();

        let (event_source, _watch_calls, unwatch_calls, _) =
            TrackingEventSource::new();
        let store = LoomStore::new();

        // Register the loom first (so it exists in the store)
        store.register(loom);

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = UnregisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(&loom_id);

        // Should succeed
        assert!(result.is_ok());

        // unwatch() called for each knot's strand directory
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 2);

        // Both knots use their strand_dir ("strands")
        let unwatched: HashSet<_> =
            unwatches.iter().map(|p| p.as_path()).collect();
        assert!(unwatched.contains(std::path::Path::new("strands")));

        // Loom is no longer in the store
        assert!(store.get(&loom_id).is_none());
    }

    /// `UnregisterLoom` with no knots unregisters without unwatch.
    #[test]
    fn unregister_loom_stops_watcher_empty_knots() {
        let loom = build_loom("empty-unwatch-loom", vec![]);
        let loom_id = loom.id.clone();

        let (event_source, _watch_calls, unwatch_calls, _) =
            TrackingEventSource::new();
        let store = LoomStore::new();

        // Register the loom first
        store.register(loom);

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = UnregisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(&loom_id);

        assert!(result.is_ok());

        // No knots means no unwatch
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 0);

        assert!(store.get(&loom_id).is_none());
    }

    /// `UnregisterLoom` for unknown loom returns error without calling
    /// `unwatch()`.
    #[test]
    fn unregister_loom_not_found_no_unwatch() {
        let (event_source, _watch_calls, unwatch_calls, _) =
            TrackingEventSource::new();
        let store = LoomStore::new();

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = UnregisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result =
            use_case.execute(&LoomId("nonexistent".to_string()));

        // Should fail with LoomNotFound
        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("nonexistent".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }

        // No unwatch calls
        let unwatches = unwatch_calls.lock().unwrap();
        assert!(unwatches.is_empty());
    }
}
