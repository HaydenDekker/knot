//! Application-layer use cases.
//!
//! Each use case orchestrates domain entities through port traits and the
//! in-memory loom store. Tests use mock port implementations — no IO.

use std::path::Path;

use crate::application::ports::{KnotStatePort, LoomLogPort, LoomRepository, PortError};
use crate::application::store::LoomStore;
use crate::domain::entities::Loom;
use crate::domain::events::LoomEvent;

// ── DiscoverLooms ──────────────────────────────────────────────────────────

/// Use case: discover looms in a workspace and register them.
///
/// Calls `LoomRepository::scan()` to find looms, then for each loom:
/// - Creates knot state via `KnotStatePort::create()`
/// - Appends `KnotRegistered` to the loom log via `LoomLogPort::append()`
/// - Registers the loom in `LoomStore`
pub struct DiscoverLooms {
    repository: Box<dyn LoomRepository>,
    state_port: Box<dyn KnotStatePort>,
    log_port: Box<dyn LoomLogPort>,
    store: LoomStore,
}

impl DiscoverLooms {
    /// Create a new `DiscoverLooms` use case.
    pub fn new(
        repository: Box<dyn LoomRepository>,
        state_port: Box<dyn KnotStatePort>,
        log_port: Box<dyn LoomLogPort>,
        store: LoomStore,
    ) -> Self {
        Self {
            repository,
            state_port,
            log_port,
            store,
        }
    }

    /// Execute discovery against the given workspace path.
    ///
    /// Returns the list of discovered looms.
    pub fn execute(&self, workspace: &Path) -> Result<Vec<Loom>, PortError> {
        let looms = self.repository.scan(workspace)?;

        for loom in &looms {
            self.register_knots(loom)?;
            self.store.register(loom.clone());
        }

        Ok(looms)
    }

    /// Create state and log events for every knot in a loom.
    fn register_knots(&self, loom: &Loom) -> Result<(), PortError> {
        for knot in &loom.knots {
            self.state_port.create(&knot.id)?;
            self.log_port.append(LoomEvent::KnotRegistered {
                loom_id: loom.id.clone(),
                knot_id: knot.id.clone(),
            })?;
        }
        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId, LoomId};
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
    use std::collections::HashSet;
    use std::path::PathBuf;

    // ── Mock Implementations ────────────────────────────────────────────

    /// Mock `LoomRepository` with configurable scan results.
    struct MockLoomRepository {
        scan_result: Result<Vec<Loom>, PortError>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(&self, _workspace: &Path) -> Result<Vec<Loom>, PortError> {
            self.scan_result.clone()
        }

        fn get(&self, _id: &LoomId) -> Result<Option<Loom>, PortError> {
            Ok(None)
        }

        fn list(&self) -> Result<Vec<Loom>, PortError> {
            Ok(vec![])
        }

        fn save(&self, _loom: Loom) -> Result<(), PortError> {
            Ok(())
        }
    }

    /// Mock `KnotStatePort` that never errors.
    #[derive(Default)]
    struct MockKnotStatePort;

    impl KnotStatePort for MockKnotStatePort {
        fn create(&self, _knot_id: &KnotId) -> Result<(), PortError> {
            Ok(())
        }

        fn update(&self, _state: crate::application::ports::KnotState) -> Result<(), PortError> {
            Ok(())
        }

        fn get(
            &self,
            _knot_id: &KnotId,
        ) -> Result<Option<crate::application::ports::KnotState>, PortError> {
            Ok(None)
        }
    }

    /// Mock `LoomLogPort` that never errors.
    #[derive(Default)]
    struct MockLoomLogPort;

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(&self, _loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Build a loom with the given ID and optional knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            source_dir: PathBuf::from("src"),
            tie_off_dir: PathBuf::from("out"),
            knots,
        }
    }

    /// Build a knot with the given ID.
    fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_config: AgentConfig::new("review".to_string()).unwrap(),
            prompt_template: PromptTemplate::new(
                "full-file".to_string(),
                "check it".to_string(),
            )
            .unwrap(),
        }
    }

    // ── Tests ───────────────────────────────────────────────────────────

    #[test]
    fn discover_looms_success() {
        let loom1 = build_loom("looms/a", vec![build_knot("k1")]);
        let loom2 = build_loom("looms/b", vec![build_knot("k2"), build_knot("k3")]);
        let discovered = vec![loom1.clone(), loom2.clone()];

        let repo = Box::new(MockLoomRepository {
            scan_result: Ok(discovered),
        });
        let state_port = Box::new(MockKnotStatePort::default());
        let log_port = Box::new(MockLoomLogPort::default());
        let store = LoomStore::new();

        let use_case =
            DiscoverLooms::new(repo, state_port, log_port, store.clone());

        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_ok());
        let looms = result.unwrap();
        assert_eq!(looms.len(), 2);

        // Store should contain both looms
        let stored = store.list();
        assert_eq!(stored.len(), 2);
        let stored_ids: HashSet<_> = stored.iter().map(|l| l.id.0.as_str()).collect();
        assert!(stored_ids.contains("looms/a"));
        assert!(stored_ids.contains("looms/b"));
    }

    #[test]
    fn discover_looms_empty_workspace() {
        let repo = Box::new(MockLoomRepository {
            scan_result: Ok(vec![]),
        });
        let state_port = Box::new(MockKnotStatePort::default());
        let log_port = Box::new(MockLoomLogPort::default());
        let store = LoomStore::new();

        let use_case =
            DiscoverLooms::new(repo, state_port, log_port, store.clone());

        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
        assert!(store.list().is_empty());
    }

    #[test]
    fn discover_looms_repository_error() {
        let repo = Box::new(MockLoomRepository {
            scan_result: Err(PortError::WorkspaceScanFailed(
                "permission denied".to_string(),
            )),
        });
        let state_port = Box::new(MockKnotStatePort::default());
        let log_port = Box::new(MockLoomLogPort::default());
        let store = LoomStore::new();

        let use_case =
            DiscoverLooms::new(repo, state_port, log_port, store.clone());

        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            PortError::WorkspaceScanFailed("permission denied".to_string())
        );
        // Store should be untouched
        assert!(store.list().is_empty());
    }
}
