//! Helper functions for `ProcessStrand::execute()`.
//!
//! Extracted from `ProcessStrand` impl to reduce the size of
//! `process_strand.rs` and improve modularity.

use crate::application::usecases::process_strand::ProcessStrand;
use crate::application::usecases::strand_event_metadata::{extract_expected_event_ids, extract_event_metadata};
use crate::application::ports::PortError;
use crate::domain::entities::{Knot, KnotId, LoomId, StrandPath, TieOff, TieOffOutcome, TieOffPath};

/// Result of config resolution, prompt building, and agent execution.
///
/// Returned by `resolve_config_and_build()` and consumed by the
/// `execute()` coordinator to drive tie-off writing and success/failure paths.
pub struct ResolvedExecution {
    /// Derived outcome from agent execution.
    pub outcome: TieOffOutcome,
    /// Session ID (set by session-resume retry logic).
    pub session_id: Option<String>,
    /// Listener context injected into the prompt.
    /// Used by event enforcement to check if events were expected.
    pub listener_context: String,
    /// All knots from all looms (for event enforcement follow-up).
    pub all_knots: Vec<Knot>,
    /// Profile's session timeout (for event enforcement follow-up).
    pub profile_timeout: Option<std::time::Duration>,
}

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

/// Handle non-success outcome: write KnotFailed + StrandProcessed logs.
pub fn handle_failure(
    ps: &ProcessStrand,
    outcome: &TieOffOutcome,
    strand_kind: &str,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_path: &StrandPath,
) -> Result<(), PortError> {
    use crate::adapters::logging;
    use crate::domain::events::LoomEvent;
    use crate::application::usecases::types::format_timestamp;

    let error_msg = outcome
        .error_message()
        .map(|s| s.to_string())
        .unwrap_or_default();

    ps.log_port.append(LoomEvent::KnotFailed {
        loom_id: loom_id.clone(),
        knot_id: knot_id.clone(),
        strand_path: strand_path.clone(),
        error: error_msg.clone(),
        timestamp: format_timestamp(),
    })?;

    ps.log_port.append(LoomEvent::StrandProcessed {
        loom_id: loom_id.clone(),
        strand_path: strand_path.clone(),
        error: Some(error_msg.clone()),
        timestamp: format_timestamp(),
    })?;

    logging::log_strand_event(
        &format!("{} failed (knot={}): {}", strand_kind, knot_id.0, error_msg),
        &strand_path.0,
    );

    Ok(())
}

/// Handle success outcome: event dispatch, KnotCompleted, StrandProcessed,
/// event enforcement, git commit, and completion logging.
pub fn handle_success(
    ps: &ProcessStrand,
    outcome: &TieOffOutcome,
    resolved: &ResolvedExecution,
    strand_kind: &str,
    knot: &Knot,
    tie_off_path: &TieOffPath,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_path: &StrandPath,
    event_label: &str,
) -> Result<(), PortError> {
    use crate::adapters::logging;
    use crate::application::session_resume;
    use crate::application::usecases::types::format_timestamp;
    use crate::domain::events::LoomEvent;
    use crate::domain::entities::Knot;

    // Dispatch agent events to matching consumer knots
    // (best-effort — dispatch failures are non-fatal).
    if let Some(ref content) = outcome.tie_off_content() {
        if let Ok(Some(dispatch_event)) = ps.dispatch_agent_events(
            content,
            knot,
            loom_id,
            strand_path,
        ) {
            let _ = ps.log_port.append(dispatch_event);
        }
    }

    // Append KnotCompleted to loom-log.
    ps.log_port.append(LoomEvent::KnotCompleted {
        loom_id: loom_id.clone(),
        knot_id: knot_id.clone(),
        strand_path: strand_path.clone(),
        tie_off_path: tie_off_path.clone(),
        timestamp: format_timestamp(),
    })?;

    // Append StrandProcessed to loom-log.
    ps.log_port.append(LoomEvent::StrandProcessed {
        loom_id: loom_id.clone(),
        strand_path: strand_path.clone(),
        error: None,
        timestamp: format_timestamp(),
    })?;

    // ── Event Enforcement ──────────────────────────────────────────
    // If the agent was instructed to emit events but produced
    // none, log a KnotEventsMissing and attempt one follow-up.
    let all_knot_ids: Vec<&str> = resolved
        .all_knots
        .iter()
        .map(|k| k.id.0.as_str())
        .collect();
    if !resolved.listener_context.is_empty() {
        if let Some(ref content) = outcome.tie_off_content() {
            if crate::domain::tieoff_parser::has_no_events(content)
            {
                let expected_events = extract_expected_event_ids(
                    knot,
                    loom_id,
                    &resolved.all_knots,
                );

                // Log the first KnotEventsMissing
                let _ = ps.log_port.append(
                    LoomEvent::KnotEventsMissing {
                        loom_id: loom_id.clone(),
                        knot_id: knot_id.clone(),
                        strand_path: strand_path.clone(),
                        expected_events: expected_events.clone(),
                        timestamp: format_timestamp(),
                    },
                );

                // Attempt follow-up re-entry (best-effort).
                // Only possible if session_id is available.
                if let Ok((followup_config, _, _)) =
                    ps.resolve_agent_config(knot)
                {
                    let followup_result =
                        session_resume::inject_event_request(
                            &*ps.agent_runner,
                            &*ps.log_port,
                            loom_id,
                            knot_id,
                            strand_path,
                            &resolved.session_id,
                            followup_config,
                            resolved.listener_context.clone(),
                            event_label.to_string(),
                            Some(knot.id.0.clone()),
                            resolved.profile_timeout.clone(),
                        );

                    match followup_result {
                    Ok(response) => {
                        // Parse follow-up for events
                        let followup_events =
                            crate::domain::tieoff_parser::
                                extract_agent_events(&response);

                        if !followup_events.is_empty() {
                            // Dispatch follow-up events
                            // (dispatch failures are non-fatal)
                            let _ = ps.dispatch_events_to_consumers(
                                &followup_events,
                                knot,
                                loom_id,
                                &all_knot_ids,
                            );
                        } else {
                            // Still no events — log again
                            let _ = ps.log_port.append(
                                LoomEvent::KnotEventsMissing {
                                    loom_id: loom_id.clone(),
                                    knot_id: knot_id.clone(),
                                    strand_path: strand_path
                                        .clone(),
                                    expected_events: expected_events
                                        .clone(),
                                    timestamp: format_timestamp(),
                                },
                            );
                        }
                    }
                    Err(e) => {
                        // No session ID or runner error
                        // — log gracefully, do not fail strand
                        eprintln!(
                            "event enforcement follow-up failed (knot={}): {}",
                            knot_id.0,
                            e
                        );
                    }
                }
                }
            }
        }
    }

    // Git versioning commit (best-effort, non-fatal).
    // Runs last so the commit captures all artifacts from the
    // turn: tie-off, dispatched events, and loom-log entries.
    if knot.git_versioned {
        if let Some(ref content) = outcome.tie_off_content() {
            let commit_result = ps.git_versioning_port.commit(
                loom_id,
                knot_id,
                strand_path,
                event_label,
                content,
            );
            if let Err(ref e) = commit_result {
                logging::log_strand_event(
                    &format!("git commit warning: {}", e),
                    &strand_path.0,
                );
            }
        }
    }

    logging::log_strand_event(
        &format!("{} completed (knot={})", strand_kind, knot_id.0),
        &strand_path.0,
    );

    Ok(())
}
