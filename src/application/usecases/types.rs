//! Shared types used across use case modules.

use serde::{Deserialize, Serialize};

use crate::adapters::logging;
use crate::application::ports::ProcessingStatus;
use crate::domain::entities::{KnotId, LoomId, StrandPath, TieOffPath};

/// Generate an ISO 8601 UTC timestamp string.
pub fn format_timestamp() -> String {
    logging::format_timestamp()
}

// ── Query Result Types ───────────────────────────────────────────────────

/// A summary of a loom (lightweight, for list responses).
///
/// The loom directory is derived from the loom ID and rig base path
/// (naming convention `*-loom`). Strand and tie-off directories are
/// per-knot fields, not loom-level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoomSummary {
    /// The loom's unique ID (must end in `-loom`).
    pub id: LoomId,
    /// Number of knots in this loom.
    pub knot_count: usize,
}

/// Result of the `GetKnotStatus` use case.
///
/// Derived from the latest loom-log entries for a knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnotStatus {
    /// The knot whose status was retrieved.
    pub knot_id: KnotId,
    /// The loom this knot belongs to.
    pub loom_id: LoomId,
    /// The current processing status derived from loom-log events.
    pub status: ProcessingStatus,
    /// Path to the last strand processed (if any).
    pub last_strand_path: Option<StrandPath>,
    /// Path to the last tie-off produced (if any).
    pub last_tie_off_path: Option<TieOffPath>,
    /// Error message from the last failed processing (if any).
    pub last_error: Option<String>,
}
