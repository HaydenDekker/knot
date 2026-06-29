//! `ReloadConfig` use case — re-scan the rig and register new looms.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::ports::{EventSource, LoomLogPort, LoomRepository, PortError};
use crate::application::store::LoomStore;
use crate::domain::entities::{Knot, Loom, LoomId};

use super::discover::DiscoverLooms;

// ── ReloadConfig ───────────────────────────────────────────────────────────

/// Use case: re-scan the rig and register any looms not already in the store.
///
/// Provides the business logic for the manual reload endpoint
/// (`POST /config/reload`). Delegates to `DiscoverLooms` to avoid
/// duplicating discovery and registration logic.
///
/// Returns the list of *newly* registered loom IDs (those not already
/// in the store). Already-registered looms are silently skipped.
pub struct ReloadConfig {
    repository: Arc<dyn LoomRepository>,
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
    rig_dir: PathBuf,
}

impl ReloadConfig {
    /// Create a new `ReloadConfig` use case.
    pub fn new(
        repository: Arc<dyn LoomRepository>,
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
        event_source: Arc<dyn EventSource>,
        rig_dir: PathBuf,
    ) -> Self {
        Self {
            repository,
            log_port,
            store,
            event_source,
            rig_dir,
        }
    }

    /// Re-scan the rig and register any looms not already in the store.
    ///
    /// Returns the list of newly registered loom IDs.
    pub fn execute(&self) -> Result<Vec<LoomId>, PortError> {
        let discover = DiscoverLooms::new(
            self.repository.clone(),
            self.log_port.clone(),
            self.store.clone(),
            self.event_source.clone(),
        );
        let new_looms = discover.execute(&self.rig_dir)?;
        Ok(new_looms.into_iter().map(|l| l.id).collect())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod reload_config_tests {
    use super::*;

    use super::super::super::test_fixtures::{
        build_knot, build_loom, MockLoomLogPort, TrackingEventSource,
    };

    // ── Mock LoomRepository (simplified for ReloadConfig tests) ───────

    struct MockLoomRepository {
        scan_looms: Vec<Loom>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<(Vec<Loom>, Vec<String>), PortError> {
            Ok((self.scan_looms.clone(), vec![]))
        }

        fn scan_knot_files(
            &self,
            _loom_dir: &std::path::Path,
        ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
            Ok((vec![], vec![]))
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

    // ── Tests ──────────────────────────────────────────────────────────

    /// `ReloadConfig` re-scans the rig and registers any looms not
    /// already in the store. Returns the IDs of newly registered looms.
    #[test]
    fn reload_config_discovers_new_looms() {
        let new_loom = build_loom("new-loom", vec![build_knot("k1")]);
        let new_loom2 = build_loom("new-loom2", vec![]);

        let store = LoomStore::new();
        let repo = Arc::new(MockLoomRepository {
            scan_looms: vec![new_loom, new_loom2],
        });
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = ReloadConfig::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
            PathBuf::from("/workspace/rig"),
        );

        let result = use_case.execute();

        // Should succeed and return 2 new loom IDs
        assert!(result.is_ok());
        let ids = result.unwrap();
        assert_eq!(ids.len(), 2);
        let id_set: HashSet<_> = ids.iter().map(|id| id.0.as_str()).collect();
        assert!(id_set.contains("new-loom"));
        assert!(id_set.contains("new-loom2"));

        // Both looms are in the store
        assert!(store.get(&LoomId("new-loom".to_string())).is_some());
        assert!(store.get(&LoomId("new-loom2".to_string())).is_some());

        // Watchers started for new looms (1 knot in new-loom, 0 in new-loom2)
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
    }

    /// `ReloadConfig` skips looms that are already in the store.
    /// Only new looms are registered and returned.
    #[test]
    fn reload_config_skips_registered() {
        let existing_loom = build_loom("existing", vec![build_knot("k1")]);
        let new_loom = build_loom("new-loom", vec![build_knot("k2")]);

        // Pre-register one loom
        let store = LoomStore::new();
        store.register(existing_loom.clone());

        let repo = Arc::new(MockLoomRepository {
            scan_looms: vec![existing_loom, new_loom],
        });
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = ReloadConfig::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
            PathBuf::from("/workspace/rig"),
        );

        let result = use_case.execute();

        // Should succeed and return only 1 new loom ID
        assert!(result.is_ok());
        let ids = result.unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], LoomId("new-loom".to_string()));

        // Existing loom is still in the store (unchanged)
        assert!(store.get(&LoomId("existing".to_string())).is_some());
        // New loom is now in the store
        assert!(store.get(&LoomId("new-loom".to_string())).is_some());

        // Watchers started only for the new loom
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
    }

    /// `ReloadConfig` when all looms are already registered returns
    /// empty vector (no side effects).
    #[test]
    fn reload_config_all_registered_returns_empty() {
        let loom1 = build_loom("loom-a", vec![build_knot("k1")]);
        let loom2 = build_loom("loom-b", vec![build_knot("k2")]);

        let store = LoomStore::new();
        store.register(loom1.clone());
        store.register(loom2.clone());

        let repo = Arc::new(MockLoomRepository {
            scan_looms: vec![loom1, loom2],
        });
        let (event_source, watch_calls, _, _) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = ReloadConfig::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
            PathBuf::from("/workspace/rig"),
        );

        let result = use_case.execute();

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());

        // No watchers started
        let watches = watch_calls.lock().unwrap();
        assert!(watches.is_empty());
    }
}
