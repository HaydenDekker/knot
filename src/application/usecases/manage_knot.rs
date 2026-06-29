//! Use case: manage individual knots within a loom.

use crate::adapters::logging;
use crate::application::ports::PortError;
use crate::application::store::LoomStore;
use crate::domain::entities::{Knot, KnotId, LoomId};

/// Action to perform on a knot within a loom.
///
/// Used by the `ManageKnot` use case for HTTP-driven knot CRUD.
#[derive(Debug, Clone)]
pub enum KnotAction {
    /// Add a new knot to the loom.
    ///
    /// Returns `PortError::LoomSaveFailed` if the loom is not found
    /// or a knot with the same ID already exists.
    Create { loom_id: LoomId, knot: Knot },
    /// Update an existing knot's configuration.
    ///
    /// Returns `PortError::LoomSaveFailed` if the loom or knot
    /// is not found.
    Update { loom_id: LoomId, knot: Knot },
    /// Remove a knot from the loom.
    ///
    /// Returns `PortError::LoomSaveFailed` if the loom or knot
    /// is not found.
    Delete { loom_id: LoomId, knot_id: KnotId },
}

/// Use case: manage individual knots within a loom.
///
/// Pure in-memory operation — updates `LoomStore` only. File I/O
/// (writing `.md` files) is handled by the HTTP handler, consistent
/// with the `POST /looms` pattern. The `ConfigEventHandler` picks up
/// file changes via the watcher (idempotent — store already matches).
///
/// Supports:
/// - `KnotAction::Create` — add a new knot to the loom
/// - `KnotAction::Update` — modify an existing knot's config
/// - `KnotAction::Delete` — remove a knot from the loom
pub struct ManageKnot {
    store: LoomStore,
}

impl ManageKnot {
    /// Create a new `ManageKnot` use case.
    pub fn new(store: LoomStore) -> Self {
        Self { store }
    }

    /// Execute the knot management action.
    pub fn execute(&self, action: KnotAction) -> Result<(), PortError> {
        match action {
            KnotAction::Create { loom_id, knot } => {
                self.create_knot(&loom_id, knot)
            }
            KnotAction::Update { loom_id, knot } => {
                self.update_knot(&loom_id, knot)
            }
            KnotAction::Delete { loom_id, knot_id } => {
                self.delete_knot(&loom_id, &knot_id)
            }
        }
    }

    fn create_knot(
        &self,
        loom_id: &LoomId,
        knot: Knot,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        // Check for duplicate knot ID
        if loom.knots.iter().any(|k| k.id == knot.id) {
            return Err(PortError::LoomSaveFailed(format!(
                "knot '{}' already exists in loom '{}'",
                knot.id.0,
                loom_id.0
            )));
        }

        loom.knots.push(knot.clone());
        self.store.register(loom);
        logging::log_knot_event(
            "created",
            &loom_id.0,
            &knot.id.0,
            "store updated (watcher started by caller)",
        );
        Ok(())
    }

    fn update_knot(
        &self,
        loom_id: &LoomId,
        knot: Knot,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        let pos = loom.knots.iter()
            .position(|k| k.id == knot.id)
            .ok_or_else(|| PortError::LoomSaveFailed(format!(
                "knot '{}' not found in loom '{}'",
                knot.id.0,
                loom_id.0
            )))?;

        loom.knots[pos] = knot.clone();
        self.store.register(loom);
        logging::log_knot_event(
            "updated",
            &loom_id.0,
            &knot.id.0,
            "store updated (watcher managed by caller)",
        );
        Ok(())
    }

