//! Consolidated service-log renderers — plan 083.
//!
//! Every event line in the service log is a single physical line:
//!
//! ```text
//! [ts] [KNOT][EVENT] <Variant> key=value …
//! [ts] [KNOT][STATE] change <aspect> …
//! ```
//!
//! The renderers are pure functions over the domain types (unit-tested
//! per variant); the `log_*` wrappers do the single `eprintln!` so each
//! event produces exactly one physical line.
//!
//! Field names per variant mirror the domain event fields (`loom=`,
//! `knot=`, `strand=`, `tie-off=`, `error=`, `attempt=`, `session=`,
//! `silent=`, `window=`, `blocked-call=`, `reason=`, `tokens-before=`,
//! `directory=`, `file=`, `message=`, `expected=`, `dispatch=` repeated
//! per dispatch). Option fields are omitted when `None`, except where
//! clearing is itself the news (rendered `→ cleared` in `[STATE]` lines).
//!
//! The event's own `timestamp` field is not repeated — the line's
//! leading `[ts]` is the emit time (the same instant in practice).

use crate::domain::entities::RigState;
use crate::domain::events::{LoomEvent, RigLogEvent};
use crate::domain::state_change::{KnotField, ProfileField, StateChange};

/// Render the `[KNOT][EVENT]` line body for a loom event (no timestamp,
/// no `[KNOT][EVENT]` prefix — [`log_loom_event_line`] adds both).
///
/// Returns e.g. `KnotProcessing loom=review-loom knot=review
/// strand=strands/prd.md`.
pub fn render_loom_event_line(event: &LoomEvent) -> String {
    match event {
        LoomEvent::LoomStarted { loom_id, .. } => {
            format!("LoomStarted loom={}", loom_id.0)
        }
        LoomEvent::LoomStopped { loom_id, .. } => {
            format!("LoomStopped loom={}", loom_id.0)
        }
        LoomEvent::KnotRegistered { loom_id, knot_id, .. } => {
            format!("KnotRegistered loom={} knot={}", loom_id.0, knot_id.0)
        }
        LoomEvent::KnotDeregistered { loom_id, knot_id, .. } => {
            format!("KnotDeregistered loom={} knot={}", loom_id.0, knot_id.0)
        }
        LoomEvent::KnotProcessing {
            loom_id,
            knot_id,
            strand_path,
            ..
        } => format!(
            "KnotProcessing loom={} knot={} strand={}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display()
        ),
        LoomEvent::KnotCompleted {
            loom_id,
            knot_id,
            strand_path,
            tie_off_path,
            ..
        } => format!(
            "KnotCompleted loom={} knot={} strand={} tie-off={}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            tie_off_path.0.display()
        ),
        LoomEvent::KnotFailed {
            loom_id,
            knot_id,
            strand_path,
            error,
            ..
        } => format!(
            "KnotFailed loom={} knot={} strand={} error={}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            error
        ),
        LoomEvent::StrandProcessed {
            loom_id,
            strand_path,
            error,
            ..
        } => {
            let mut line = format!(
                "StrandProcessed loom={} strand={}",
                loom_id.0,
                strand_path.0.display()
            );
            if let Some(e) = error {
                line.push_str(&format!(" error={e}"));
            }
            line
        }
        LoomEvent::StrandIgnored {
            loom_id,
            knot_id,
            strand_path,
            reason,
            ..
        } => format!(
            "StrandIgnored loom={} knot={} strand={} reason={}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            reason
        ),
        LoomEvent::StrandSkipped {
            loom_id,
            knot_id,
            strand_path,
            reason,
            ..
        } => format!(
            "StrandSkipped loom={} knot={} strand={} reason={}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            reason
        ),
        LoomEvent::AgentInactivity {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            silent_secs,
            window_secs,
            blocked_call,
            attempt,
            ..
        } => {
            let mut line = format!(
                "AgentInactivity loom={} knot={} strand={} session={} silent={} window={}",
                loom_id.0,
                knot_id.0,
                strand_path.0.display(),
                session_id,
                silent_secs,
                window_secs
            );
            if let Some(call) = blocked_call {
                line.push_str(&format!(" blocked-call={call}"));
            }
            line.push_str(&format!(" attempt={attempt}"));
            line
        }
        LoomEvent::SessionResumed {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            attempt,
            ..
        } => format!(
            "SessionResumed loom={} knot={} strand={} session={} attempt={attempt}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            session_id
        ),
        LoomEvent::KnotEmptyResponse {
            loom_id,
            knot_id,
            strand_path,
            attempt,
            ..
        } => format!(
            "KnotEmptyResponse loom={} knot={} strand={} attempt={attempt}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display()
        ),
        LoomEvent::CompactionStarted {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            reason,
            attempt,
            ..
        } => format!(
            "CompactionStarted loom={} knot={} strand={} session={} reason={reason} attempt={attempt}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            session_id
        ),
        LoomEvent::ContextCompacted {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            reason,
            tokens_before,
            attempt,
            ..
        } => {
            let mut line = format!(
                "ContextCompacted loom={} knot={} strand={} session={} reason={reason}",
                loom_id.0,
                knot_id.0,
                strand_path.0.display(),
                session_id
            );
            if let Some(tokens) = tokens_before {
                line.push_str(&format!(" tokens-before={tokens}"));
            }
            line.push_str(&format!(" attempt={attempt}"));
            line
        }
        LoomEvent::ContextCompactionFailed {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            reason,
            error,
            aborted,
            attempt,
            ..
        } => {
            let mut line = format!(
                "ContextCompactionFailed loom={} knot={} strand={} session={} reason={reason}",
                loom_id.0,
                knot_id.0,
                strand_path.0.display(),
                session_id
            );
            if let Some(err) = error {
                line.push_str(&format!(" error={err}"));
            }
            line.push_str(&format!(" aborted={aborted} attempt={attempt}"));
            line
        }
        LoomEvent::CompactionInterrupted {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            reason,
            attempt,
            ..
        } => format!(
            "CompactionInterrupted loom={} knot={} strand={} session={} reason={reason} attempt={attempt}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            session_id
        ),
        LoomEvent::ManualCompactionSucceeded {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            tokens_before,
            attempt,
            ..
        } => format!(
            "ManualCompactionSucceeded loom={} knot={} strand={} session={} tokens-before={tokens_before} attempt={attempt}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            session_id
        ),
        LoomEvent::ManualCompactionFailed {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            error,
            attempt,
            ..
        } => format!(
            "ManualCompactionFailed loom={} knot={} strand={} session={} error={error} attempt={attempt}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            session_id
        ),
        LoomEvent::SessionRestarted {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            attempt,
            ..
        } => format!(
            "SessionRestarted loom={} knot={} strand={} session={} attempt={attempt}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            session_id
        ),
        // Plan 089 (D9): `session=` only appears when the abandoned run's
        // session id is known (it never is at startup — the id died with the
        // process), so the common line stays short.
        LoomEvent::RunAbandoned {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            ..
        } => {
            let mut line = format!(
                "RunAbandoned loom={} knot={} strand={}",
                loom_id.0,
                knot_id.0,
                strand_path.0.display()
            );
            if let Some(sid) = session_id {
                line.push_str(&format!(" session={sid}"));
            }
            line
        }
        LoomEvent::TurnContinued {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            reason,
            attempt,
            ..
        } => format!(
            "TurnContinued loom={} knot={} strand={} session={} reason={reason} attempt={attempt}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            session_id
        ),
        LoomEvent::ContextWrapUpSteered {
            loom_id,
            knot_id,
            strand_path,
            session_id,
            context_tokens,
            limit,
            attempt,
            ..
        } => format!(
            "ContextWrapUpSteered loom={} knot={} strand={} session={} context-tokens={context_tokens} limit={limit} attempt={attempt}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            session_id
        ),
        LoomEvent::EventsDispatched {
            loom_id,
            knot_id,
            strand_path,
            dispatches,
            ..
        } => {
            let mut line = format!(
                "EventsDispatched loom={} knot={} strand={}",
                loom_id.0,
                knot_id.0,
                strand_path.0.display()
            );
            for (event_id, consumer_knot, consumer_loom, file_path) in dispatches {
                line.push_str(&format!(
                    " dispatch={event_id}→{consumer_loom}/{consumer_knot}: {file_path}"
                ));
            }
            line
        }
        LoomEvent::KnotEventsMissing {
            loom_id,
            knot_id,
            strand_path,
            expected_events,
            missing_events,
            ..
        } => format!(
            "KnotEventsMissing loom={} knot={} strand={} expected={} missing={}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display(),
            expected_events.join(","),
            missing_events.join(",")
        ),
        LoomEvent::KnotParseWarning {
            loom_id,
            knot_file_name,
            message,
            ..
        } => format!(
            "KnotParseWarning loom={} file={knot_file_name} message={message}",
            loom_id.0
        ),
        LoomEvent::DirectoryCreated {
            loom_id,
            knot_id,
            directory,
            ..
        } => format!(
            "DirectoryCreated loom={} knot={} directory={directory}",
            loom_id.0,
            knot_id.0
        ),
        LoomEvent::TasksIncomplete {
            loom_id,
            knot_id,
            strand_path,
            continuations,
            reason,
            budget_secs,
            batch_start_epoch,
            ..
        } => {
            let mut line = format!(
                "[task-loop] handoff loom={} knot={} strand={} hop={}/{} reason={reason}",
                loom_id.0,
                knot_id.0,
                strand_path.0.display(),
                continuations,
                crate::application::session_resume::MAX_CONTINUATIONS,
            );
            if let Some(budget) = budget_secs {
                line.push_str(&format!(" remaining={budget}'s"));
            }
            if let Some(start) = batch_start_epoch {
                line.push_str(&format!(" batch-start={start}"));
            }
            line
        }
        LoomEvent::BatchIncomplete {
            loom_id,
            knot_id,
            reason,
            continuations,
            budget_secs,
            batch_start_epoch,
            ..
        } => {
            let mut line = format!(
                "[task-loop] batch-incomplete loom={} knot={} reason={reason} continuations={continuations}",
                loom_id.0,
                knot_id.0
            );
            if let Some(budget) = budget_secs {
                line.push_str(&format!(" budget={budget}'s"));
            }
            if let Some(start) = batch_start_epoch {
                line.push_str(&format!(" batch-start={start}"));
            }
            line
        },
    }
}

