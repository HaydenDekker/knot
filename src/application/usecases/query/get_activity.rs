//! Use case: read the activity log for a loom.

use std::sync::Arc;

use crate::application::ports::{LoomLogPort, PortError};
use crate::domain::entities::LoomId;
use crate::domain::events::LoomEvent;

/// Calls `LoomLogPort::read_all()` and returns all recorded events.
pub struct GetLoomActivity {
    log_port: Arc<dyn LoomLogPort>,
}

impl GetLoomActivity {
    /// Create a new `GetLoomActivity` use case.
    pub fn new(log_port: Arc<dyn LoomLogPort>) -> Self {
        Self { log_port }
    }

    /// Return all log entries for the given loom.
    pub fn execute(&self, loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
        self.log_port.read_all(loom_id)
    }
}
