//! Use case: retrieve a full loom by ID.

use crate::application::ports::PortError;
use crate::application::store::LoomStore;
use crate::domain::entities::{Loom, LoomId};

/// Reads from `LoomStore::get()`. Returns `PortError::LoomNotFound` if
/// the loom does not exist.
pub struct GetLoom {
    store: LoomStore,
}

impl GetLoom {
    /// Create a new `GetLoom` use case.
    pub fn new(store: LoomStore) -> Self {
        Self { store }
    }

    /// Return the full loom with the given ID.
    pub fn execute(&self, id: &LoomId) -> Result<Loom, PortError> {
        self.store
            .get(id)
            .ok_or_else(|| PortError::LoomNotFound(id.clone()))
    }
}