/// Emit a loom event as one physical `[KNOT][EVENT]` stderr line.
pub fn log_loom_event_line(event: &LoomEvent) {
    eprintln!("[{}] [KNOT][EVENT] {}", timestamp(), render_loom_event_line(event));
}

/// Render the `[KNOT][EVENT]` line body for a rig event.
///
/// Returns e.g. `TimeoutExceeded loom=prds-loom knot=review
/// strand=strands/big.md error=session exceeded 300s`.
pub fn render_rig_event_line(event: &RigLogEvent) -> String {
    match event {
        RigLogEvent::TimeoutExceeded {
            loom_id,
            knot_id,
            strand_path,
            error,
            ..
        } => format!(
            "TimeoutExceeded loom={} knot={} strand={} error={error}",
            loom_id.0,
            knot_id.0,
            strand_path.0.display()
        ),
        RigLogEvent::QueueIdle { .. } => "QueueIdle".to_string(),
    }
}

/// Emit a rig event as one physical `[KNOT][EVENT]` stderr line.
pub fn log_rig_event_line(event: &RigLogEvent) {
    eprintln!("[{}] [KNOT][EVENT] {}", timestamp(), render_rig_event_line(event));
}

fn timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

// ── State write lines ─────────────────────────────────────────────────────

/// Render the `[KNOT][STATE]` lines for a state write.
///
/// Returns a list of full lines **without** the leading timestamp — one
/// per changed aspect. The caller ([`log_state_write_lines`]) prepends a
/// single shared timestamp.
///
/// * `first_write == true` — the first write after startup (nothing
///   stored yet): a single baseline line with the snapshot totals.
/// * `first_write == false` — delta lines from the `change` (empty list
///   for a no-op write).
pub fn render_state_write_lines(
    first_write: bool,
    change: &StateChange,
    snapshot: &RigState,
) -> Vec<String> {
    if first_write {
        return vec![format!(
            "[KNOT][STATE] initial snapshot looms={} knots={} profiles={} queue={}",
            snapshot.looms.len(),
            snapshot.looms.iter().map(|l| l.knots.len()).sum::<usize>(),
            snapshot.profiles.len(),
            snapshot.strand_queue.len()
        )];
    }

    let mut lines = Vec::new();

    for loom_id in &change.looms_added {
        let knot_count = snapshot
            .looms
            .iter()
            .find(|l| &l.id == loom_id)
            .map(|l| l.knots.len())
            .unwrap_or(0);
        lines.push(format!(
            "[KNOT][STATE] change loom+ {loom_id} ({knot_count} knots)"
        ));
    }
    for loom_id in &change.looms_removed {
        lines.push(format!("[KNOT][STATE] change loom- {loom_id}"));
    }

    for (loom, knot) in &change.knots_added {
        lines.push(format!(
            "[KNOT][STATE] change knot {loom}/{knot}+ (registered)"
        ));
    }
    for (loom, knot) in &change.knots_removed {
        lines.push(format!("[KNOT][STATE] change knot {loom}/{knot}- (removed)"));
    }
    for update in &change.knot_updates {
        lines.push(format!(
            "[KNOT][STATE] change knot {}/{}: {}",
            update.loom,
            update.knot,
            render_knot_fields(&update.field_changes)
        ));
    }

    for name in &change.profiles_added {
        lines.push(format!("[KNOT][STATE] change profile+ {name} (added)"));
    }
    for name in &change.profiles_removed {
        lines.push(format!("[KNOT][STATE] change profile- {name} (removed)"));
    }
    for update in &change.profile_updates {
        lines.push(format!(
            "[KNOT][STATE] change profile {}: {}",
            update.name,
            render_profile_fields(&update.field_changes)
        ));
    }

    for entry in &change.queue_added {
        lines.push(format!(
            "[KNOT][STATE] change queue+ {} ({} {}/{})",
            entry.strand_path,
            entry.event_kind,
            entry.loom_id,
            entry.knot_id
        ));
    }
    for entry in &change.queue_removed {
        lines.push(format!("[KNOT][STATE] change queue- {}", entry.strand_path));
    }

    lines
}

