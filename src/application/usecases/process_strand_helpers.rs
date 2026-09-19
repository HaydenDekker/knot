//! Helper functions for `ProcessStrand::execute()`.
//!
//! Extracted from `ProcessStrand` impl to reduce the size of
//! `process_strand.rs` and improve modularity.

use crate::application::usecases::context_providers::{AgentEventsContextProvider, ContextProvider};
use crate::application::usecases::process_strand::ProcessStrand;
use crate::application::usecases::strand_event_metadata::{extract_expected_event_ids, extract_event_metadata};
use crate::application::ports::{KnotEventType, PortError};
use crate::application::session_resume;
use crate::domain::entities::{Knot, KnotId, LoomId, StrandPath, TieOff, TieOffOutcome, TieOffPath};
use crate::domain::events::BuildContext;
use crate::domain::pending_event::PendingEventId;

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
    /// Plan 086: the incoming continuations count from the strand file's
    /// front-matter (0 for a fresh dispatch, N for a continuation hop).
    pub incoming_continuations: u32,
    /// Plan 087: the incoming `budget-secs` from the strand file's
    /// front-matter — the batch's remaining *execution* budget in
    /// seconds (queue wait is exempt; only execution decrements it).
    /// `None` for a fresh dispatch.
    pub budget_secs: Option<u64>,
    /// Plan 087: the inherited `batch-start-epoch` (Unix epoch seconds)
    /// — the batch's first handoff. `None` for a fresh dispatch.
    pub batch_start_epoch: Option<u64>,
    /// Plan 087: this hop's dequeue instant (Unix epoch seconds) — the
    /// start of the hop's execution window. `execution_secs` at handoff
    /// is `now − dequeue_epoch` (wall-clock, including Knot overhead),
    /// the only thing that decrements the budget.
    pub dequeue_epoch: Option<u64>,
    /// Plan 086: whether a `ContextWrapUpSteered` fired this session
    /// (the water-mark note was delivered). Used for the handoff-missed
    /// enforcement.
    pub was_watermarked: bool,
}

/// Construct a `TieOff` from execution outcome and write it.
///
/// Skipped for timeout outcomes (tie-off preserved unchanged).
///
/// `session_id` is the pi session captured for this execution (from
/// output metadata on success, from the `PortError` on failure); recorded
/// on the tie-off so a reviewer can resume the exact session (plan 071).
pub fn write_tie_off(
    ps: &ProcessStrand,
    outcome: &TieOffOutcome,
    knot: &Knot,
    tie_off_path: &TieOffPath,
    strand_path: &StrandPath,
    event_label: &str,
    session_id: &Option<String>,
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
        session_id: session_id.clone(),
    };
    let _ = ps.tie_off_sink.append(tie_off);
}

/// Handle non-success outcome: write KnotFailed + StrandProcessed logs.
///
/// Late removal: the queued event file (when `event_id` is `Some`) is
/// removed at the point of failure — consume-on-failure, no poison-pill
/// retry loops. The removal happens exactly once on every return, even
/// when a log append fails.
pub fn handle_failure(
    ps: &ProcessStrand,
    outcome: &TieOffOutcome,
    strand_kind: &str,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_path: &StrandPath,
    event_id: Option<&PendingEventId>,
) -> Result<(), PortError> {
    use crate::adapters::logging;
    use crate::domain::events::LoomEvent;
    use crate::application::usecases::system_event_emitter::EventScope;
    use crate::application::usecases::types::format_timestamp;

    let error_msg = outcome
        .error_message()
        .map(|s| s.to_string())
        .unwrap_or_default();

    let result: Result<(), PortError> = (|| {
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

        Ok(())
    })();

    // System events (plan 082) — the run's terminal failure. Emitted
    // regardless of whether the log appends above succeeded, mirroring the
    // loom-log records. Self-exclusion prevents the failing knot from
    // re-triggering on its own failure.
    let fail_scope =
        EventScope::knot(loom_id.clone(), knot_id.clone(), strand_path);
    ps.emit_system(
        &fail_scope,
        "KnotFailed",
        ProcessStrand::run_payload(strand_path, &[
            ("error", Some(error_msg.clone())),
        ]),
        Some(format!("Knot '{}' failed: {}", knot_id.0, error_msg)),
    );
    ps.emit_system(
        &fail_scope,
        "StrandProcessed",
        ProcessStrand::run_payload(strand_path, &[
            ("error", Some(error_msg.clone())),
        ]),
        Some(format!("Knot '{}' processed (failed)", knot_id.0)),
    );

    // Consume the event at the point of failure — exactly once, even
    // when a log append failed above.
    ps.remove_pending_event(event_id);

    if result.is_ok() {
        logging::log_strand_event(
            &format!("{} failed (knot={}): {}", strand_kind, knot_id.0, error_msg),
            &strand_path.0,
        );
    }

    result
}

/// Plan 084: warn (once per process) that a `ctx-wrap-up-limit` is set but
/// the active adapter cannot steer mid-session (only `pi-rpc` samples
/// `get_session_stats` and sends a `steer`). Emitted via the config log so
/// it lands in the service log greppable as `[KNOT][CONFIG]`;
/// `STEER_UNSUPPORTED_WARNED` keeps it to a single line per service
/// lifetime regardless of how many strands trip it.
static STEER_UNSUPPORTED_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn warn_once_steer_unsupported(runner_type: &str) {
    use std::sync::atomic::Ordering;
    if STEER_UNSUPPORTED_WARNED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        crate::adapters::logging::log_config_event(
            "warn-adapter-cannot-steer",
            &format!(
                "ctx-wrap-up-limit is set but the active agent adapter '{runner_type}' cannot steer mid-session (only `pi-rpc` samples context and steers); the limit will not be enforced for this strand",
            ),
        );
    }
}

