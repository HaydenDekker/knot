//! In-memory loom registry.
//!
//! `LoomStore` holds the active set of looms in memory.
//! It depends on no concrete adapters — it is pure application layer.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::domain::entities::{Loom, LoomId};

// ── LoomStore ──────────────────────────────────────────────────────────────

/// In-memory registry of looms.
///
/// Thread-safe via `Arc<RwLock<...>>`. Cloning the store is cheap —
/// it clones the `Arc`, not the inner data. Methods return clones to
/// avoid holding locks across call boundaries.
#[derive(Clone)]
pub struct LoomStore {
    looms: Arc<RwLock<HashMap<LoomId, Loom>>>,
}

impl Default for LoomStore {
    fn default() -> Self {
        Self {
            looms: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl LoomStore {
    /// Create a new empty loom store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a loom in the store.
    ///
    /// If a loom with the same ID already exists it is replaced.
    pub fn register(&self, loom: Loom) {
        let mut store = self.looms.write().unwrap();
        store.insert(loom.id.clone(), loom);
    }

    /// Unregister a loom by its ID.
    ///
    /// Returns `true` if the loom was present and removed.
    pub fn unregister(&self, id: &LoomId) -> bool {
        let mut store = self.looms.write().unwrap();
        store.remove(id).is_some()
    }

    /// Get a loom by its ID.
    ///
    /// Returns `Some(loom)` if present, `None` otherwise.
    pub fn get(&self, id: &LoomId) -> Option<Loom> {
        let store = self.looms.read().unwrap();
        store.get(id).cloned()
    }

    /// List all registered looms.
    ///
    /// Returns a `Vec` — order is not guaranteed.
    pub fn list(&self) -> Vec<Loom> {
        let store = self.looms.read().unwrap();
        store.values().cloned().collect()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
    use std::path::PathBuf;

    /// Build a loom for testing with the given ID.
    fn build_loom(id: LoomId) -> Loom {
        Loom {
            id,
            source_dir: PathBuf::from("src"),
            tie_off_dir: PathBuf::from("out"),
            knots: vec![
                crate::domain::entities::Knot {
                    id: crate::domain::entities::KnotId("k1".to_string()),
                    agent_config: AgentConfig::new(
                        "review".to_string(),
                        "openai".to_string(),
                        "gpt-4o".to_string(),
                    )
                    .unwrap(),
                    prompt_template: PromptTemplate::new(
                        "full-file".to_string(),
                        "check it".to_string(),
                    )
                    .unwrap(),
                    source_dir: None,
                    tie_off_dir: None,
                },
            ],
        }
    }

    #[test]
    fn register_loom() {
        let store = LoomStore::new();
        let loom = build_loom(LoomId("prds".to_string()));

        store.register(loom.clone());

        let listed = store.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, LoomId("prds".to_string()));
    }

    #[test]
    fn list_looms() {
        let store = LoomStore::new();

        let loom1 = build_loom(LoomId("prds".to_string()));
        let loom2 = build_loom(LoomId("docs".to_string()));

        store.register(loom1);
        store.register(loom2);

        let listed = store.list();
        assert_eq!(listed.len(), 2);

        let ids: Vec<_> = listed.iter().map(|l| &l.id).collect();
        assert!(ids.contains(&&LoomId("prds".to_string())));
        assert!(ids.contains(&&LoomId("docs".to_string())));
    }

    #[test]
    fn get_loom_by_id() {
        let store = LoomStore::new();
        let loom = build_loom(LoomId("prds".to_string()));

        store.register(loom.clone());

        let found = store.get(&LoomId("prds".to_string()));
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, LoomId("prds".to_string()));
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let store = LoomStore::new();

        let found = store.get(&LoomId("unknown".to_string()));
        assert!(found.is_none());
    }

    #[test]
    fn unregister_loom() {
        let store = LoomStore::new();
        let loom = build_loom(LoomId("prds".to_string()));

        store.register(loom);
        assert!(store.get(&LoomId("prds".to_string())).is_some());

        let removed = store.unregister(&LoomId("prds".to_string()));
        assert!(removed);

        assert!(store.get(&LoomId("prds".to_string())).is_none());
        assert!(store.list().is_empty());
    }
}