/// Emit the `[KNOT][STATE]` lines (one shared timestamp).
pub fn log_state_write_lines(
    first_write: bool,
    change: &StateChange,
    snapshot: &RigState,
) {
    for line in render_state_write_lines(first_write, change, snapshot) {
        eprintln!("[{}] {line}", timestamp());
    }
}

/// Render knot field changes: `status idle→processing
/// strand=strands/prd.md` (joining with ` `, `→` for transitions,
/// `k= cleared` for clearing).
fn render_knot_fields(changes: &[(KnotField, Option<String>, Option<String>)]) -> String {
    let mut parts = Vec::new();
    for (field, from, to) in changes {
        match field {
            KnotField::Status => parts.push(format!("status {}→{}", from.as_deref().unwrap_or("?"), to.as_deref().unwrap_or("removed"))),
            KnotField::LastStrandPath => {
                if let (Some(f), None) = (from, to) {
                    parts.push(format!("strand {f}→ cleared"));
                } else {
                    parts.push(format!("strand={}", to.as_deref().unwrap_or_default()));
                }
            }
            KnotField::LastTieOffPath => {
                if let (Some(f), None) = (from, to) {
                    parts.push(format!("tie-off {f}→ cleared"));
                } else {
                    parts.push(format!("tie-off={}", to.as_deref().unwrap_or_default()));
                }
            }
            KnotField::LastError => {
                if let (Some(f), None) = (from, to) {
                    parts.push(format!("error {f}→ cleared"));
                } else {
                    parts.push(format!("error={}", to.as_deref().unwrap_or_default()));
                }
            }
            KnotField::LastEventAt => {
                parts.push(format!("event-at={}", to.as_deref().unwrap_or_default()));
            }
        }
    }
    parts.join(" ")
}