/// Resolve agent config, build prompt, execute agent, derive outcome.
///
/// Covers: profile resolution, deleted-event history, prompt building,
/// listener context, agent execution, and outcome derivation.
///
/// Returns a `ResolvedExecution` with the outcome, session ID, listener
/// context, and all knots for downstream event enforcement.
pub fn resolve_config_and_build(
    ps: &ProcessStrand,
    knot: &Knot,
    event_type: KnotEventType,
    event_label: String,
    strand_path: &StrandPath,
    loom_id: &LoomId,
    tie_off_path: &TieOffPath,
) -> Result<ResolvedExecution, PortError> {
    // Resolve effective agent config (profile).
    let (agent_config, mut effective_timeout, profile) =
        ps.resolve_agent_config(knot)?;

    // Plan 087: continuation file detection — read the strand file's
    // front-matter for the batch stamps (`budget-secs`,
    // `batch-start-epoch`, `continuations`). When a budget is present,
    // the timeout is the stamped remaining *execution* budget applied in
    // full — queue wait never erodes it (plan 087 Bound 2).
    let (incoming_continuations, budget_secs, batch_start_epoch) =
        read_continuation_stamps(strand_path);
    // Plan 087: capture this hop's dequeue instant — the start of the
    // hop's execution window. At handoff, `execution_secs =
    // now − dequeue_epoch` is the only amount that decrements the batch
    // budget (queue wait is exempt).
    let dequeue_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if let Some(budget) = budget_secs {
        if budget < crate::application::session_resume::MIN_REMAINING_SECS {
            // Plan 087: budget exhausted — do not spawn a session.
            // Write a Knot-authored degenerate terminal tie-off
            // (deferral), record `BatchIncomplete` (reason `deadline`),
            // and let the normal late-removal consume the event.
            return degenerate_deadline_tieoff(
                ps,
                knot,
                loom_id,
                strand_path,
                tie_off_path,
                incoming_continuations,
                budget_secs,
                batch_start_epoch,
                dequeue_epoch,
            );
        }
        effective_timeout = Some(std::time::Duration::from_secs(budget));
    }

    // Plan 084: a wrap-up limit is only meaningful under `pi-rpc` (the sole
    // adapter that samples context and can steer). Surface a one-shot warning
    // if the profile set the limit but the active adapter cannot act on it.
    if agent_config.ctx_wrap_up_limit.is_some()
        && ps.agent_runner.runner_type() != "pi-rpc"
    {
        warn_once_steer_unsupported(ps.agent_runner.runner_type());
    }

    // For Deleted events: read existing tie-off content and extract
    // scoped strand history (last 5 entries for this strand).
    let is_deleted = matches!(event_type, KnotEventType::Deleted);
    let strand_history = if is_deleted {
        let tie_off_content = ps
            .tie_off_sink
            .read_content(tie_off_path)
            .unwrap_or_default();
        let strand_path_str =
            strand_path.0.to_string_lossy().to_string();
        let sections =
            crate::domain::tieoff_parser::extract_last_n(
                &tie_off_content,
                &strand_path_str,
                5,
            );
        if sections.is_empty() {
            None
        } else {
            Some(sections)
        }
    } else {
        None
    };

    // Strand filename — used in prompt for Deleted events.
    let strand_filename = strand_path.0
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    // Build the prompt. For Deleted events, use domain method
    // Knot::deleted_prompt() which composes the deletion notice
    // and scoped strand history.
    let base_prompt = if is_deleted {
        let sections = strand_history
            .as_deref()
            .unwrap_or_default();
        knot.deleted_prompt(&strand_filename, sections)
    } else {
        knot.prompt_template.instructions.clone()
    };

    // Build listener context (per-invocation, not cached).
    // Scans all knots' strand_source entries and injects event
    // instructions at the beginning of the prompt, including
    // pending events from the dispatch directory.
    let all_knots = ProcessStrand::collect_all_knots(&ps.store);
    // Plan 086: the `TasksIncomplete` self-continuation contract is NOT
    // placed in the base prompt. It is delivered at the water-mark trigger
    // (the self-contained `HANDOFF_NOTE` steer), so the first hop keeps its
    // context headroom and is not nudged into a spurious declaration. All
    // *other* subscriber events still inject at the beginning.
    let self_continuation_desc: Option<String> = None;
    let build_ctx = BuildContext {
        knot: knot.clone(),
        loom_id: loom_id.clone(),
        all_knots: all_knots.clone(),
        rig_dir: ps.rig_dir.clone(),
        strand_queue: ps.strand_queue.clone(),
        self_continuation_desc,
    };
    let listener_context =
        AgentEventsContextProvider.build_context(&build_ctx);

    let prompt = if listener_context.is_empty() {
        base_prompt
    } else {
        format!("{}\n\n{}", listener_context, base_prompt)
    };

    // Execute agent with session-resume retry logic.
    let strand_file_ref = if is_deleted {
        None
    } else {
        Some(strand_path.clone())
    };
    let mut session_id: Option<String> = None;
    let result = session_resume::execute_with_resume(
        &*ps.agent_runner,
        &ps.log_port,
        loom_id,
        &knot.id,
        strand_path,
        &mut session_id,
        agent_config,
        prompt,
        strand_file_ref,
        profile.profile_prompt,
        event_label,
        Some(knot.id.0.clone()),
        effective_timeout.clone(),
        ps.system_emitter.as_deref(),
    );

    // Plan 086: whether the water-mark note was delivered this session.
    let was_watermarked = matches!(&result, Ok(output) if output.metadata.as_ref().and_then(|m| m.wrap_up.as_ref()).is_some());

    // Derive outcome from execution result — domain rule.
    let outcome = TieOffOutcome::derive(result);

    Ok(ResolvedExecution {
        outcome,
        session_id,
        listener_context,
        all_knots,
        profile_timeout: effective_timeout,
        incoming_continuations,
        budget_secs,
        batch_start_epoch,
        dequeue_epoch: Some(dequeue_epoch),
        was_watermarked,
    })
}

