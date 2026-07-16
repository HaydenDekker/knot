//! Helper functions for `ProcessStrand::execute()`.
//!
//! Extracted from `ProcessStrand` impl to reduce the size of
//! `process_strand.rs` and improve modularity.

use crate::application::usecases::process_strand::ProcessStrand;
use crate::application::usecases::strand_event_metadata::extract_event_metadata;
use crate::domain::entities::{Knot, StrandPath, TieOff, TieOffOutcome, TieOffPath};

/// Construct a `TieOff` from execution outcome and write it.
///
/// Skipped for timeout outcomes (tie-off preserved unchanged).
pub fn write_tie_off(
    ps: &ProcessStrand,
    outcome: &TieOffOutcome,
    knot: &Knot,
    tie_off_path: &TieOffPath,
    strand_path: &StrandPath,
    event_label: &str,
) {
    if !outcome.should_write_tie_off() {
        return;
    }

    // Extract event metadata if this strand is an event file
    // (dispatched by intent-based routing).
    let event_metadata = extract_event_metadata(strand_path);

    let tie_off = TieOff {
        content: outcome.tie_off_content().unwrap_or_default(),
        path: tie_off_path.clone(),
        status: outcome
            .tie_off_status()
            .unwrap_or(crate::domain::entities::TieOffStatus::Produced),
        knot_name: Some(knot.id.0.clone()),
        event_type: Some(event_label.to_string()),
        strand_path: Some(strand_path.0.display().to_string()),
        timestamp: None,
        agent_events: Vec::new(),
        event_metadata: event_metadata.unwrap_or_default(),
    };
    let _ = ps.tie_off_sink.append(tie_off);
}