/// Render profile field changes: `model gpt-4o→o3 thinking-level=xhigh`.
fn render_profile_fields(changes: &[(ProfileField, Option<String>, Option<String>)]) -> String {
    let mut parts = Vec::new();
    for (field, from, to) in changes {
        let key = match field {
            ProfileField::ModelRef => "model-ref",
            ProfileField::Provider => "provider",
            ProfileField::Model => "model",
            ProfileField::ThinkingLevel => "thinking-level",
            ProfileField::Timeout => "timeout",
        };
        match (from, to) {
            (Some(f), Some(t)) if f == t => {}
            (Some(f), Some(t)) => parts.push(format!("{key} {f}→{t}")),
            (Some(f), None) => parts.push(format!("{key} {f}→ cleared")),
            (None, Some(t)) => parts.push(format!("{key}={t}")),
            (None, None) => {}
        }
    }
    parts.join(" ")
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{KnotId, LoomId, StrandPath, TieOffPath};
    use std::path::PathBuf;

    fn loom(s: &str) -> LoomId {
        LoomId(s.to_string())
    }
    fn knot(s: &str) -> KnotId {
        KnotId(s.to_string())
    }
    fn strand(s: &str) -> StrandPath {
        StrandPath(PathBuf::from(s))
    }
    fn tie_off(s: &str) -> TieOffPath {
        TieOffPath(PathBuf::from(s))
    }
    fn ts() -> String {
        "2026-09-07T10:00:00+00:00".into()
    }

    fn rig_state_empty() -> RigState {
        use crate::domain::entities::{RigStateLoom, RigStateProfile};
        RigState {
            rig_path: "/tmp/rig".into(),
            looms: Vec::<RigStateLoom>::new(),
            profiles: Vec::<RigStateProfile>::new(),
            strand_queue: Vec::new(),
            updated_at: ts(),
        }
    }

    // ── LoomEvent lines (all variants) ─────────────────────────────────

    #[test]
    fn loom_started_line() {
        let e = LoomEvent::LoomStarted { loom_id: loom("review-loom"), timestamp: ts() };
        assert_eq!(render_loom_event_line(&e), "LoomStarted loom=review-loom");
    }

    #[test]
    fn loom_stopped_line() {
        let e = LoomEvent::LoomStopped { loom_id: loom("review-loom"), timestamp: ts() };
        assert_eq!(render_loom_event_line(&e), "LoomStopped loom=review-loom");
    }

    #[test]
    fn knot_registered_line() {
        let e = LoomEvent::KnotRegistered { loom_id: loom("review-loom"), knot_id: knot("review"), timestamp: ts() };
        assert_eq!(render_loom_event_line(&e), "KnotRegistered loom=review-loom knot=review");
    }

    #[test]
    fn knot_deregistered_line() {
        let e = LoomEvent::KnotDeregistered { loom_id: loom("review-loom"), knot_id: knot("review"), timestamp: ts() };
        assert_eq!(render_loom_event_line(&e), "KnotDeregistered loom=review-loom knot=review");
    }

    #[test]
    fn knot_processing_line() {
        let e = LoomEvent::KnotProcessing { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "KnotProcessing loom=review-loom knot=review strand=strands/prd.md"
        );
    }

    #[test]
    fn knot_completed_line() {
        let e = LoomEvent::KnotCompleted { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), tie_off_path: tie_off("tie-offs/rig/review-loom/prd.md"), timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "KnotCompleted loom=review-loom knot=review strand=strands/prd.md tie-off=tie-offs/rig/review-loom/prd.md"
        );
    }

    #[test]
    fn knot_failed_line() {
        let e = LoomEvent::KnotFailed { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), error: "boom".into(), timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "KnotFailed loom=review-loom knot=review strand=strands/prd.md error=boom"
        );
    }

    #[test]
    fn strand_processed_success_omits_error() {
        let e = LoomEvent::StrandProcessed { loom_id: loom("review-loom"), strand_path: strand("strands/prd.md"), error: None, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "StrandProcessed loom=review-loom strand=strands/prd.md"
        );
    }

    #[test]
    fn strand_processed_failure_includes_error() {
        let e = LoomEvent::StrandProcessed { loom_id: loom("review-loom"), strand_path: strand("strands/prd.md"), error: Some("kaboom".into()), timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "StrandProcessed loom=review-loom strand=strands/prd.md error=kaboom"
        );
    }

    #[test]
    fn strand_ignored_line() {
        let e = LoomEvent::StrandIgnored { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/bin.dat"), reason: "binary file".into(), timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "StrandIgnored loom=review-loom knot=review strand=strands/bin.dat reason=binary file"
        );
    }

    #[test]
    fn strand_skipped_line() {
        let e = LoomEvent::StrandSkipped { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/vanished.md"), reason: "missing file (unknown pattern)".into(), timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "StrandSkipped loom=review-loom knot=review strand=strands/vanished.md reason=missing file (unknown pattern)"
        );
    }

    #[test]
    fn agent_inactivity_line_with_blocked_call() {
        let e = LoomEvent::AgentInactivity { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), silent_secs: 90, window_secs: 60, blocked_call: Some("bash(\"npm run build\")".into()), attempt: 2, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "AgentInactivity loom=review-loom knot=review strand=strands/prd.md session=sess-1 silent=90 window=60 blocked-call=bash(\"npm run build\") attempt=2"
        );
    }

    #[test]
    fn agent_inactivity_line_omits_missing_blocked_call() {
        let e = LoomEvent::AgentInactivity { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "".into(), silent_secs: 120, window_secs: 60, blocked_call: None, attempt: 1, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "AgentInactivity loom=review-loom knot=review strand=strands/prd.md session= silent=120 window=60 attempt=1"
        );
    }

    #[test]
    fn session_resumed_line() {
        let e = LoomEvent::SessionResumed { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), attempt: 3, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "SessionResumed loom=review-loom knot=review strand=strands/prd.md session=sess-1 attempt=3"
        );
    }

    // ── Plan 089: the interrupted-compact / manual-compact / restart events ──

    #[test]
    fn compaction_interrupted_line() {
        let e = LoomEvent::CompactionInterrupted { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), reason: "overflow".into(), attempt: 1, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "CompactionInterrupted loom=review-loom knot=review strand=strands/prd.md session=sess-1 reason=overflow attempt=1"
        );
    }

    #[test]
    fn manual_compaction_succeeded_line() {
        let e = LoomEvent::ManualCompactionSucceeded { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), tokens_before: 180_000, attempt: 1, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "ManualCompactionSucceeded loom=review-loom knot=review strand=strands/prd.md session=sess-1 tokens-before=180000 attempt=1"
        );
    }

    #[test]
    fn manual_compaction_failed_line() {
        let e = LoomEvent::ManualCompactionFailed { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), error: "still too large".into(), attempt: 1, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "ManualCompactionFailed loom=review-loom knot=review strand=strands/prd.md session=sess-1 error=still too large attempt=1"
        );
    }

    /// Plan 089 (D9): the restart-time record of a run that never finished.
    /// No session id is known, so the field is omitted entirely.
    #[test]
    fn run_abandoned_line_omits_the_unknown_session() {
        let e = LoomEvent::RunAbandoned {
            loom_id: loom("review-loom"),
            knot_id: knot("review"),
            strand_path: strand("strands/prd.md"),
            session_id: None,
            timestamp: ts(),
        };
        assert_eq!(
            render_loom_event_line(&e),
            "RunAbandoned loom=review-loom knot=review strand=strands/prd.md"
        );
    }

    /// Plan 089 (D9): when the session id *is* known (a future caller), the
    /// field is carried.
    #[test]
    fn run_abandoned_line_with_session() {
        let e = LoomEvent::RunAbandoned {
            loom_id: loom("review-loom"),
            knot_id: knot("review"),
            strand_path: strand("strands/prd.md"),
            session_id: Some("sess-dead".into()),
            timestamp: ts(),
        };
        assert_eq!(
            render_loom_event_line(&e),
            "RunAbandoned loom=review-loom knot=review strand=strands/prd.md session=sess-dead"
        );
    }

    #[test]
    fn turn_continued_line() {
        let e = LoomEvent::TurnContinued { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), reason: "threshold".into(), attempt: 1, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "TurnContinued loom=review-loom knot=review strand=strands/prd.md session=sess-1 reason=threshold attempt=1"
        );
    }

    #[test]
    fn session_restarted_line() {
        let e = LoomEvent::SessionRestarted { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), attempt: 1, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "SessionRestarted loom=review-loom knot=review strand=strands/prd.md session=sess-1 attempt=1"
        );
    }

    #[test]
    fn knot_empty_response_line() {
        let e = LoomEvent::KnotEmptyResponse { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), attempt: 1, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "KnotEmptyResponse loom=review-loom knot=review strand=strands/prd.md attempt=1"
        );
    }

    #[test]
    fn context_compacted_line_with_tokens() {
        let e = LoomEvent::ContextCompacted { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), reason: "overflow".into(), tokens_before: Some(180000), attempt: 2, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "ContextCompacted loom=review-loom knot=review strand=strands/prd.md session=sess-1 reason=overflow tokens-before=180000 attempt=2"
        );
    }

    #[test]
    fn context_wrap_up_steered_line() {
        let e = LoomEvent::ContextWrapUpSteered { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), context_tokens: 150_000, limit: 140_000, attempt: 1, mechanism: "steer".into(), timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "ContextWrapUpSteered loom=review-loom knot=review strand=strands/prd.md session=sess-1 context-tokens=150000 limit=140000 attempt=1"
        );
    }

    #[test]
    fn context_compacted_line_omits_absent_tokens() {
        let e = LoomEvent::ContextCompacted { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/prd.md"), session_id: "sess-1".into(), reason: "threshold".into(), tokens_before: None, attempt: 1, timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "ContextCompacted loom=review-loom knot=review strand=strands/prd.md session=sess-1 reason=threshold attempt=1"
        );
    }

    #[test]
    fn events_dispatched_line_multiple_dispatches() {
        let e = LoomEvent::EventsDispatched { loom_id: loom("prd-loom"), knot_id: knot("author"), strand_path: strand("strands/spec.md"), dispatches: vec![
            ("PlanCreated".into(), "write".into(), "docs-loom".into(), "/p/tie-offs/rig/docs-loom/spec.md".into()),
            ("PlanCreated".into(), "critique".into(), "crit-loom".into(), "/p/tie-offs/rig/crit-loom/spec.md".into()),
        ], timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "EventsDispatched loom=prd-loom knot=author strand=strands/spec.md \
             dispatch=PlanCreated→docs-loom/write: /p/tie-offs/rig/docs-loom/spec.md \
             dispatch=PlanCreated→crit-loom/critique: /p/tie-offs/rig/crit-loom/spec.md"
        );
    }

    #[test]
    fn knot_events_missing_line() {
        let e = LoomEvent::KnotEventsMissing { loom_id: loom("prd-loom"), knot_id: knot("author"), strand_path: strand("strands/spec.md"), expected_events: vec!["PlanCreated".into(), "PlanRejected".into()], missing_events: vec!["PlanRejected".into()], timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "KnotEventsMissing loom=prd-loom knot=author strand=strands/spec.md expected=PlanCreated,PlanRejected missing=PlanRejected"
        );
    }

    #[test]
    fn knot_parse_warning_line() {
        let e = LoomEvent::KnotParseWarning { loom_id: loom("review-loom"), knot_file_name: "review.md".into(), message: "unknown property `widgets`".into(), timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "KnotParseWarning loom=review-loom file=review.md message=unknown property `widgets`"
        );
    }

    #[test]
    fn directory_created_line() {
        let e = LoomEvent::DirectoryCreated { loom_id: loom("review-loom"), knot_id: knot("review"), directory: "/p/rig/review-loom/strands".into(), timestamp: ts() };
        assert_eq!(
            render_loom_event_line(&e),
            "DirectoryCreated loom=review-loom knot=review directory=/p/rig/review-loom/strands"
        );
    }

    // ── RigLogEvent lines ──────────────────────────────────────────────

    #[test]
    fn timeout_exceeded_line() {
        let e = RigLogEvent::TimeoutExceeded { loom_id: loom("review-loom"), knot_id: knot("review"), strand_path: strand("strands/big.md"), error: "session exceeded 300s".into(), timestamp: ts() };
        assert_eq!(
            render_rig_event_line(&e),
            "TimeoutExceeded loom=review-loom knot=review strand=strands/big.md error=session exceeded 300s"
        );
    }

    #[test]
    fn queue_idle_line() {
        let e = RigLogEvent::QueueIdle { timestamp: ts() };
        assert_eq!(render_rig_event_line(&e), "QueueIdle");
    }

    // ── [STATE] lines ──────────────────────────────────────────────────

    use crate::domain::entities::{
        RigStateKnot, RigStateLoom, RigStateProfile, RigStateStrandQueueEntry,
    };

    fn state_with_loom() -> RigState {
        let mut s = rig_state_empty();
        s.looms.push(RigStateLoom {
            id: "review-loom".into(),
            knots: vec![RigStateKnot {
                id: "review".into(),
                status: "idle".into(),
                last_strand_path: None,
                last_tie_off_path: None,
                last_error: None,
                last_event_at: None,
            }],
        });
        s.profiles.push(RigStateProfile {
            name: "fast".into(),
            model_ref: Some("fast".into()),
            provider: Some("test".into()),
            model: Some("o3".into()),
            thinking_level: None,
            timeout: None,
        });
        s
    }

    #[test]
    fn first_write_is_baseline_line() {
        let change = StateChange::default();
        let lines = render_state_write_lines(true, &change, &state_with_loom());
        assert_eq!(lines, vec!["[KNOT][STATE] initial snapshot looms=1 knots=1 profiles=1 queue=0"]);
    }

    #[test]
    fn no_op_diff_produces_no_lines() {
        let change = StateChange::default();
        let lines = render_state_write_lines(false, &change, &state_with_loom());
        assert!(lines.is_empty());
    }

    #[test]
    fn knot_status_change_line() {
        use crate::domain::state_change::KnotUpdate;
        let mut change = StateChange::default();
        change.knot_updates.push(KnotUpdate {
            loom: "review-loom".into(),
            knot: "review".into(),
            field_changes: vec![
                (
                    crate::domain::state_change::KnotField::Status,
                    Some("idle".into()),
                    Some("processing".into()),
                ),
                (
                    crate::domain::state_change::KnotField::LastStrandPath,
                    None,
                    Some("strands/prd.md".into()),
                ),
                (
                    crate::domain::state_change::KnotField::LastEventAt,
                    None,
                    Some("2026-09-07T10:00:00+00:00".into()),
                ),
            ],
        });
        let lines = render_state_write_lines(false, &change, &state_with_loom());
        assert_eq!(
            lines,
            vec!["[KNOT][STATE] change knot review-loom/review: status idle→processing strand=strands/prd.md event-at=2026-09-07T10:00:00+00:00"]
        );
    }

    #[test]
    fn knot_tie_off_on_completion_line() {
        use crate::domain::state_change::KnotUpdate;
        let mut change = StateChange::default();
        change.knot_updates.push(KnotUpdate {
            loom: "review-loom".into(),
            knot: "review".into(),
            field_changes: vec![
                (
                    crate::domain::state_change::KnotField::Status,
                    Some("processing".into()),
                    Some("idle".into()),
                ),
                (
                    crate::domain::state_change::KnotField::LastTieOffPath,
                    None,
                    Some("tie-offs/rig/review-loom/prd.md".into()),
                ),
            ],
        });
        let lines = render_state_write_lines(false, &change, &state_with_loom());
        assert_eq!(
            lines,
            vec!["[KNOT][STATE] change knot review-loom/review: status processing→idle tie-off=tie-offs/rig/review-loom/prd.md"]
        );
    }

    #[test]
    fn knot_error_cleared_line() {
        use crate::domain::state_change::KnotUpdate;
        let mut change = StateChange::default();
        change.knot_updates.push(KnotUpdate {
            loom: "review-loom".into(),
            knot: "review".into(),
            field_changes: vec![
                (
                    crate::domain::state_change::KnotField::Status,
                    Some("error".into()),
                    Some("idle".into()),
                ),
                (
                    crate::domain::state_change::KnotField::LastError,
                    Some("boom".into()),
                    None,
                ),
            ],
        });
        let lines = render_state_write_lines(false, &change, &state_with_loom());
        assert_eq!(
            lines,
            vec!["[KNOT][STATE] change knot review-loom/review: status error→idle error boom→ cleared"]
        );
    }

    #[test]
    fn loom_and_knot_add_remove_lines() {
        let mut change = StateChange::default();
        change.looms_added.push("prds-loom".into());
        change.knots_added.push(("prds-loom".into(), "author".into()));
        change.knots_removed.push(("prds-loom".into(), "stale".into()));
        change.looms_removed.push("old-loom".into());
        // The added loom is present in the snapshot so its knot count is
        // available for the `loom+` line.
        let mut snapshot = state_with_loom();
        snapshot.looms.push(RigStateLoom {
            id: "prds-loom".into(),
            knots: vec![
                RigStateKnot {
                    id: "author".into(),
                    status: "idle".into(),
                    last_strand_path: None,
                    last_tie_off_path: None,
                    last_error: None,
                    last_event_at: None,
                },
                RigStateKnot {
                    id: "critique".into(),
                    status: "idle".into(),
                    last_strand_path: None,
                    last_tie_off_path: None,
                    last_error: None,
                    last_event_at: None,
                },
            ],
        });
        let lines = render_state_write_lines(false, &change, &snapshot);
        assert_eq!(
            lines,
            vec![
                "[KNOT][STATE] change loom+ prds-loom (2 knots)",
                "[KNOT][STATE] change loom- old-loom",
                "[KNOT][STATE] change knot prds-loom/author+ (registered)",
                "[KNOT][STATE] change knot prds-loom/stale- (removed)",
            ]
        );
    }

    #[test]
    fn profile_update_and_add_remove_lines() {
        use crate::domain::state_change::ProfileUpdate;
        let mut change = StateChange::default();
        change.profiles_added.push("slow".into());
        change.profiles_removed.push("deprecated".into());
        change.profile_updates.push(ProfileUpdate {
            name: "fast".into(),
            field_changes: vec![
                (
                    crate::domain::state_change::ProfileField::Model,
                    Some("gpt-4o".into()),
                    Some("o3".into()),
                ),
                (
                    crate::domain::state_change::ProfileField::ThinkingLevel,
                    None,
                    Some("xhigh".into()),
                ),
            ],
        });
        let lines = render_state_write_lines(false, &change, &state_with_loom());
        assert_eq!(
            lines,
            vec![
                "[KNOT][STATE] change profile+ slow (added)",
                "[KNOT][STATE] change profile- deprecated (removed)",
                "[KNOT][STATE] change profile fast: model gpt-4o→o3 thinking-level=xhigh",
            ]
        );
    }

    #[test]
    fn queue_add_and_remove_lines() {
        let mut change = StateChange::default();
        change.queue_added.push(RigStateStrandQueueEntry {
            strand_path: "strands/prd.md".into(),
            loom_id: "review-loom".into(),
            knot_id: "review".into(),
            event_kind: "created".into(),
            queued_at: ts(),
        });
        change.queue_removed.push(RigStateStrandQueueEntry {
            strand_path: "strands/old.md".into(),
            loom_id: "review-loom".into(),
            knot_id: "review".into(),
            event_kind: "modified".into(),
            queued_at: ts(),
        });
        let lines = render_state_write_lines(false, &change, &state_with_loom());
        assert_eq!(
            lines,
            vec![
                "[KNOT][STATE] change queue+ strands/prd.md (created review-loom/review)",
                "[KNOT][STATE] change queue- strands/old.md",
            ]
        );
    }

    #[test]
    fn multiple_changes_one_line_each() {
        use crate::domain::state_change::KnotUpdate;
        let mut change = StateChange::default();
        change.knot_updates.push(KnotUpdate {
            loom: "review-loom".into(),
            knot: "review".into(),
            field_changes: vec![(
                crate::domain::state_change::KnotField::Status,
                Some("idle".into()),
                Some("processing".into()),
            )],
        });
        change.knot_updates.push(KnotUpdate {
            loom: "prds-loom".into(),
            knot: "author".into(),
            field_changes: vec![(
                crate::domain::state_change::KnotField::Status,
                Some("processing".into()),
                Some("idle".into()),
            )],
        });
        let lines = render_state_write_lines(false, &change, &state_with_loom());
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("review-loom/review: status idle→processing"));
        assert!(lines[1].contains("prds-loom/author: status processing→idle"));
    }
}
