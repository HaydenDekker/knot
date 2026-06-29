//! Use case: list all registered looms as summaries.

use crate::application::store::LoomStore;

use super::super::types::LoomSummary;

/// Reads from `LoomStore::list()` and maps each loom to a lightweight
/// `LoomSummary`.
pub struct ListLooms {
    store: LoomStore,
}

impl ListLooms {
    /// Create a new `ListLooms` use case.
    pub fn new(store: LoomStore) -> Self {
        Self { store }
    }

    /// Return summaries of all registered looms.
    pub fn execute(&self) -> Vec<LoomSummary> {
        self.store
            .list()
            .into_iter()
            .map(|loom| LoomSummary {
                id: loom.id,
                knot_count: loom.knots.len(),
            })
            .collect()
    }
}
