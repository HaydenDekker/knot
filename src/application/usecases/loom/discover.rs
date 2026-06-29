//! `DiscoverLooms` use case — discover looms in a workspace.

use std::path::Path;
use std::sync::Arc;

use crate::adapters::logging;
use crate::application::ports::{EventSource, LoomLogPort, LoomRepository, PortError};
use crate::application::store::LoomStore;
use crate::domain::entities::{Loom, LoomId};
use crate::domain::events::LoomEvent;

use super::super::types::format_timestamp;

// ── DiscoverLooms ──────────────────────────────────────────────────────────

/// Use case: discover looms in a workspace and register any new ones.
///
/// Calls `LoomRepository::scan()` to find looms, then for each loom
/// not already in `LoomStore`:
/// - Opens the loom activity log via `LoomLogPort::open()`
/// - Appends `KnotRegistered` to the loom log via `LoomLogPort::append()`
/// - Appends `LoomStarted` to the loom log via `LoomLogPort::append()`
/// - Registers the loom in `LoomStore`
/// - Starts file watchers via `EventSource::watch()`
///
/// Looms already present in the store are skipped — no duplicate
/// registration or watcher starts. Returns only the newly discovered
/// (previously unknown) loom IDs.
pub struct DiscoverLooms {
    repository: Arc<dyn LoomRepository>,
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
}

impl DiscoverLooms {
    /// Create a new `DiscoverLooms` use case.
    pub fn new(
        repository: Arc<dyn LoomRepository>,
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
        event_source: Arc<dyn EventSource>,
    ) -> Self {
        Self {
            repository,
            log_port,
            store,
            event_source,
        }
    }

    /// Execute discovery against the given workspace path.
    ///
    /// Returns the list of *newly* discovered looms (those not already
    /// in the store). Already-registered looms are silently skipped.
    pub fn execute(&self, workspace: &Path) -> Result<Vec<Loom>, PortError> {
        let (looms, warnings) = self.repository.scan(workspace)?;
        let mut new_looms = Vec::new();

        for loom in &looms {
            // Skip looms already registered in the store
            if self.store.get(&loom.id).is_some() {
                continue;
            }

            logging::log_loom_event(
                "discover",
                &loom.id.0,
                &format!("new loom found, {} knots", loom.knots.len()),
            );
            self.register_single(loom, &warnings)?;
            new_looms.push(loom.clone());
        }

        Ok(new_looms)
    }

    /// Register a single loom: log events, store, and start watchers.
    fn register_single(
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
            super::ensure_strand_dir_and_watch(
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

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod discover_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use super::super::super::test_fixtures::{
        build_knot, build_loom, MockLoomLogPort, MockLoomRepository,
        TrackingEventSource,
    };

    /// `DiscoverLooms` use case given looms where one ID already in store
    /// → only new looms are registered (log entries + watchers), existing
    /// ones skipped.
    #[test]
    fn discover_looms_runtime_skips_registered() {
        let existing_loom = build_loom("existing", vec![build_knot("k1")]);
        let new_loom = build_loom("new-loom", vec![build_knot("k2")]);
        let new_loom2 = build_loom("new-loom2", vec![]); // no knots

        // Pre-register one loom in the store
        let store = LoomStore::new();
        store.register(existing_loom.clone());

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![
                existing_loom.clone(),
                new_loom.clone(),
                new_loom2.clone(),
            ])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = DiscoverLooms::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(Path::new("/workspace"));

        // Should succeed
        assert!(result.is_ok());

        // Only the 2 new looms returned (existing one skipped)
        let discovered = result.unwrap();
        assert_eq!(discovered.len(), 2);

        let ids: Vec<_> = discovered.iter().map(|l| l.id.0.as_str()).collect();
        assert!(ids.contains(&"new-loom"));
        assert!(ids.contains(&"new-loom2"));
        assert!(!ids.contains(&"existing"));

        // Watchers started only for new looms (not existing)
        let watches = watch_calls.lock().unwrap();
        // new_loom has 1 knot → 1 watch for "strands"
        // new_loom2 has 0 knots → no watch
        assert_eq!(watches.len(), 1);

        // Both new looms are in store
        assert!(store.get(&LoomId("new-loom".to_string())).is_some());
        assert!(store.get(&LoomId("new-loom2".to_string())).is_some());
    }

    /// `DiscoverLooms` with all looms already registered returns empty.
    #[test]
    fn discover_looms_all_registered_returns_empty() {
        let loom1 = build_loom("loom-a", vec![build_knot("k1")]);
        let loom2 = build_loom("loom-b", vec![build_knot("k2")]);

        let store = LoomStore::new();
        store.register(loom1.clone());
        store.register(loom2.clone());

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![loom1.clone(), loom2.clone()])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = DiscoverLooms::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_ok());

        // No new looms discovered
        let discovered = result.unwrap();
        assert!(discovered.is_empty());

        // No watchers started
        let watches = watch_calls.lock().unwrap();
        assert!(watches.is_empty());
    }

    /// `DiscoverLooms` with empty scan returns empty (no side effects).
    #[test]
    fn discover_looms_empty_scan() {
        let store = LoomStore::new();

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = DiscoverLooms::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
        let watches = watch_calls.lock().unwrap();
        assert!(watches.is_empty());
    }
}
