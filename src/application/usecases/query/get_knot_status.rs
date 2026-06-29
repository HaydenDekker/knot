//! Use case: get the current processing state of a knot.

use std::sync::Arc;

use crate::application::ports::{LoomLogPort, ProcessingStatus, PortError};
use crate::application::store::LoomStore;
use crate::domain::entities::{KnotId, LoomId};
use crate::domain::events::LoomEvent;

use super::super::types::KnotStatus;

/// Reads the loom-log via `LoomLogPort::read_all()` and derives the
/// current status from the latest knot-related event for the given
/// `knot_id` in the given `loom_id`.
///
/// Returns `PortError::KnotStatusDeriveFailed` if the loom is not found
/// or no events exist for the knot.
pub struct GetKnotStatus {
    store: LoomStore,
    log_port: Arc<dyn LoomLogPort>,
}

impl GetKnotStatus {
    /// Create a new `GetKnotStatus` use case.
    pub fn new(store: LoomStore, log_port: Arc<dyn LoomLogPort>) -> Self {
        Self { store, log_port }
    }

    /// Derive the current status for the given knot from loom-log events.
    pub fn execute(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) -> Result<KnotStatus, PortError> {
        // Verify the loom exists
        if self.store.get(loom_id).is_none() {
            return Err(PortError::KnotStatusDeriveFailed(format!(
                "loom '{}' not found",
                loom_id.0
            )));
        }

        // Read all events from the loom log
        let events = self.log_port.read_all(loom_id).map_err(|_| {
            PortError::KnotStatusDeriveFailed(format!(
                "failed to read loom-log for loom '{}'",
                loom_id.0
            ))
        })?;

        // Find the latest knot-specific event
        let latest = Self::find_latest_knot_event(&events, knot_id);

        match latest {
            Some(event) => Ok(Self::derive_status(loom_id, knot_id, event)),
            None => Err(PortError::KnotStatusDeriveFailed(format!(
                "no events found for knot '{}' in loom '{}'",
                knot_id.0,
                loom_id.0
            ))),
        }
    }

    /// Find the latest loom event that references the given knot.
    fn find_latest_knot_event<'a>(
        events: &'a [LoomEvent],
        knot_id: &KnotId,
    ) -> Option<&'a LoomEvent> {
        events.iter().rev().find(|event| match event {
            LoomEvent::KnotRegistered { knot_id: kid, .. }
            | LoomEvent::KnotProcessing { knot_id: kid, .. }
            | LoomEvent::KnotCompleted { knot_id: kid, .. }
            | LoomEvent::KnotFailed { knot_id: kid, .. }
            | LoomEvent::KnotEmptyResponse { knot_id: kid, .. } => kid == knot_id,
            _ => false,
        })
    }

    /// Derive a `KnotStatus` from a single loom event.
    fn derive_status(
        loom_id: &LoomId,
        knot_id: &KnotId,
        event: &LoomEvent,
    ) -> KnotStatus {
        match event {
            LoomEvent::KnotRegistered { .. } => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Idle,
                last_strand_path: None,
                last_tie_off_path: None,
                last_error: None,
            },
            LoomEvent::KnotProcessing { strand_path, .. } => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Processing,
                last_strand_path: Some(strand_path.clone()),
                last_tie_off_path: None,
                last_error: None,
            },
            LoomEvent::KnotCompleted {
                strand_path,
                tie_off_path,
                ..
            } => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Completed,
                last_strand_path: Some(strand_path.clone()),
                last_tie_off_path: Some(tie_off_path.clone()),
                last_error: None,
            },
            LoomEvent::KnotFailed { strand_path, error, .. } => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Failed,
                last_strand_path: Some(strand_path.clone()),
                last_tie_off_path: None,
                last_error: Some(error.clone()),
            },
            LoomEvent::KnotEmptyResponse { strand_path, .. } => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Failed,
                last_strand_path: Some(strand_path.clone()),
                last_tie_off_path: None,
                last_error: Some("agent returned empty response".to_string()),
            },
            // Fallback for non-knot-specific events
            _ => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Idle,
                last_strand_path: None,
                last_tie_off_path: None,
                last_error: None,
            },
        }
    }
}