    fn delete_knot(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        let found = loom.knots.iter()
            .position(|k| k.id == *knot_id)
            .ok_or_else(|| PortError::LoomSaveFailed(format!(
                "knot '{}' not found in loom '{}'",
                knot_id.0,
                loom_id.0
            )))?;

        loom.knots.remove(found);
        self.store.register(loom);
        logging::log_knot_event(
            "deleted",
            &loom_id.0,
            &knot_id.0,
            "store updated (watcher stopped by caller)",
        );
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod manage_knot_tests {
    use super::*;
    use crate::domain::value_objects::PromptTemplate;
    use std::path::PathBuf;

    use super::super::test_fixtures::build_loom;

    /// Build a knot with the given ID (uses "default" profile).
    fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: "default".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    /// `ManageKnot` with `KnotAction::Create` adds a new knot to the
    /// loom in the store.
    #[test]
    fn manage_knot_create() {
        let store = LoomStore::new();
        // Pre-register a loom with one knot
        let loom = build_loom("test", vec![build_knot("k1")]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let new_knot = build_knot("k2");
        let result = use_case.execute(KnotAction::Create {
            loom_id: LoomId("test".to_string()),
            knot: new_knot,
        });

        // Should succeed
        assert!(result.is_ok());

        // Loom now has 2 knots
        let updated = store.get(&LoomId("test".to_string())).unwrap();
        assert_eq!(updated.knots.len(), 2);

        // New knot is present with correct ID
        let found = updated.knots.iter()
            .find(|k| k.id == KnotId("k2".to_string()));
        assert!(found.is_some());
        let k = found.unwrap();
        assert_eq!(k.agent_profile_ref, "default");
        assert_eq!(k.strand_dir, PathBuf::from("strands"));
    }

    /// `ManageKnot` with `KnotAction::Create` returns error when loom
    /// does not exist.
    #[test]
    fn manage_knot_create_loom_not_found() {
        let store = LoomStore::new();
        let use_case = ManageKnot::new(store.clone());

        let result = use_case.execute(KnotAction::Create {
            loom_id: LoomId("unknown".to_string()),
            knot: build_knot("k1"),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("unknown".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }
    }

    /// `ManageKnot` with `KnotAction::Create` returns error when knot
    /// already exists.
    #[test]
    fn manage_knot_create_duplicate() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![build_knot("k1")]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let result = use_case.execute(KnotAction::Create {
            loom_id: LoomId("test".to_string()),
            knot: build_knot("k1"),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomSaveFailed(msg) => {
                assert!(msg.contains("already exists"));
            }
            other => panic!("Expected LoomSaveFailed, got {other:?}"),
        }
    }

    /// `ManageKnot` with `KnotAction::Update` updates an existing knot's
    /// configuration in the store.
    #[test]
    fn manage_knot_update() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![
            build_knot("k1"),
            build_knot("k2"),
        ]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        // Update k1 with a new profile ref
        let mut updated_knot = build_knot("k1");
        updated_knot.agent_profile_ref = "slow".to_string();
        updated_knot.prompt_template.instructions = "new instructions".to_string();

        let result = use_case.execute(KnotAction::Update {
            loom_id: LoomId("test".to_string()),
            knot: updated_knot,
        });

        // Should succeed
        assert!(result.is_ok());

        // Loom still has 2 knots
        let loom = store.get(&LoomId("test".to_string())).unwrap();
        assert_eq!(loom.knots.len(), 2);

        // k1 has updated config
        let k1 = loom.knots.iter()
            .find(|k| k.id == KnotId("k1".to_string()))
            .unwrap();
        assert_eq!(k1.agent_profile_ref, "slow");
        assert_eq!(
            k1.prompt_template.instructions,
            "new instructions"
        );

        // k2 is unchanged
        let k2 = loom.knots.iter()
            .find(|k| k.id == KnotId("k2".to_string()))
            .unwrap();
        assert_eq!(k2.agent_profile_ref, "default");
    }

    /// `ManageKnot` with `KnotAction::Update` returns error when knot
    /// does not exist.
    #[test]
    fn manage_knot_update_not_found() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![build_knot("k1")]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let result = use_case.execute(KnotAction::Update {
            loom_id: LoomId("test".to_string()),
            knot: build_knot("k_unknown"),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomSaveFailed(msg) => {
                assert!(msg.contains("not found"));
            }
            other => panic!("Expected LoomSaveFailed, got {other:?}"),
        }
    }

    /// `ManageKnot` with `KnotAction::Delete` removes a knot from the
    /// loom in the store.
    #[test]
    fn manage_knot_delete() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![
            build_knot("k1"),
            build_knot("k2"),
            build_knot("k3"),
        ]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let result = use_case.execute(KnotAction::Delete {
            loom_id: LoomId("test".to_string()),
            knot_id: KnotId("k2".to_string()),
        });

        // Should succeed
        assert!(result.is_ok());

        // Loom now has 2 knots (k2 removed)
        let updated = store.get(&LoomId("test".to_string())).unwrap();
        assert_eq!(updated.knots.len(), 2);

        let ids: Vec<_> = updated.knots.iter()
            .map(|k| k.id.0.as_str())
            .collect();
        assert!(ids.contains(&"k1"));
        assert!(ids.contains(&"k3"));
        assert!(!ids.contains(&"k2"));
    }

    /// `ManageKnot` with `KnotAction::Delete` returns error when knot
    /// does not exist.
    #[test]
    fn manage_knot_delete_not_found() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![build_knot("k1")]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let result = use_case.execute(KnotAction::Delete {
            loom_id: LoomId("test".to_string()),
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

    /// `ManageKnot` with `KnotAction::Delete` returns error when loom
    /// does not exist.
    #[test]
    fn manage_knot_delete_loom_not_found() {
        let store = LoomStore::new();
        let use_case = ManageKnot::new(store.clone());

        let result = use_case.execute(KnotAction::Delete {
            loom_id: LoomId("unknown".to_string()),
            knot_id: KnotId("k1".to_string()),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("unknown".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }
    }
}