/// Handle success outcome: event dispatch, KnotCompleted, StrandProcessed,
/// event enforcement, late removal, git commit, and completion logging.
///
/// Late removal: when `event_id` is `Some`, the queued event file is
/// removed exactly once on every return, and on success the removal is
/// the **last step before the git commit** — dispatch, `KnotCompleted`,
/// `StrandProcessed`, and event enforcement all happen first, and the
/// commit captures everything, including the removal. The removal is
/// unconditional on success (even when the knot is not git-versioned or
/// has no commit content).
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
    event_id: Option<&PendingEventId>,
) -> Result<(), PortError> {
    use crate::adapters::logging;
    use crate::application::session_resume;
    use crate::application::usecases::system_event_emitter::EventScope;
    use crate::application::usecases::types::format_timestamp;
    use crate::domain::events::LoomEvent;

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

        // ── Plan 086: Self-continuation dispatch ──────────────────────
        // Parse the tie-off for `TasksIncomplete` events. The first
        // `occurred: true` takes the self-continuation path (dedup
        // guard: at most one continuation per invocation). `occurred:
        // false` is normal completion (no dispatch, no loom event).
        // A missing block is normal for non-water-marked sessions;
        // for water-marked sessions it is the handoff-missed case.
        let all_events =
            crate::domain::tieoff_parser::extract_agent_events(content);
        let ti_events: Vec<&crate::domain::events::AgentEvent> = all_events
            .iter()
            .filter(|e| e.event_id == "TasksIncomplete")
            .collect();

        if let Some(first_ti) = ti_events.first() {
            if first_ti.occurred {
                // occurred: true — self-continuation.
                // Dedup guard: only the first is dispatched.
                if ti_events.len() > 1 {
                    eprintln!(
                        "[task-loop] duplicate TasksIncomplete in knot={} tie-off — dispatching first only (serial-hops guarantee)",
                        knot_id.0,
                    );
                }

                // Plan 087: compute the stamped budget for the
                // continuation — the batch's remaining *execution*
                // budget after this hop. Only this hop's execution
                // (dequeue → handoff, wall-clock) decrements it; queue
                // wait is exempt. Computed here so the loom event +
                // service-log line carry the same budget the
                // continuation file will be stamped with.
                let (stamped_budget, stamped_start) = stamp_continuation_budget(
                    ps,
                    knot,
                    resolved.budget_secs,
                    resolved.batch_start_epoch,
                    resolved.dequeue_epoch,
                );

                // Plan 087 phase 2: pre-check the suppression conditions
                // **before** recording the `TasksIncomplete` loom event +
                // the `[task-loop] handoff` service-log line, so the log
                // never claims a hop that did not happen. Suppression:
                // (a) the max-continuations cap (`next_continuations >
                // MAX_CONTINUATIONS`), (b) an exhausted batch execution
                // budget (`stamped_budget < MIN_REMAINING_SECS` — the
                // next hop would be a degenerate deferral).
                let next_continuations = resolved.incoming_continuations + 1;
                let suppressed_reason = if next_continuations
                    > session_resume::MAX_CONTINUATIONS
                {
                    Some("caps")
                } else if stamped_budget < session_resume::MIN_REMAINING_SECS {
                    Some("deadline")
                } else {
                    None
                };

                if let Some(reason) = suppressed_reason {
                    // Suppressed: record `BatchIncomplete` and skip the
                    // `TasksIncomplete` event + the `[task-loop] handoff`
                    // line + the continuation file. The work is not lost
                    // (checklist + commits are durable); the batch resumes
                    // on the next dispatch with a fresh budget.
                    let _ = ps.log_port.append(LoomEvent::BatchIncomplete {
                        loom_id: loom_id.clone(),
                        knot_id: knot_id.clone(),
                        strand_path: strand_path.clone(),
                        reason: reason.to_string(),
                        continuations: resolved.incoming_continuations,
                        budget_secs: Some(stamped_budget),
                        batch_start_epoch: Some(stamped_start),
                        timestamp: crate::application::usecases::types::format_timestamp(),
                    });
                } else {
                    // Not suppressed — the hop happens. Record
                    // `LoomEvent::TasksIncomplete` (before the dispatch,
                    // as before), then dispatch, then the service-log line.
                    let reason = if resolved.was_watermarked {
                        "water-mark".to_string()
                    } else {
                        "voluntary".to_string()
                    };
                    let _ = ps.log_port.append(LoomEvent::TasksIncomplete {
                        loom_id: loom_id.clone(),
                        knot_id: knot_id.clone(),
                        strand_path: strand_path.clone(),
                        session_id: resolved.session_id.clone(),
                        continuations: next_continuations,
                        tasks_done: first_ti
                            .payload
                            .get("tasks-done")
                            .and_then(|v| v.parse::<u32>().ok()),
                        tasks_remaining: first_ti
                            .payload
                            .get("tasks-remaining")
                            .and_then(|v| v.parse::<u32>().ok()),
                        budget_secs: Some(stamped_budget),
                        batch_start_epoch: Some(stamped_start),
                        reason: reason.clone(),
                        timestamp: crate::application::usecases::types::format_timestamp(),
                    });

                    // Dispatch the self-continuation (best-effort).
                    let _ = dispatch_self_continuation(
                        ps,
                        knot,
                        loom_id,
                        strand_path,
                        first_ti,
                        resolved.incoming_continuations,
                        stamped_budget,
                        stamped_start,
                    );

                    // Service log line (plan 087: the budget is the
                    // stamped remaining execution budget; the hop is
                    // `N/MAX_CONTINUATIONS`; `source` tags the delivery
                    // path so the two are greppable).
                    let source = match &knot.strand_source {
                        crate::domain::value_objects::StrandSource::Filesystem(_)
                        => "filesystem",
                        crate::domain::value_objects::StrandSource::EventUri {
                            ..
                        } => "event",
                    };
                    crate::adapters::logging::log_strand_event(
                        &format!(
                            "[task-loop] handoff (knot={}, hop={}/{}, remaining={}s, trigger={reason}, source={source})",
                            knot_id.0,
                            next_continuations,
                            crate::application::session_resume::MAX_CONTINUATIONS,
                            stamped_budget,
                        ),
                        &strand_path.0,
                    );
                }
            }
            // occurred: false — normal completion, no dispatch, no
            // loom event. The tie-off itself is the explicit
            // "batch complete" declaration.
        } else if resolved.was_watermarked {
            // Water-marked session missing the TasksIncomplete block —
            // handoff-missed. The existing KnotEventsMissing enforcement
            // below will catch this (expected events include the self
            // entry for water-marked sessions). Log the correlation.
            crate::adapters::logging::log_strand_event(
                &format!(
                    "[task-loop] handoff-missed (knot={}, session={})",
                    knot_id.0,
                    resolved.session_id.as_deref().unwrap_or("unknown"),
                ),
                &strand_path.0,
            );
        }
    }

    // Append KnotCompleted to loom-log.
    // On failure: remove the event (consume-on-failure) and propagate —
    // the commit below is skipped, as before.
    if let Err(err) = ps.log_port.append(LoomEvent::KnotCompleted {
        loom_id: loom_id.clone(),
        knot_id: knot_id.clone(),
        strand_path: strand_path.clone(),
        tie_off_path: tie_off_path.clone(),
        timestamp: format_timestamp(),
    }) {
        ps.remove_pending_event(event_id);
        return Err(err);
    }

    // System events (plan 082) — successful run outcome. KnotCompleted
    // carries the tie-off path; StrandProcessed mirrors the success.
    let done_scope =
        EventScope::knot(loom_id.clone(), knot_id.clone(), strand_path);
    ps.emit_system(
        &done_scope,
        "KnotCompleted",
        ProcessStrand::run_payload(strand_path, &[
            ("tie-off-path", Some(tie_off_path.0.display().to_string())),
        ]),
        Some(format!(
            "Knot '{}' completed ({})",
            knot_id.0,
            tie_off_path.0.display()
        )),
    );

    // Append StrandProcessed to loom-log.
    if let Err(err) = ps.log_port.append(LoomEvent::StrandProcessed {
        loom_id: loom_id.clone(),
        strand_path: strand_path.clone(),
        error: None,
        timestamp: format_timestamp(),
    }) {
        ps.remove_pending_event(event_id);
        return Err(err);
    }
    ps.emit_system(
        &done_scope,
        "StrandProcessed",
        ProcessStrand::run_payload(strand_path, &[]),
        Some(format!("Knot '{}' processed", knot_id.0)),
    );

    // ── Event Enforcement ──────────────────────────────────────────
    // If the agent was instructed to emit events but did not
    // acknowledge all of them, log a KnotEventsMissing and attempt
    // one follow-up (plan 091: per-event completeness; the
    // zero-block case is missing = expected).
    let all_knot_ids: Vec<&str> = resolved
        .all_knots
        .iter()
        .map(|k| k.id.0.as_str())
        .collect();
    if !resolved.listener_context.is_empty() {
        let expected_events = extract_expected_event_ids(
            knot,
            loom_id,
            &resolved.all_knots,
        );
        // Empty expected set: only the TasksIncomplete
        // self-continuation entry was injected — a conditional
        // signal, never enforced (plan 091 D1).
        if !expected_events.is_empty() {
            if let Some(ref content) = outcome.tie_off_content() {
                // Fast path: zero blocks → missing = expected (no
                // full parse needed). Otherwise diff the parsed
                // blocks against the expected set.
                let missing_events =
                    if crate::domain::tieoff_parser::has_no_events(content) {
                        expected_events.clone()
                    } else {
                        crate::domain::tieoff_parser::missing_event_ids(
                            &expected_events,
                            content,
                        )
                    };
                if !missing_events.is_empty() {
                    // Log the first KnotEventsMissing
                    let _ = ps.log_port.append(
                        LoomEvent::KnotEventsMissing {
                            loom_id: loom_id.clone(),
                            knot_id: knot_id.clone(),
                            strand_path: strand_path.clone(),
                            expected_events: expected_events.clone(),
                            missing_events: missing_events.clone(),
                            timestamp: format_timestamp(),
                        },
                    );
                    ps.emit_system(
                        &EventScope::knot(
                            loom_id.clone(),
                            knot_id.clone(),
                            strand_path,
                        ),
                        "KnotEventsMissing",
                        ProcessStrand::run_payload(strand_path, &[
                            (
                                "expected-events",
                                Some(expected_events.join(", ")),
                            ),
                            (
                                "missing-events",
                                Some(missing_events.join(", ")),
                            ),
                        ]),
                        Some(format!(
                            "Knot '{}' completed but did not acknowledge all expected events (missing: {})",
                            knot_id.0,
                            missing_events.join(", ")
                        )),
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
                                missing_events.clone(),
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

                                // Dispatch whatever the follow-up emitted
                                // (dispatch failures are non-fatal).
                                // `occurred: false` acknowledgements count
                                // here (agent responded) but are filtered
                                // inside dispatch_events_to_consumers.
                                if !followup_events.is_empty() {
                                    let _ = ps.dispatch_events_to_consumers(
                                        &followup_events,
                                        knot,
                                        loom_id,
                                        &all_knot_ids,
                                    );
                                }

                                // Recompute what is still missing: the
                                // follow-up was asked to emit blocks for
                                // the missing set only, so diff its blocks
                                // against that set (an empty follow-up
                                // leaves everything missing; a follow-up
                                // with only unrelated blocks still leaves
                                // the expected set uncovered — plan 091).
                                let still_missing =
                                    crate::domain::tieoff_parser::
                                        missing_event_ids(
                                            &missing_events,
                                            &response,
                                        );
                                if !still_missing.is_empty() {
                                    // Still missing after follow-up —
                                    // log again (max one retry)
                                    let _ = ps.log_port.append(
                                        LoomEvent::KnotEventsMissing {
                                            loom_id: loom_id.clone(),
                                            knot_id: knot_id.clone(),
                                            strand_path: strand_path
                                                .clone(),
                                            expected_events: expected_events
                                                .clone(),
                                            missing_events: still_missing
                                                .clone(),
                                            timestamp: format_timestamp(),
                                        },
                                    );
                                    ps.emit_system(
                                        &EventScope::knot(
                                            loom_id.clone(),
                                            knot_id.clone(),
                                            strand_path,
                                        ),
                                        "KnotEventsMissing",
                                        ProcessStrand::run_payload(
                                            strand_path,
                                            &[
                                                (
                                                    "expected-events",
                                                    Some(
                                                        expected_events.join(", ")
                                                    ),
                                                ),
                                                (
                                                    "missing-events",
                                                    Some(
                                                        still_missing.join(", ")
                                                    ),
                                                ),
                                            ],
                                        ),
                                        Some(format!(
                                            "Knot '{}' still did not acknowledge all expected events after follow-up (missing: {})",
                                            knot_id.0,
                                            still_missing.join(", ")
                                        )),
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
    }

    // ── Late removal ──────────────────────────────────────────────
    // Remove the event file exactly once, as the last step before the
    // git commit: dispatch, KnotCompleted, StrandProcessed, and event
    // enforcement have all happened. The commit (below) captures
    // everything, including the removal. Unconditional on success.
    ps.remove_pending_event(event_id);

    // Git versioning commit (best-effort, non-fatal).
    // Runs after the removal so the commit captures all artifacts from
    // the turn: tie-off, dispatched events, loom-log entries, and the
    // queue-file removal.
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

// ── Plan 086: Continuation helpers ─────────────────────────────────────────

/// Read the continuation stamps from a strand file's YAML front-matter.
///
/// Returns `(continuations, budget_secs, batch_start_epoch)` — all
/// `0`/`None` for a fresh dispatch (no continuation stamps in the file).
///
/// Plan 087: the batch stamps are `budget-secs` (the batch's remaining
/// *execution* budget in seconds — queue wait never erodes it) and
/// `batch-start-epoch` (the absolute origin — the batch's first
/// handoff). **Compatibility shim (one release):** pre-087 files carry
/// only `batch-deadline-epoch`; when `budget-secs` is absent the budget
/// is derived as `deadline − now` at read time (the 086 wall-clock
/// semantics, where queue wait erodes the budget) and the batch start is
/// `None`.
pub fn read_continuation_stamps(
    strand_path: &StrandPath,
) -> (u32, Option<u64>, Option<u64>) {
    use crate::application::usecases::strand_event_metadata::parse_yaml_frontmatter;
    use std::time::{SystemTime, UNIX_EPOCH};

    let content = match std::fs::read_to_string(&strand_path.0) {
        Ok(c) => c,
        Err(_) => return (0, None, None),
    };
    let frontmatter = match parse_yaml_frontmatter(&content) {
        Some(fm) => fm,
        None => return (0, None, None),
    };

    let continuations = frontmatter
        .get("continuations")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);

    // Plan 087: the stamped budget, with the pre-087 fallback.
    let budget_secs = frontmatter
        .get("budget-secs")
        .and_then(|v| v.parse::<u64>().ok())
        .or_else(|| {
            frontmatter
                .get("batch-deadline-epoch")
                .and_then(|v| v.parse::<u64>().ok())
                .map(|deadline| {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    deadline.saturating_sub(now)
                })
        });
    let batch_start_epoch = frontmatter
        .get("batch-start-epoch")
        .and_then(|v| v.parse::<u64>().ok());

    (continuations, budget_secs, batch_start_epoch)
}

/// Plan 086/087: write a Knot-authored degenerate terminal tie-off when
/// the batch's execution budget is exhausted. This is a deferral, not a
/// failure: "batch execution budget exhausted; remaining work is in the
/// checklist; resume on the next dispatch." Records `BatchIncomplete`
/// (reason `deadline`).
///
/// Returns a `ResolvedExecution` with a `TieOffOutcome::Failed` so the
/// caller can proceed through the normal failure path (late-removal,
/// log, etc.) without spawning an agent session.
#[allow(clippy::too_many_arguments)] // flat stamp list keeps the single call site readable
pub fn degenerate_deadline_tieoff(
    ps: &ProcessStrand,
    knot: &Knot,
    loom_id: &LoomId,
    strand_path: &StrandPath,
    _tie_off_path: &TieOffPath,
    continuations: u32,
    budget_secs: Option<u64>,
    batch_start_epoch: Option<u64>,
    dequeue_epoch: u64,
) -> Result<ResolvedExecution, PortError> {
    use crate::application::usecases::types::format_timestamp;
    use crate::domain::entities::TieOffStatus;
    use crate::domain::events::LoomEvent;

    let knot_id = knot.id.clone();
    let note = format!(
        "Batch execution budget exhausted. Remaining work is in the checklist; resume on the next dispatch.\n\nKnot: {}\nContinuations: {continuations}",
        knot_id.0,
    );

    // Write the degenerate tie-off (Knot-authored, status Failed).
    let tie_off = TieOff {
        content: note.clone(),
        path: _tie_off_path.clone(),
        status: TieOffStatus::Failed,
        knot_name: Some(knot_id.0.clone()),
        event_type: Some("TasksIncomplete".to_string()),
        strand_path: Some(strand_path.0.display().to_string()),
        timestamp: None,
        agent_events: Vec::new(),
        event_metadata: crate::domain::entities::EventMetadata {
            event_id: Some("TasksIncomplete".to_string()),
            source_knot: Some(knot_id.0.clone()),
            original_strand: None,
        },
        session_id: None,
    };
    let _ = ps.tie_off_sink.append(tie_off);

    // Record BatchIncomplete (reason deadline) with the batch's budget
    // state (plan 087: remaining execution budget + batch origin).
    let _ = ps.log_port.append(LoomEvent::BatchIncomplete {
        loom_id: loom_id.clone(),
        knot_id: knot_id.clone(),
        strand_path: strand_path.clone(),
        reason: "deadline".to_string(),
        continuations,
        budget_secs,
        batch_start_epoch,
        timestamp: format_timestamp(),
    });

    // Return a Failed outcome so the caller takes the failure path
    // (late-removal, log, git commit). The stamps are carried through
    // so the `ResolvedExecution` faithfully reflects the continuation
    // file that was deferred.
    Ok(ResolvedExecution {
        outcome: TieOffOutcome::Failed {
            error: "batch execution budget exhausted (degenerate tie-off)".to_string(),
        },
        session_id: None,
        listener_context: String::new(),
        all_knots: Vec::new(),
        profile_timeout: None,
        incoming_continuations: continuations,
        budget_secs,
        batch_start_epoch,
        dequeue_epoch: Some(dequeue_epoch),
        was_watermarked: false,
    })
}

/// Plan 087 Bound 2: compute the stamped budget + batch start for the
/// continuation event (the batch total execution budget).
///
/// The budget is the batch's **remaining execution duration** in seconds
/// — only execution decrements it; queue wait never does. First handoff
/// (fresh dispatch, no incoming budget): `profile_secs −
/// execution_secs` with `start = now` (the batch's first handoff stamps
/// the origin). Continuation hop: `incoming_budget − execution_secs`
/// with `start` inherited verbatim (never recomputed). `execution_secs`
/// is wall-clock (dequeue → handoff), including Knot overhead.
///
/// The budget only ever *decreases* — it is never reset to a full
/// `profile_timeout` at any handoff ("fresh context, never a fresh
/// budget"). The profile timeout defaults to 300s when unset.
pub fn stamp_continuation_budget(
    ps: &ProcessStrand,
    knot: &Knot,
    incoming_budget: Option<u64>,
    incoming_start: Option<u64>,
    dequeue_epoch: Option<u64>,
) -> (u64, u64) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // This hop's execution: wall-clock from dequeue to handoff, including
    // Knot overhead (locked decision — the runner-metadata variant is a
    // future refinement).
    let execution_secs = now.saturating_sub(dequeue_epoch.unwrap_or(now));

    match incoming_budget {
        Some(budget) => {
            // Continuation hop: decay by this hop's execution; inherit
            // the batch origin.
            (
                budget.saturating_sub(execution_secs),
                incoming_start.unwrap_or(now),
            )
        }
        None => {
            // First handoff: the batch gets one total execution budget —
            // the profile's per-invocation timeout, hop 1's execution
            // already deducted.
            let profile_secs = ps
                .resolve_agent_config(knot)
                .ok()
                .and_then(|(_, timeout, _)| timeout)
                .map(|d| d.as_secs())
                .unwrap_or(300);
            (profile_secs.saturating_sub(execution_secs), now)
        }
    }
}

/// Plan 086/087: dispatch the self-continuation for a `TasksIncomplete`
/// `occurred: true` declaration.
///
/// Creates a continuation event file in the knot's **existing input** —
/// its watched strand dir for a filesystem source, or its event dispatch
/// dir (`derive_runtime_root(rig_dir)/<loom_id>/<event_id>`) for an
/// event source (plan 087 D1) — with the stamps (`budget-secs`,
/// `batch-start-epoch`, `continuations`, accumulated
/// `background-additional`) and the handoff content (accumulated
/// background + handoff body + `next-task-context`). The existing
/// watcher (bound to this `(loom_id, knot_id)` pair) picks the file up
/// like any other dispatched event and re-triggers the knot.
///
/// `budget_secs` + `batch_start_epoch` are the stamped values computed by
/// the caller ([`stamp_continuation_budget`]) so the loom event and the
/// service-log line carry the same budget the file is stamped with.
///
/// Returns the path of the created file, or `None` when the hop is
/// suppressed: the max-continuations cap (records `BatchIncomplete`
/// reason `caps`) or an exhausted batch execution budget (`budget_secs`
/// below `MIN_REMAINING_SECS` — the next hop would be a degenerate
/// deferral; records `BatchIncomplete` reason `deadline`).
#[allow(clippy::too_many_arguments)] // flat stamp list keeps the single call site readable
pub fn dispatch_self_continuation(
    ps: &ProcessStrand,
    knot: &Knot,
    loom_id: &LoomId,
    strand_path: &StrandPath,
    event: &crate::domain::events::AgentEvent,
    incoming_continuations: u32,
    budget_secs: u64,
    batch_start_epoch: u64,
) -> Result<Option<std::path::PathBuf>, PortError> {
    use crate::application::usecases::types::format_timestamp;
    use crate::domain::events::LoomEvent;

    let knot_id = knot.id.clone();
    let next_continuations = incoming_continuations + 1;

    // Plan 086: max-continuations cap — if at the cap, suppress the
    // dispatch and record BatchIncomplete (reason caps) with the budget
    // the suppressed continuation would have carried (plan 087).
    if next_continuations > crate::application::session_resume::MAX_CONTINUATIONS {
        let _ = ps.log_port.append(LoomEvent::BatchIncomplete {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            reason: "caps".to_string(),
            continuations: incoming_continuations,
            budget_secs: Some(budget_secs),
            batch_start_epoch: Some(batch_start_epoch),
            timestamp: format_timestamp(),
        });
        return Ok(None);
    }

    // Plan 087 phase 2: exhausted batch execution budget — the stamped
    // remaining budget fell below `MIN_REMAINING_SECS`, so the next hop
    // would be a degenerate deferral. Suppress the dispatch and record
    // `BatchIncomplete` (reason deadline) instead of writing a
    // continuation that could only be deferred.
    if budget_secs < crate::application::session_resume::MIN_REMAINING_SECS {
        let _ = ps.log_port.append(LoomEvent::BatchIncomplete {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            reason: "deadline".to_string(),
            continuations: incoming_continuations,
            budget_secs: Some(budget_secs),
            batch_start_epoch: Some(batch_start_epoch),
            timestamp: format_timestamp(),
        });
        return Ok(None);
    }

    // Accumulate the background-additional: incoming accumulation +
    // this event's contribution (append-only, hop-labelled).
    let incoming_bg = crate::application::usecases::strand_event_metadata::parse_yaml_frontmatter(
        &std::fs::read_to_string(&strand_path.0).unwrap_or_default(),
    )
    .and_then(|fm| fm.get("background-additional").cloned())
    .unwrap_or_default();
    let new_bg = event.payload.get("background-additional").cloned().unwrap_or_default();
    let accumulated_bg = if incoming_bg.is_empty() {
        format!("[hop {}]\n{}", next_continuations, new_bg)
    } else if new_bg.is_empty() {
        incoming_bg
    } else {
        format!("{}\n[hop {}]\n{}", incoming_bg, next_continuations, new_bg)
    };

    // Build the continuation file content.
    let timestamp = format_timestamp();
    let next_task_context =
        event.payload.get("next-task-context").cloned().unwrap_or_default();
    let handoff_body = event.body.as_deref().unwrap_or("");

    let mut lines = Vec::new();
    lines.push("---".to_string());
    lines.push(format!("event-id: TasksIncomplete"));
    lines.push(format!("target-knot: {}", knot_id.0));
    lines.push(format!("timestamp: {timestamp}"));
    lines.push(format!("continuations: {next_continuations}"));
    lines.push(format!("budget-secs: {budget_secs}"));
    lines.push(format!("batch-start-epoch: {batch_start_epoch}"));
    if !accumulated_bg.is_empty() {
        // Multi-line YAML: use a block scalar
        lines.push("background-additional: |".to_string());
        for bg_line in accumulated_bg.lines() {
            lines.push(format!("  {bg_line}"));
        }
    }
    lines.push("---".to_string());
    lines.push(String::new());

    if !accumulated_bg.is_empty() {
        lines.push("## Accumulated Background".to_string());
        lines.push(String::new());
        lines.push(accumulated_bg);
        lines.push(String::new());
    }

    if !handoff_body.is_empty() {
        lines.push("## Handoff".to_string());
        lines.push(String::new());
        lines.push(handoff_body.to_string());
        lines.push(String::new());
    }

    if !next_task_context.is_empty() {
        lines.push("## Next Task Context".to_string());
        lines.push(String::new());
        lines.push(next_task_context);
    }

    let content = lines.join("\n");

    // Plan 087 (D1): the continuation lands in the knot's *existing
    // input* — the dir the watcher already watches, bound to this
    // `(loom_id, knot_id)` pair:
    // - filesystem source: the knot's own strand dir (v1 behaviour);
    // - event source: the event dispatch dir (the same dir
    //   `FileSystemEventDispatcher::dispatch` writes and
    //   `ensure_event_uri_watch` watches).
    let target_dir = match &knot.strand_source {
        crate::domain::value_objects::StrandSource::Filesystem(path) => {
            path.clone()
        }
        crate::domain::value_objects::StrandSource::EventUri { event_id, .. } =>
            crate::domain::knot_file::derive_runtime_root(&ps.rig_dir)
                .join(&loom_id.0)
                .join(event_id),
    };
    std::fs::create_dir_all(&target_dir).map_err(|e| {
        PortError::EventDispatchFailed(format!(
            "failed to create continuation dir '{}': {e}",
            target_dir.display()
        ))
    })?;

    let filename = format!("event-{timestamp}-continuation.md");
    let file_path = target_dir.join(filename);
    std::fs::write(&file_path, content).map_err(|e| {
        PortError::EventDispatchFailed(format!(
            "failed to write continuation file '{}': {e}",
            file_path.display()
        ))
    })?;

    Ok(Some(file_path))
}

// ── Plan 087: tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::AgentRunner;
    use crate::application::usecases::test_fixtures::{
        build_knot_with_profile, build_loom, MockAgentRunner, MockEventDispatcher,
        MockGitVersioningPort, MockLoomLogPort, MockModelRegistry,
        MockProfileRepository, MockRigLogPort, MockStrandFileChecker,
        MockTieOffSink,
    };
    use crate::application::store::LoomStore;
    use crate::domain::events::LoomEvent;
    use crate::domain::value_objects::{AgentProfile, RigAgentConfig};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Write a continuation-file body into `dir` and return its path.
    fn write_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).expect("write test strand file");
        path
    }

    fn build_process_strand(
        profile: AgentProfile,
    ) -> (ProcessStrand, Arc<Mutex<Vec<LoomEvent>>>) {
        build_process_strand_at(PathBuf::from("/rig"), profile)
    }

    /// Same as `build_process_strand` but with an explicit `rig_dir`
    /// (plan 087 phase 1: the event-source continuation test needs a
    /// writable runtime root under the rig dir).
    fn build_process_strand_at(
        rig_dir: PathBuf,
        profile: AgentProfile,
    ) -> (ProcessStrand, Arc<Mutex<Vec<LoomEvent>>>) {
        let store = LoomStore::new();
        store.register(build_loom(
            "test-loom",
            vec![build_knot_with_profile("k1", "budgeted")],
        ));

        let (log_port, log_events) = MockLoomLogPort::new();
        let (rig_log, _rig_events) = MockRigLogPort::new();
        let use_case = ProcessStrand::new(
            store,
            Arc::new(log_port),
            Arc::new(MockAgentRunner::default()) as Arc<dyn AgentRunner>,
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            rig_dir,
            Arc::new(MockProfileRepository {
                profiles: Arc::new(Mutex::new(HashMap::from_iter([(
                    "budgeted".to_string(),
                    profile,
                )]))),
            }),
            Arc::new(MockModelRegistry::default()),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );
        (use_case, log_events)
    }

    /// The knot registered by `build_process_strand`.
    fn budgeted_knot() -> Knot {
        build_knot_with_profile("k1", "budgeted")
    }

    // ── read_continuation_stamps (plan 087) ──────────────────────────

    /// The 087 stamps (`budget-secs` + `batch-start-epoch`) resolve to the
    /// `(continuations, budget_secs, batch_start_epoch)` triple.
    #[test]
    fn read_continuation_stamps_reads_budget_and_start() {
        let dir = TempDir::new().unwrap();
        let t0 = now_secs() - 3600;
        let path = write_file(
            dir.path(),
            "event-20260101T000000-continuation.md",
            &format!(
                "---\nevent-id: TasksIncomplete\nbatch-start-epoch: {t0}\nbudget-secs: 1200\ncontinuations: 2\n---\n\n## Handoff\nbody",
            ),
        );
        let (continuations, budget, start) =
            read_continuation_stamps(&StrandPath(path));
        assert_eq!(continuations, 2);
        assert_eq!(budget, Some(1200));
        assert_eq!(start, Some(t0));
    }

    /// Compatibility shim: a pre-087 file with only `batch-deadline-epoch`
    /// still resolves a budget (deadline − now, at read time) and no batch
    /// start.
    #[test]
    fn read_continuation_stamps_shim_deadline_only() {
        let dir = TempDir::new().unwrap();
        let deadline = now_secs() + 1200;
        let path = write_file(
            dir.path(),
            "event-20260101T000000-continuation.md",
            &format!(
                "---\nevent-id: TasksIncomplete\nbatch-deadline-epoch: {deadline}\ncontinuations: 1\n---\n\n## Handoff\nbody",
            ),
        );
        let (continuations, budget, start) =
            read_continuation_stamps(&StrandPath(path));
        assert_eq!(continuations, 1);
        let b = budget.expect("shim must resolve a budget from the deadline");
        assert!(
            (1195..=1200).contains(&b),
            "budget should be deadline−now (~1200), got {b}"
        );
        assert_eq!(start, None, "pre-087 files carry no batch start");
    }

    /// A fresh dispatch (no continuation stamps) resolves to
    /// `(0, None, None)`.
    #[test]
    fn read_continuation_stamps_fresh_dispatch() {
        let dir = TempDir::new().unwrap();
        let path = write_file(dir.path(), "strand.md", "plain strand content");
        let (continuations, budget, start) =
            read_continuation_stamps(&StrandPath(path));
        assert_eq!(continuations, 0);
        assert_eq!(budget, None);
        assert_eq!(start, None);
    }

    // ── stamp_continuation_budget (plan 087 Bound 2) ─────────────────

    /// First handoff (no incoming budget): the batch gets one total
    /// execution budget — `profile_secs − execution_secs` — with the batch
    /// start stamped at now.
    #[test]
    fn stamp_continuation_budget_first_handoff() {
        let profile = AgentProfile::new(
            "budgeted".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "Budgeted.".to_string(),
        )
        .unwrap()
        .with_timeout(Some(1800));
        let (ps, _events) = build_process_strand(profile);
        let knot = budgeted_knot();
        let dequeue = now_secs();
        let (budget, start) =
            stamp_continuation_budget(&ps, &knot, None, None, Some(dequeue));
        let after = now_secs();
        // execution_secs ∈ [0, after − dequeue]; budget = 1800 − execution.
        assert!((1799..=1800).contains(&budget), "got budget {budget}");
        assert!(
            (start as i64 - after as i64).abs() <= 1,
            "batch start must be stamped at the first handoff (start={start}, after={after})"
        );
    }

    /// Continuation hop (incoming budget): the budget decays by this hop's
    /// execution only — `incoming_budget − execution_secs` — and the batch
    /// start is inherited verbatim (never recomputed), even when stale.
    #[test]
    fn stamp_continuation_budget_continuation_hop_inherits_start() {
        let (ps, _events) = build_process_strand(
            crate::application::usecases::test_fixtures::default_profile(),
        );
        let knot = budgeted_knot();
        let t0 = now_secs() - 7200; // a stale origin must be inherited
        let dequeue = now_secs();
        let (budget, start) =
            stamp_continuation_budget(&ps, &knot, Some(1200), Some(t0), Some(dequeue));
        assert!((1199..=1200).contains(&budget), "got budget {budget}");
        assert_eq!(start, t0, "batch start must be inherited, never recomputed");
    }

    /// The budget only ever *decreases* by execution — a long queue wait
    /// (stale `batch-start-epoch`, full `budget-secs`) does not erode the
    /// stamped budget.
    #[test]
    fn stamp_continuation_budget_queue_wait_exempt() {
        let (ps, _events) = build_process_strand(
            crate::application::usecases::test_fixtures::default_profile(),
        );
        let knot = budgeted_knot();
        // The continuation sat in the queue for 24h; its budget is intact.
        let t0 = now_secs() - 86_400;
        let dequeue = now_secs();
        let (budget, start) =
            stamp_continuation_budget(&ps, &knot, Some(1200), Some(t0), Some(dequeue));
        assert!((1199..=1200).contains(&budget), "queue wait must not erode the budget");
        assert_eq!(start, t0);
    }

    // ── dispatch_self_continuation delivery (plan 087 phase 1, D1) ────

    /// The `TasksIncomplete` event a `occurred: true` tie-off declares.
    fn tasks_incomplete_event() -> crate::domain::events::AgentEvent {
        let mut payload = HashMap::new();
        payload.insert("tasks-done".to_string(), "3".to_string());
        payload.insert("tasks-remaining".to_string(), "2".to_string());
        payload.insert(
            "next-task-context".to_string(),
            "Resume at checklist task 4.".to_string(),
        );
        crate::domain::events::AgentEvent {
            event_id: "TasksIncomplete".to_string(),
            occurred: true,
            payload,
            body: Some("Checklist is at tasks/checklist.md.".to_string()),
        }
    }

    /// **Event-source** delivery (the v1 gap this phase closes): the
    /// continuation lands in the knot's event dispatch dir —
    /// `derive_runtime_root(rig_dir)/<loom_id>/<event_id>/` — and carries
    /// the stamped budget + batch origin. The existing watcher (bound to
    /// this `(loom, knot)` pair) picks the file up like any other
    /// dispatched event.
    #[test]
    fn dispatch_self_continuation_event_source_lands_in_event_dir() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir_all(&rig_dir).unwrap();
        let (ps, _events) = build_process_strand_at(
            rig_dir.clone(),
            crate::application::usecases::test_fixtures::default_profile(),
        );

        let knot = Knot {
            id: KnotId("k1".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: crate::domain::value_objects::PromptTemplate {
                instructions: "React to events.".to_string(),
            },
            git_versioned: true,
            strand_source: crate::domain::value_objects::StrandSource::EventUri {
                producer_knot: "producer".to_string(),
                event_id: "SomeEvent".to_string(),
            },
            event_description: Some("When SomeEvent occurs.".to_string()),
        };
        let loom_id = LoomId("test-loom".to_string());
        let strand_path = StrandPath(write_file(
            dir.path(),
            "event-20260101T000000-SomeEvent.md",
            "event file body",
        ));
        let t0 = now_secs() - 60;

        let result = dispatch_self_continuation(
            &ps,
            &knot,
            &loom_id,
            &strand_path,
            &tasks_incomplete_event(),
            0,
            1200,
            t0,
        )
        .expect("dispatch must not fail");
        let path = result.expect(
            "an event-source knot whose work remains must get a continuation (v1 gap)",
        );

        let expected_dir =
            crate::domain::knot_file::derive_runtime_root(&rig_dir)
                .join("test-loom")
                .join("SomeEvent");
        assert!(
            path.starts_with(&expected_dir),
            "continuation must land in the event dispatch dir {}: got {}",
            expected_dir.display(),
            path.display()
        );

        // The file carries the stamped budget + batch origin (087 stamps).
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("event-id: TasksIncomplete"));
        assert!(content.contains("target-knot: k1"));
        assert!(content.contains("continuations: 1"));
        assert!(content.contains("budget-secs: 1200"));
        assert!(content.contains(&format!("batch-start-epoch: {t0}")));
        assert!(content.contains("## Next Task Context"));
    }

    /// **Filesystem** delivery (preserved from v1): the continuation still
    /// lands in the knot's own watched strand dir.
    #[test]
    fn dispatch_self_continuation_filesystem_lands_in_strand_dir() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir_all(&rig_dir).unwrap();
        let (ps, _events) = build_process_strand_at(
            rig_dir,
            crate::application::usecases::test_fixtures::default_profile(),
        );

        let strand_dir = dir.path().join("strands");
        let knot = crate::application::usecases::test_fixtures::build_knot_with_strand_source(
            "k1",
            strand_dir.clone(),
        );
        let loom_id = LoomId("test-loom".to_string());
        let strand_path = StrandPath(write_file(dir.path(), "strand.md", "plain strand content"));
        let t0 = now_secs() - 60;

        let result = dispatch_self_continuation(
            &ps,
            &knot,
            &loom_id,
            &strand_path,
            &tasks_incomplete_event(),
            0,
            1200,
            t0,
        )
        .expect("dispatch must not fail");
        let path = result.expect("filesystem knots keep their strand-dir continuation");

        assert!(
            path.starts_with(&strand_dir),
            "continuation must land in the knot's own strand dir {}: got {}",
            strand_dir.display(),
            path.display()
        );

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("budget-secs: 1200"));
        assert!(content.contains("batch-start-epoch"));
    }

    /// **Plan 087 p4 (accumulation round-trip)**: the continuation chain
    /// must pass the incoming accumulation **through** and **append** the
    /// agent's new input each hop. Hop 1 writes a continuation whose
    /// `background-additional: |` carries hop 1's facts; hop 2 (fed by that
    /// file, so `incoming_bg` reads it back through the now block-scalar
    /// -aware `parse_yaml_frontmatter`) must produce a continuation whose
    /// `## Accumulated Background` contains **both** hop 1's body **and** the
    /// new `[hop 2]` contribution.
    #[test]
    fn dispatch_self_continuation_accumulates_background_across_hops() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir_all(&rig_dir).unwrap();
        let (ps, _events) = build_process_strand_at(
            rig_dir.clone(),
            crate::application::usecases::test_fixtures::default_profile(),
        );

        let knot = Knot {
            id: KnotId("k1".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: crate::domain::value_objects::PromptTemplate {
                instructions: "React to events.".to_string(),
            },
            git_versioned: true,
            strand_source: crate::domain::value_objects::StrandSource::EventUri {
                producer_knot: "producer".to_string(),
                event_id: "SomeEvent".to_string(),
            },
            event_description: Some("When SomeEvent occurs.".to_string()),
        };
        let loom_id = LoomId("test-loom".to_string());
        let t0 = now_secs() - 120;

        // An initial (non-continuation) trigger file for hop 1.
        let trigger = StrandPath(write_file(
            dir.path(),
            "event-20260101T000000-SomeEvent.md",
            "event file body",
        ));

        // Hop 1: the agent's tie-off contributes new facts (hop 1).
        let mut hop1 = tasks_incomplete_event();
        hop1.payload.insert(
            "background-additional".to_string(),
            "Only the data layer changed; app wiring untouched.".to_string(),
        );

        let hop1_path = dispatch_self_continuation(
            &ps,
            &knot,
            &loom_id,
            &trigger,
            &hop1,
            0,
            1200,
            t0,
        )
        .expect("hop 1 must not fail")
        .expect("hop 1 must get a continuation");

        // Hop 1's file carries its `[hop 1]` contribution (unindented) in the
        // `## Accumulated Background` body — the pass-through source for hop 2.
        let hop1_content = std::fs::read_to_string(&hop1_path).unwrap();
        assert!(
            hop1_content
                .contains("[hop 1]\nOnly the data layer changed; app wiring untouched."),
            "hop 1 contribution must be written: {}",
            hop1_content
        );

        // Hop 2: fed by hop 1's file (so `incoming_bg` reads it back), the
        // agent contributes new facts (hop 2).
        let mut hop2 = tasks_incomplete_event();
        hop2.payload.insert(
            "background-additional".to_string(),
            "test:unit and test:components are green.".to_string(),
        );

        let hop2_path = dispatch_self_continuation(
            &ps,
            &knot,
            &loom_id,
            &StrandPath(hop1_path.clone()),
            &hop2,
            1,
            1169,
            t0,
        )
        .expect("hop 2 must not fail")
        .expect("hop 2 must get a continuation");

        // The invariant: hop 2's accumulated background carries hop 1's
        // pass-through body AND the new [hop 2] contribution, in order.
        let hop2_content = std::fs::read_to_string(&hop2_path).unwrap();
        assert!(
            hop2_content
                .contains("[hop 1]\nOnly the data layer changed; app wiring untouched."),
            "hop 1 pass-through must survive into hop 2: {}",
            hop2_content
        );
        assert!(
            hop2_content.contains("[hop 2]\ntest:unit and test:components are green."),
            "hop 2 append must be present: {}",
            hop2_content
        );
        let h1 = hop2_content.find("[hop 1]").expect("hop 1 label");
        let h2 = hop2_content.find("[hop 2]").expect("hop 2 label");
        assert!(h1 < h2, "hop 1 must precede hop 2");
    }

    // ── FIFO / D3 ordering (plan 087 phase 3) ─────────────────────────

    /// **FIFO (D3 ordering)**: a continuation written to the knot's inbox
    /// is queued **after** any already-queued events (appended to the end,
    /// never front-inserted). The continuation enters the durable queue
    /// through the existing watcher → debounce → `push_or_replace` path, so
    /// it is pushed after the events that were already queued. The head of
    /// the queue is still the oldest already-queued event; the continuation
    /// is popped last.
    #[test]
    fn continuation_written_to_inbox_is_queued_after_already_queued_events() {
        use crate::adapters::outbound::disk_event_queue::DiskBackedEventQueue;
        use crate::application::ports::StrandEventQueue;
        use crate::domain::pending_event::{
            PendingEvent, PendingEventId, PendingEventOrShutdown,
        };

        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir_all(&rig_dir).unwrap();
        let (ps, _events) = build_process_strand_at(
            rig_dir,
            crate::application::usecases::test_fixtures::default_profile(),
        );

        // An event-source knot — its inbox is the event dispatch dir
        // `derive_runtime_root(rig_dir)/<loom>/<event-id>`.
        let knot = Knot {
            id: KnotId("k1".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: crate::domain::value_objects::PromptTemplate {
                instructions: "React to events.".to_string(),
            },
            git_versioned: true,
            strand_source: crate::domain::value_objects::StrandSource::EventUri {
                producer_knot: "producer".to_string(),
                event_id: "SomeEvent".to_string(),
            },
            event_description: Some("When SomeEvent occurs.".to_string()),
        };
        let loom_id = LoomId("test-loom".to_string());
        let trigger = StrandPath(write_file(
            dir.path(),
            "event-20260101T000000-SomeEvent.md",
            "event file body",
        ));
        let t0 = now_secs() - 60;

        // The handoff writes the continuation into the knot's inbox.
        let cont_path = dispatch_self_continuation(
            &ps,
            &knot,
            &loom_id,
            &trigger,
            &tasks_incomplete_event(),
            0,
            1200,
            t0,
        )
        .expect("dispatch must not fail")
        .expect("an event-source knot whose work remains must get a continuation");

        // The durable queue, seeded with two already-queued events whose
        // 13-digit timestamp prefixes are far earlier than the
        // continuation's (a real, current Unix-millisecond timestamp), so
        // they deterministically sort before it by filename (FIFO).
        let queue = DiskBackedEventQueue::new(dir.path().join("events"));
        queue.push_or_replace(PendingEvent {
            id: PendingEventId("1000000000000-aaaa".to_string()),
            kind: "Created".to_string(),
            loom_id: "test-loom".to_string(),
            knot_id: "k1".to_string(),
            strand_path: "/already/queued/a.md".to_string(),
            queued_at: "2026-01-01T00:00:00Z".to_string(),
        });
        queue.push_or_replace(PendingEvent {
            id: PendingEventId("1500000000000-bbbb".to_string()),
            kind: "Created".to_string(),
            loom_id: "test-loom".to_string(),
            knot_id: "k1".to_string(),
            strand_path: "/already/queued/b.md".to_string(),
            queued_at: "2026-01-01T00:00:01Z".to_string(),
        });

        // Simulate the watcher → debounce push: the continuation file's
        // `StrandEvent::Created` is converted via the real
        // `From<StrandEvent>` (a fresh, later timestamp id) and
        // `push_or_replace`d, exactly as the debounce engine does.
        let cont_pending: PendingEvent =
            crate::domain::events::StrandEvent::Created {
                loom_id: LoomId("test-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
                strand_path: StrandPath(cont_path.clone()),
            }
            .into();
        queue.push_or_replace(cont_pending);

        // The head is still the first already-queued event — the
        // continuation was appended, not front-inserted.
        assert_eq!(
            queue.front().unwrap().strand_path,
            "/already/queued/a.md",
            "the continuation must not be front-inserted ahead of queued events"
        );

        // Pop order: the two already-queued events first, the continuation
        // last (FIFO append-to-end).
        let order: Vec<String> = (0..3)
            .map(|_| match queue.pop() {
                Some(PendingEventOrShutdown::Event(e)) => e.strand_path,
                other => panic!("expected an event, got {other:?}"),
            })
            .collect();
        assert_eq!(order[0], "/already/queued/a.md");
        assert_eq!(order[1], "/already/queued/b.md");
        assert_eq!(
            order[2],
            cont_path.to_string_lossy().to_string(),
            "the continuation must be queued after the already-queued events"
        );
    }
}
