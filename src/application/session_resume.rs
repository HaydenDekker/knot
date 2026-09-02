//! Session-resume retry module.
//!
//! When an agent invocation fails with a resumable error (timeout, mid-stream
//! failure) — or ends its turn abruptly without a final response (plan 078) —
//! and a session ID was captured, this module retries the invocation using
//! `--session-id <id>` to continue the same Pi session. The retry prompt is
//! the original prompt plus the final-response request
//! ([`FINAL_RESPONSE_REQUEST`]). An inactivity kill (plan 081) is the one
//! exception to the session-ID requirement: it restarts even without a
//! captured session (fresh restart — knots are idempotent), and the retry
//! prompt carries the blocking-call note ([`INACTIVITY_RESTART_NOTE`])
//! instead. Retries are limited to 10 attempts or the profile's overall
//! timeout budget, whichever comes first.

use std::time::{Duration, Instant};

use crate::application::ports::{
    AgentOutput, AgentRunner, ExecutionContext,
    LoomLogPort, PortError,
};
use crate::domain::entities::{KnotId, LoomId, StrandPath};
use crate::domain::events::LoomEvent;
use crate::domain::value_objects::AgentConfig;

/// Maximum number of retry attempts (not counting the initial attempt).
const MAX_RETRIES: u32 = 10;

/// Default delay between retry attempts to allow transient errors to recover.
const RETRY_DELAY: Duration = Duration::from_secs(10);

/// Minimum remaining time (seconds) required to attempt a retry.
///
/// If less than this amount of budget remains, the loop bails rather
/// than starting an attempt that is almost certain to time out.
const MIN_REMAINING_SECS: u64 = 5;

/// The final-response request appended to the prompt on every session
/// resume (plan 078). One message covers both failure shapes:
/// "produce your final response" for the abrupt turn-end (empty
/// response), "continue if you have not finished" for the mid-stream
/// case (timeout, non-zero exit). User-facing agent text — keep it
/// greppable.
const FINAL_RESPONSE_REQUEST: &str =
    "Please produce your final response, or continue if you have not finished.";

/// The inactivity restart note appended to the prompt on the attempt that
/// follows an inactivity kill (plan 081). Replaces
/// [`FINAL_RESPONSE_REQUEST`] for that one attempt: it tells the agent
/// *why* the previous turn was stopped (a call blocked with no output)
/// and how to keep the session alive on the retry (emit progress within
/// the window). The helper below substitutes the actual silence duration
/// (`{silent_secs}`) and the configured window phrasing (`{window}`,
/// e.g. `5-minute window`). User-facing agent text — keep it greppable.
const INACTIVITY_RESTART_NOTE: &str = "Your last call blocked for more than \
    {silent_secs} seconds with no output, so your previous turn was \
    stopped. If you have a long-running task, ensure it emits a progress \
    update at least once within the {window} (e.g. run it in the \
    background and poll its output, or stream the output). Continue from \
    where you left off and produce your final response when done.";

/// Build the inactivity restart note from the error's silence duration
/// and configured window (plan 081). The window is phrased in minutes
/// when it is a whole number of minutes (300s → `5-minute window`),
/// otherwise in seconds (90s → `90-second window`).
fn inactivity_restart_note(silent_secs: u64, window_secs: u64) -> String {
    let window = if window_secs.is_multiple_of(60) {
        format!("{}-minute window", window_secs / 60)
    } else {
        format!("{window_secs}-second window")
    };
    // `format!` needs a literal format string, so the template const is
    // filled by substitution instead — the const stays the single
    // greppable source of the note text.
    INACTIVITY_RESTART_NOTE
        .replace("{silent_secs}", &silent_secs.to_string())
        .replace("{window}", &window)
}

/// Timestamp helper for loom-log events.
fn format_timestamp() -> String {
    crate::adapters::logging::format_timestamp()
}

/// Append one `ContextCompacted` loom event per successful compaction in
/// the invocation's metadata (plan 079). Best-effort: observability must
/// never fail a strand.
fn log_compactions(
    loom_log: &dyn LoomLogPort,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_path: &StrandPath,
    attempt: u32,
    output: &AgentOutput,
) {
    let Some(metadata) = output.metadata.as_ref() else {
        return;
    };
    for record in metadata
        .compactions
        .iter()
        .filter(|r| r.error.is_none())
    {
        let _ = loom_log.append(LoomEvent::ContextCompacted {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            session_id: metadata.session_id.clone().unwrap_or_default(),
            reason: record.reason.clone(),
            tokens_before: record.tokens_before,
            attempt,
            timestamp: format_timestamp(),
        });
    }
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Attempt to re-enter the session to request missing events.
///
/// Called after successful strand processing when the agent was instructed
/// to emit events but produced none. Re-enters the Pi session with a
/// follow-up prompt that repeats the original listener context (the exact
/// event emission instructions the agent was already given).
///
/// Returns the agent's response text, which the caller parses for events.
/// Returns `Err` if the session cannot be re-entered (e.g. no session ID,
/// runner error).
pub fn inject_event_request(
    agent_runner: &dyn AgentRunner,
    _loom_log: &dyn LoomLogPort,
    _loom_id: &LoomId,
    _knot_id: &KnotId,
    strand_path: &StrandPath,
    session_id: &Option<String>,
    mut agent_config: AgentConfig,
    listener_context: String,
    event_type: String,
    knot_name: Option<String>,
    profile_timeout: Option<Duration>,
) -> Result<String, PortError> {
    // No session ID (e.g. stdio adapter) — cannot re-enter
    let sid = session_id.as_ref().ok_or_else(|| {
        PortError::AgentExecutionFailed {
            message: "cannot re-enter session for event enforcement: no session ID".to_string(),
            session_id: None,
        }
    })?;

    // Repeat the listener context — the agent already saw these instructions
    // in the first turn but missed them. No profile prompt needed since the
    // session already has the full conversation history.
    let prompt = format!(
        "Your previous response did not contain any agent event blocks.\n\n\
         Please emit events as instructed below:\n\n\
         {}",
        listener_context,
    );

    // Append --session-id to extra_args
    agent_config.extra_args.push("--session-id".to_string());
    agent_config.extra_args.push(sid.clone());

    // Execute the follow-up with no profile prompt — session already
    // contains persona and instructions from the first turn.
    let output = agent_runner.execute_with_config(
        &agent_config,
        strand_path.clone(),
        None, // no strand file ref for follow-up
        prompt,
        String::new(),
        event_type,
        knot_name,
        profile_timeout,
    )?;

    Ok(output.stdout)
}

/// Attempt agent execution with automatic session-resume retry.
///
/// Returns [`Ok(AgentOutput)`] on success (first attempt or after N retries).
/// Returns [`Err(PortError)`] when retries are exhausted or the overall
/// timeout budget is expired.
///
/// `SessionResumed` events are appended to `loom_log` for each retry attempt.
///
/// `agent_config` contains the provider/model/tools. The retry loop
/// appends `--session-id` to `agent_config.extra_args` on each attempt.
pub fn execute_with_resume(
    agent_runner: &dyn AgentRunner,
    loom_log: &dyn LoomLogPort,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_path: &StrandPath,
    session_id: &mut Option<String>,
    agent_config: AgentConfig,
    prompt: String,
    strand_file_ref: Option<StrandPath>,
    profile_prompt: String,
    event_type: String,
    knot_name: Option<String>,
    profile_timeout: Option<Duration>,
) -> Result<AgentOutput, PortError> {
    // Allow test code to override the delay via env var.
    let retry_delay = std::env::var("KNOT_RETRY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|ms| Duration::from_millis(ms))
        .unwrap_or(RETRY_DELAY);

    execute_with_resume_internal(
        agent_runner,
        loom_log,
        loom_id,
        knot_id,
        strand_path,
        session_id,
        agent_config,
        prompt,
        strand_file_ref,
        profile_prompt,
        event_type,
        knot_name,
        profile_timeout,
        retry_delay,
    )
}

// ── Internal Implementation ────────────────────────────────────────────────

/// Core retry-loop implementation with configurable delay for testing.
fn execute_with_resume_internal(
    agent_runner: &dyn AgentRunner,
    loom_log: &dyn LoomLogPort,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_path: &StrandPath,
    session_id: &mut Option<String>,
    mut agent_config: AgentConfig,
    mut prompt: String,
    strand_file_ref: Option<StrandPath>,
    profile_prompt: String,
    event_type: String,
    knot_name: Option<String>,
    profile_timeout: Option<Duration>,
    retry_delay: Duration,
) -> Result<AgentOutput, PortError> {
    let start = Instant::now();

    // Plan 081: the cause-specific note for the next retry prompt — set
    // when a failure is classified as an inactivity kill, consumed when
    // the retry prompt is built; `None` → the 078 final-response request.
    let mut pending_note: Option<String> = None;
    // Plan 081: count of inactivity kills (terminal exhaustion message).
    let mut inactivity_kills: u32 = 0;

    // --- First attempt (no session ID) ---
    // Delegate to execute_with_config so the adapter layer can
    // inject --name and @{path} into extra_args.
    let result = agent_runner.execute_with_config(
        &agent_config,
        strand_path.clone(),
        strand_file_ref.clone(),
        prompt.clone(),
        profile_prompt.clone(),
        event_type.clone(),
        knot_name.clone(),
        profile_timeout,
    );

    let mut first_error;
    if let Ok(output) = result {
        if output.stdout.trim().is_empty() {
            // Abrupt turn-end (plan 078): log, then request the final
            // response by re-entering the session. No deadline was
            // exceeded, so the failure is AgentNoResponse, not Timeout
            // (plan 077).
            let sid = output.metadata.as_ref()
                .and_then(|m| m.session_id.clone());
            loom_log.append(LoomEvent::KnotEmptyResponse {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                attempt: 1,
                timestamp: format_timestamp(),
            })?;
            if sid.is_none() {
                // No session ID (stdio adapter / unparseable output) —
                // cannot re-enter; the 077 terminal failure stands.
                return Err(PortError::AgentNoResponse {
                    message: "agent returned empty response (no session id — cannot request final response)".to_string(),
                    session_id: None,
                });
            }
            // `Some` is guaranteed by the check above — narrow the type.
            let sid = sid.unwrap();
            *session_id = Some(sid.clone());
            first_error = PortError::AgentNoResponse {
                message: "agent returned empty response".to_string(),
                session_id: Some(sid),
            };
            // Fall through to the retry loop — the nudge re-enters the
            // session and requests the final response.
        } else {
            // Capture session_id from successful output metadata
            if let Some(ref metadata) = output.metadata {
                if let Some(ref sid) = metadata.session_id {
                    *session_id = Some(sid.clone());
                }
            }
            // Plan 079: record successful compactions observed on the
            // first attempt (best-effort).
            log_compactions(loom_log, loom_id, knot_id, strand_path, 1, &output);
            return Ok(output);
        }
    } else {
        let err = result.unwrap_err();

        // Check if the first failure is resumable.
        // The session_id for retry comes from the error itself (captured by
        // the JSON adapter from Pi's first JSONL line before generation
        // starts). If the error carries no session_id, we cannot resume.
        let error_session_id = err.session_id().cloned();
        // Plan 081: an inactivity kill is the one deliberate exception to
        // the session-ID gate — a fresh restart (no `--session-id`) is
        // safe because knots are idempotent, and the blocking-call note
        // is what makes the retry different.
        let inactivity = matches!(&err, PortError::AgentInactivity { .. });
        if (!err.is_resumable() || error_session_id.is_none()) && !inactivity {
            // Not resumable or no session_id — extract what we can and
            // return
            if let Some(sid) = err.session_id() {
                *session_id = Some(sid.clone());
            }
            return Err(err);
        }

        // Capture session_id from error for retry (stays None for a
        // pre-session inactivity stall — the loop's existing
        // `if let Some(sid)` skips `--session-id`).
        *session_id = error_session_id;
        first_error = err;

        // Plan 081: record the stall (attempt 1 — the initial call outside
        // the loop, KNotEmptyResponse convention) and queue the
        // blocking-call note for the retry prompt.
        if let PortError::AgentInactivity {
            silent_secs,
            window_secs,
            blocked_call,
            ..
        } = &first_error
        {
            inactivity_kills += 1;
            pending_note =
                Some(inactivity_restart_note(*silent_secs, *window_secs));
            loom_log.append(LoomEvent::AgentInactivity {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                session_id: session_id.clone().unwrap_or_default(),
                silent_secs: *silent_secs,
                window_secs: *window_secs,
                blocked_call: blocked_call.clone(),
                attempt: 1,
                timestamp: format_timestamp(),
            })?;
        }
    }

    // --- Retry loop ---
    for attempt in 1..=MAX_RETRIES {
        // Check overall timeout budget (only when profile has a timeout)
        if let Some(timeout_budget) = profile_timeout {
            let elapsed = start.elapsed();
            let remaining = timeout_budget.saturating_sub(elapsed);

            // Bail if insufficient time remains
            if remaining.as_secs() < MIN_REMAINING_SECS {
                return Err(PortError::Timeout {
                    message: format!(
                        "overall timeout budget exhausted after {} attempt(s) \
                         ({}s used of {}s budget)",
                        attempt,
                        elapsed.as_secs(),
                        timeout_budget.as_secs(),
                    ),
                    session_id: session_id.clone(),
                });
            }
        }

        // Delay between retries to allow transient errors to recover
        std::thread::sleep(retry_delay);

        // Re-check budget after the delay
        if let Some(timeout_budget) = profile_timeout {
            let elapsed = start.elapsed();
            if elapsed >= timeout_budget {
                return Err(PortError::Timeout {
                    message: format!(
                        "overall timeout budget exhausted after {} attempt(s) \
                         ({}s used of {}s budget)",
                        attempt,
                        elapsed.as_secs(),
                        timeout_budget.as_secs(),
                    ),
                    session_id: session_id.clone(),
                });
            }
        }

        // Update session_id from the error (in case it changed)
        if let Some(sid) = first_error.session_id() {
            *session_id = Some(sid.clone());
        }

        // Prepare agent_config and prompt for retry
        // Append --session-id to extra_args (skipped for a fresh
        // inactivity restart — no session was captured) and the
        // cause-specific note to the prompt: the blocking-call note
        // (plan 081) when the failure was an inactivity kill, the
        // final-response request (plan 078) otherwise.
        if let Some(sid) = session_id {
            agent_config.extra_args.push("--session-id".to_string());
            agent_config.extra_args.push(sid.clone());
        }
        let note = pending_note
            .take()
            .unwrap_or_else(|| FINAL_RESPONSE_REQUEST.to_string());
        prompt.push_str("\n\n");
        prompt.push_str(&note);

        // Log SessionResumed event
        loom_log.append(LoomEvent::SessionResumed {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            session_id: session_id.clone().unwrap_or_default(),
            attempt,
            timestamp: format_timestamp(),
        })?;

        // Build context with remaining time and execute.
        // Delegate to execute_with_config so the adapter layer
        // can inject --name and @{path} into extra_args.
        let timeout = profile_timeout.as_ref().map(|t| t.saturating_sub(start.elapsed()));
        match agent_runner.execute_with_config(
            &agent_config,
            strand_path.clone(),
            strand_file_ref.clone(),
            prompt.clone(),
            profile_prompt.clone(),
            event_type.clone(),
            knot_name.clone(),
            timeout,
        ) {
            Ok(output) => {
                if output.stdout.trim().is_empty() {
                    // Log empty response to loom-log (attempt = loop_attempt + 1
                    // since attempt 1 was the initial call outside the loop)
                    let _ = loom_log.append(LoomEvent::KnotEmptyResponse {
                        loom_id: loom_id.clone(),
                        knot_id: knot_id.clone(),
                        strand_path: strand_path.clone(),
                        attempt: attempt + 1,
                        timestamp: format_timestamp(),
                    });
                    // Resumable error (not a timeout — no deadline was
                    // exceeded) — continue retry loop
                    let error = PortError::AgentNoResponse {
                        message: "agent returned empty response".to_string(),
                        session_id: session_id.clone(),
                    };
                    if let Some(sid) = error.session_id() {
                        *session_id = Some(sid.clone());
                    }
                    first_error = error;
                    continue;
                }
                // Update session_id from successful output metadata
                if let Some(ref metadata) = output.metadata {
                    if let Some(ref sid) = metadata.session_id {
                        *session_id = Some(sid.clone());
                    }
                }
                // Plan 079: record successful compactions observed on
                // this retry (KnotEmptyResponse convention: attempt + 1).
                log_compactions(
                    loom_log,
                    loom_id,
                    knot_id,
                    strand_path,
                    attempt + 1,
                    &output,
                );
                return Ok(output);
            }
            Err(e) => {
                // Update session_id from error
                if let Some(sid) = e.session_id() {
                    *session_id = Some(sid.clone());
                }

                // Check if error is still resumable and budget allows
                if !e.is_resumable() {
                    return Err(e);
                }

                if let Some(timeout_budget) = profile_timeout {
                    let remaining = timeout_budget.saturating_sub(start.elapsed());
                    if remaining.as_secs() < MIN_REMAINING_SECS {
                        return Err(PortError::Timeout {
                            message: format!(
                                "overall timeout budget exhausted after {} \
                                 attempt(s) ({}s used of {}s budget)",
                                attempt,
                                start.elapsed().as_secs(),
                                timeout_budget.as_secs(),
                            ),
                            session_id: session_id.clone(),
                        });
                    }
                }

                first_error = e;

                // Plan 081: record the stall (attempt + 1 — KNotEmpty-
                // Response convention) and queue the blocking-call note
                // for the next retry.
                if let PortError::AgentInactivity {
                    silent_secs,
                    window_secs,
                    blocked_call,
                    ..
                } = &first_error
                {
                    inactivity_kills += 1;
                    pending_note =
                        Some(inactivity_restart_note(*silent_secs, *window_secs));
                    let _ = loom_log.append(LoomEvent::AgentInactivity {
                        loom_id: loom_id.clone(),
                        knot_id: knot_id.clone(),
                        strand_path: strand_path.clone(),
                        session_id: session_id.clone().unwrap_or_default(),
                        silent_secs: *silent_secs,
                        window_secs: *window_secs,
                        blocked_call: blocked_call.clone(),
                        attempt: attempt + 1,
                        timestamp: format_timestamp(),
                    });
                }
            }
        }
    }

    // Exhausted all retries — classify by the last failure (plan 078):
    // the terminal error reflects the cause.
    let budget_suffix = profile_timeout
        .map(|t| format!(" (overall timeout: {}s)", t.as_secs()))
        .unwrap_or_default();
    match &first_error {
        // The last failure was a genuine timeout (adapter kill mid-retry)
        // — a deadline did run out: Timeout (rig-log `TimeoutExceeded`).
        PortError::Timeout { .. } => Err(PortError::Timeout {
            message: format!(
                "session resume exhausted {MAX_RETRIES} retries{budget_suffix}"
            ),
            session_id: session_id.clone(),
        }),
        // The last failure was an inactivity kill (plan 081) — the
        // watchdog deadline did fire; cause-accurate terminal per 077/078.
        PortError::AgentInactivity {
            silent_secs,
            window_secs,
            blocked_call,
            ..
        } => Err(PortError::AgentInactivity {
            message: format!(
                "session resume exhausted {MAX_RETRIES} retries after \
                 {inactivity_kills} inactivity kills{budget_suffix}"
            ),
            silent_secs: *silent_secs,
            window_secs: *window_secs,
            blocked_call: blocked_call.clone(),
            session_id: session_id.clone(),
        }),
        // The last failure was not a timeout — typically the empty
        // response that started the nudge chain. No deadline was
        // exceeded, so this is a failure, not a timeout (plan 077).
        _ => Err(PortError::AgentNoResponse {
            message: format!(
                "agent returned empty response after {} attempts \
                 (session resume exhausted)",
                MAX_RETRIES + 1
            ),
            session_id: session_id.clone(),
        }),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::AgentInvocationMetadata;
    use crate::domain::value_objects::AgentConfig;
    use std::path::PathBuf;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    /// Mock agent runner with configurable response sequence and context
    /// capture for verifying retry parameters.
    #[derive(Default)]
    struct TestAgentRunner {
        responses: Arc<Mutex<VecDeque<Result<AgentOutput, PortError>>>>,
        contexts: Arc<Mutex<Vec<ExecutionContext>>>,
        call_count: Arc<AtomicU32>,
    }

    impl TestAgentRunner {
        fn new(
            responses: Vec<Result<AgentOutput, PortError>>,
        ) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses.into())),
                contexts: Arc::new(Mutex::new(Vec::new())),
                call_count: Arc::new(AtomicU32::new(0)),
            }
        }

        fn contexts(&self) -> Vec<ExecutionContext> {
            self.contexts.lock().unwrap().clone()
        }

        fn call_count(&self) -> u32 {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    impl AgentRunner for TestAgentRunner {
        fn execute(
            &self,
            ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.contexts.lock().unwrap().push(ctx);

            let mut responses = self.responses.lock().unwrap();
            if let Some(result) = responses.pop_front() {
                result
            } else {
                Err(PortError::Timeout {
                    message: "exhausted".to_string(),
                    session_id: Some("sess-test".to_string()),
                })
            }
        }
    }

    /// In-memory loom log that records all appended events (and any
    /// `clear_all` calls, for assertions).
    #[derive(Default)]
    struct TestLoomLog {
        events: Arc<Mutex<Vec<LoomEvent>>>,
        clear_all_calls: Arc<Mutex<usize>>,
    }

    impl TestLoomLog {
        fn events(&self) -> Vec<LoomEvent> {
            self.events.lock().unwrap().clone()
        }

        fn clear_all_calls(&self) -> usize {
            *self.clear_all_calls.lock().unwrap()
        }
    }

    impl LoomLogPort for TestLoomLog {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, event: LoomEvent) -> Result<(), PortError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(self.events.lock().unwrap().clone())
        }

        fn clear_all(&self) -> Result<(), PortError> {
            *self.clear_all_calls.lock().unwrap() += 1;
            self.events.lock().unwrap().clear();
            Ok(())
        }
    }

    fn make_loom_id() -> LoomId {
        LoomId("test-loom".to_string())
    }

    fn make_knot_id() -> KnotId {
        KnotId("k1".to_string())
    }

    fn make_strand_path() -> StrandPath {
        StrandPath(PathBuf::from("input/strand.md"))
    }

    fn ok_output(stdout: &str) -> AgentOutput {
        ok_output_with_sid(stdout, "sess-abc")
    }

    fn ok_output_with_sid(stdout: &str, sid: &str) -> AgentOutput {
        AgentOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: Some(AgentInvocationMetadata {
                session_id: Some(sid.to_string()),
                token_usage: None,
                compactions: vec![],
            }),
        }
    }

    fn err_timeout(sid: &str) -> PortError {
        PortError::Timeout {
            message: "timed out".to_string(),
            session_id: Some(sid.to_string()),
        }
    }

    fn err_timeout_no_sid() -> PortError {
        PortError::Timeout {
            message: "timed out".to_string(),
            session_id: None,
        }
    }

    fn err_fatal() -> PortError {
        PortError::CommandNotFound("pi not found".to_string())
    }

    /// Plan 079: build a compaction record for mock metadata.
    fn comp_rec(
        reason: &str,
        tokens_before: Option<u64>,
        will_retry: bool,
        error: Option<&str>,
    ) -> crate::application::ports::CompactionRecord {
        crate::application::ports::CompactionRecord {
            reason: reason.to_string(),
            tokens_before,
            will_retry,
            error: error.map(String::from),
        }
    }

    /// Plan 079: successful output with compaction records in the
    /// metadata.
    fn ok_output_with_compactions(
        stdout: &str,
        sid: &str,
        compactions: Vec<crate::application::ports::CompactionRecord>,
    ) -> AgentOutput {
        AgentOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: Some(AgentInvocationMetadata {
                session_id: Some(sid.to_string()),
                token_usage: None,
                compactions,
            }),
        }
    }

    /// Plan 079: terminal context-overflow error.
    fn err_context_limit(sid: &str) -> PortError {
        PortError::ContextLimitReached {
            message: "session context cannot fit the model window even after compaction".to_string(),
            session_id: Some(sid.to_string()),
        }
    }

    // Helper for execute_with_resume calls with zero-delay for tests.
    fn execute(
        runner: &dyn AgentRunner,
        log: &dyn LoomLogPort,
        timeout_secs: u64,
    ) -> Result<AgentOutput, PortError> {
        execute_with_resume_internal(
            runner,
            log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &mut None,
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "Review this document".to_string(),
            Some(make_strand_path()),
            "You are a reviewer.".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            Some(Duration::from_secs(timeout_secs)),
            Duration::from_millis(0),
        )
    }

    // Helper for execute_with_resume calls with NO profile timeout budget
    // (zero delay) — MAX_RETRIES is the only bound on the retry loop.
    fn execute_no_budget(
        runner: &dyn AgentRunner,
        log: &dyn LoomLogPort,
    ) -> Result<AgentOutput, PortError> {
        execute_with_resume_internal(
            runner,
            log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &mut None,
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "Review this document".to_string(),
            Some(make_strand_path()),
            "You are a reviewer.".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            None, // no profile timeout budget
            Duration::from_millis(0),
        )
    }

    #[test]
    fn retry_succeeds_on_first_retry() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Ok(ok_output("success after retry")),
        ]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);

        assert!(result.is_ok());
        assert_eq!(result.unwrap().stdout, "success after retry");

        let events = log.events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::SessionResumed {
                session_id, attempt, ..
            } => {
                assert_eq!(session_id, "sess-abc");
                assert_eq!(*attempt, 1);
            }
            _ => panic!("Expected SessionResumed, got {:?}", events[0]),
        }
    }

    #[test]
    fn retry_exhausted_then_fails() {
        let responses: Vec<Result<AgentOutput, PortError>> = (0..20)
            .map(|_| Err(err_timeout("sess-abc")))
            .collect();
        let runner = TestAgentRunner::new(responses);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 3600);

        assert!(result.is_err());

        let events = log.events();
        // 10 retries logged (MAX_RETRIES)
        assert_eq!(events.len(), 10);

        // Last attempt is 10
        match &events.last().unwrap() {
            LoomEvent::SessionResumed { attempt, .. } => {
                assert_eq!(*attempt, 10);
            }
            _ => panic!("Expected SessionResumed"),
        }
    }

    #[test]
    fn retry_stops_on_overall_timeout() {
        // Simulate budget exhaustion: budget = 1s, each attempt "takes" ~400ms.
        // After attempt 1: t≈400ms, remaining≈600ms → retry (600 > 5000? No → bail)
        // Actually 600ms < 5000ms (MIN_REMAINING_SECS=5), so bail immediately.
        // Let's use a 7s budget with 3s per attempt:
        //   attempt 1: t≈3s, remaining≈4s → bail (4 < 5)
        // That's still not enough for a retry. Use 8s budget, 3s per attempt:
        //   attempt 1: t≈3s, remaining≈5s → check: 5 >= 5, ok → retry logged
        //   retry 1: t≈6s, remaining≈2s → check: 2 < 5 → bail
        // Result: 1 retry logged, budget exhausted after 2 attempts.

        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Err(err_timeout("sess-abc")),
            Err(err_timeout("sess-abc")),
        ]);
        let log = TestLoomLog::default();

        // Verify that budget < MIN_REMAINING_SECS causes immediate bail.
        // Budget=4s < MIN_REMAINING_SECS=5s.
        // First attempt fails, check: remaining=4s < 5 → bail immediately.
        // Result: 0 retries logged, budget exhaustion error returned.
        let result = execute_with_resume_internal(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &mut None,
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "Review this document".to_string(),
            Some(make_strand_path()),
            "You are a reviewer.".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            Some(Duration::from_secs(4)), // budget < MIN_REMAINING_SECS (5)
            Duration::from_millis(0),
        );

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::Timeout { message, .. } => {
                assert!(
                    message.contains("exhausted") || message.contains("budget"),
                    "Expected budget exhaustion message, got: {}",
                    message
                );
            }
            _ => panic!("Expected Timeout error"),
        }

        // With budget < MIN_REMAINING, no retry is attempted
        let events = log.events();
        assert!(
            events.is_empty(),
            "Expected no retries when budget < MIN_REMAINING_SECS"
        );
    }

    #[test]
    fn retry_stops_on_insufficient_time() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Err(err_timeout("sess-abc")),
        ]);
        let log = TestLoomLog::default();

        // Budget of 3s is less than MIN_REMAINING_SECS (5s) after first attempt
        let result = execute_with_resume_internal(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &mut None,
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "Review this document".to_string(),
            Some(make_strand_path()),
            "You are a reviewer.".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            Some(Duration::from_secs(3)),
            Duration::from_millis(0),
        );

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::Timeout { message, .. } => {
                assert!(
                    message.contains("exhausted") || message.contains("budget"),
                    "Expected budget exhaustion message, got: {}",
                    message
                );
            }
            _ => panic!("Expected Timeout error"),
        }
    }

    #[test]
    fn no_retry_on_fatal_error() {
        let runner = TestAgentRunner::new(vec![Err(err_fatal())]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);

        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), PortError::CommandNotFound(_)),
            "Expected CommandNotFound"
        );

        // No SessionResumed logged
        assert!(
            log.events().is_empty(),
            "Expected no loom-log events for fatal error"
        );
    }

    #[test]
    fn no_retry_when_no_session_id() {
        let runner =
            TestAgentRunner::new(vec![Err(err_timeout_no_sid())]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::Timeout { session_id, .. } => {
                assert!(session_id.is_none());
            }
            _ => panic!("Expected Timeout error"),
        }

        // No SessionResumed logged (no session_id to resume)
        assert!(
            log.events().is_empty(),
            "Expected no loom-log events when no session_id"
        );
    }

    #[test]
    fn retry_preserves_other_cli_args() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Ok(ok_output("success")),
        ]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);
        assert!(result.is_ok());

        let contexts = runner.contexts();
        assert!(contexts.len() >= 2);

        // Second context (retry) has --session-id in extra_args
        let retry_ctx = &contexts[1];
        let extra_args = &retry_ctx.agent_config.extra_args;

        // --session-id appended via extra_args
        assert!(extra_args.contains(&"--session-id".to_string()));
        assert!(extra_args.contains(&"sess-abc".to_string()));
    }

    #[test]
    fn retry_appends_final_response_request() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Ok(ok_output("success")),
        ]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);
        assert!(result.is_ok());

        let contexts = runner.contexts();
        assert!(contexts.len() >= 2);

        // First attempt: original prompt
        assert_eq!(
            contexts[0].prompt,
            "Review this document"
        );

        // Retry: prompt includes the final-response request (plan 078)
        assert!(
            contexts[1].prompt.contains(FINAL_RESPONSE_REQUEST),
            "Retry prompt should contain the final-response request, got: {}",
            contexts[1].prompt
        );
    }

    #[test]
    fn retry_delay_between_attempts() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Ok(ok_output("success")),
        ]);
        let log = TestLoomLog::default();

        // Use a real delay for this test (100ms instead of 10s)
        let start = Instant::now();

        let result = execute_with_resume_internal(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &mut None,
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "Review this document".to_string(),
            Some(make_strand_path()),
            "You are a reviewer.".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            Some(Duration::from_secs(120)),
            Duration::from_millis(100),
        );

        assert!(result.is_ok());
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(80),
            "Expected at least 80ms delay (configured 100ms), got {}ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn session_id_captured_from_error() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-captured")),
            Ok(ok_output_with_sid("success", "sess-captured")),
        ]);
        let log = TestLoomLog::default();

        let mut session_id: Option<String> = None;

        let result = execute_with_resume_internal(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &mut session_id,
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "Review this document".to_string(),
            Some(make_strand_path()),
            "You are a reviewer.".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            Some(Duration::from_secs(120)),
            Duration::from_millis(0),
        );

        assert!(result.is_ok());
        assert_eq!(session_id, Some("sess-captured".to_string()));
    }

    #[test]
    fn successful_retry_transparent() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Ok(ok_output("success")),
        ]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);
        assert!(result.is_ok());

        let events = log.events();
        // Only SessionResumed — no KnotFailed (that's ProcessStrand's job)
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], LoomEvent::SessionResumed { .. }),
            "Expected SessionResumed"
        );
    }

    #[test]
    fn first_attempt_succeeds_no_retry() {
        let runner = TestAgentRunner::new(vec![Ok(ok_output("immediate"))]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().stdout, "immediate");

        // No SessionResumed logged
        assert!(
            log.events().is_empty(),
            "Expected no loom-log events for immediate success"
        );
    }

    /// Plan 077: an abrupt turn-end with an empty response is
    /// `AgentNoResponse`, **not** `Timeout` — no deadline was exceeded.
    ///
    /// Plan 078 update: with a session ID the call now *retries* (see
    /// `empty_response_first_attempt_triggers_retry`); this test is the
    /// no-session-ID reproduction where the 077 terminal failure stands.
    #[test]
    fn empty_response_first_attempt_logs_knot_empty_response() {
        let runner = TestAgentRunner::new(vec![Ok(AgentOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        })]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);

        // Returns error — an empty response is a failure, not a timeout
        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::AgentNoResponse { message, session_id } => {
                assert!(
                    message.contains("empty response"),
                    "Expected empty response message, got: {}",
                    message
                );
                assert!(
                    session_id.is_none(),
                    "no session ID → error carries no session_id, got: {:?}",
                    session_id
                );
            }
            other => panic!("Expected AgentNoResponse error, got: {other:?}"),
        }

        // KnotEmptyResponse logged with attempt 1
        let events = log.events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::KnotEmptyResponse { attempt, .. } => {
                assert_eq!(*attempt, 1);
            }
            _ => panic!("Expected KnotEmptyResponse"),
        }
    }

    #[test]
    fn empty_response_retry_logs_knot_empty_response() {
        // First attempt: timeout (triggers retry setup)
        // Second attempt: empty response → logged with attempt 2
        // Third attempt: success
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Ok(ok_output("")),
            Ok(ok_output("success after empty")),
        ]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().stdout, "success after empty");

        // SessionResumed (retry 1) + KnotEmptyResponse (attempt 2)
        // + SessionResumed (retry 2) — but wait, empty response sets
        // first_error and continues the loop, then next iteration
        // does SessionResumed + execute → success
        let events = log.events();
        assert_eq!(events.len(), 3);

        // Event 1: SessionResumed for retry 1
        match &events[0] {
            LoomEvent::SessionResumed { attempt, .. } => {
                assert_eq!(*attempt, 1);
            }
            _ => panic!("Expected SessionResumed, got {:?}", events[0]),
        }
        // Event 2: KnotEmptyResponse for attempt 2 (retry 1's result)
        match &events[1] {
            LoomEvent::KnotEmptyResponse { attempt, .. } => {
                assert_eq!(*attempt, 2);
            }
            _ => panic!("Expected KnotEmptyResponse, got {:?}", events[1]),
        }
        // Event 3: SessionResumed for retry 2
        match &events[2] {
            LoomEvent::SessionResumed { attempt, .. } => {
                assert_eq!(*attempt, 2);
            }
            _ => panic!("Expected SessionResumed, got {:?}", events[2]),
        }
    }

    // ── Plan 078: first-attempt empty response re-enters the session ──

    /// Plan 078: an abrupt turn-end (empty response) **with** a captured
    /// session ID re-enters the session to request the final response.
    /// First attempt: `KnotEmptyResponse(1)`; retry: `SessionResumed(1)`
    /// with `--session-id` and the final-response request prompt; the
    /// non-empty follow-up response is returned transparently.
    #[test]
    fn empty_response_first_attempt_triggers_retry() {
        let runner = TestAgentRunner::new(vec![
            Ok(ok_output_with_sid("", "sess-abc")),
            Ok(ok_output_with_sid("final response", "sess-abc")),
        ]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);

        assert!(
            result.is_ok(),
            "expected Ok after the final-response nudge, got: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().stdout, "final response");

        // Runner called twice: initial attempt + 1 retry
        assert_eq!(runner.call_count(), 2);

        // Retry context carries --session-id and the final-response request
        let contexts = runner.contexts();
        assert_eq!(contexts.len(), 2);
        let retry_ctx = &contexts[1];
        assert!(
            retry_ctx
                .agent_config
                .extra_args
                .contains(&"--session-id".to_string()),
            "retry extra_args should contain --session-id: {:?}",
            retry_ctx.agent_config.extra_args
        );
        assert!(
            retry_ctx
                .agent_config
                .extra_args
                .contains(&"sess-abc".to_string()),
            "retry extra_args should contain the session ID: {:?}",
            retry_ctx.agent_config.extra_args
        );
        assert!(
            retry_ctx.prompt.contains(
                "Please produce your final response, or continue if you have not finished."
            ),
            "retry prompt should contain the final-response request, got: {}",
            retry_ctx.prompt
        );

        // Loom-log: KnotEmptyResponse(1) then SessionResumed(1)
        let events = log.events();
        assert_eq!(events.len(), 2);
        match &events[0] {
            LoomEvent::KnotEmptyResponse { attempt, .. } => {
                assert_eq!(*attempt, 1);
            }
            other => panic!("Expected KnotEmptyResponse, got {other:?}"),
        }
        match &events[1] {
            LoomEvent::SessionResumed { attempt, session_id, .. } => {
                assert_eq!(*attempt, 1);
                assert_eq!(session_id, "sess-abc");
            }
            other => panic!("Expected SessionResumed, got {other:?}"),
        }
    }

    /// Plan 078: an empty response **without** a session ID (stdio
    /// adapter / unparseable output) cannot be re-entered — the 077
    /// terminal failure stands: one runner call, `AgentNoResponse` with
    /// no session ID, and only `KnotEmptyResponse(1)` in the loom-log
    /// (no `SessionResumed`).
    #[test]
    fn empty_response_without_session_id_no_retry() {
        let runner = TestAgentRunner::new(vec![Ok(AgentOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        })]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::AgentNoResponse { session_id, .. } => {
                assert!(
                    session_id.is_none(),
                    "error should carry no session_id, got: {:?}",
                    session_id
                );
            }
            other => panic!("Expected AgentNoResponse, got: {other:?}"),
        }

        // Runner called exactly once — no retry without a session ID
        assert_eq!(runner.call_count(), 1);

        // Loom-log: KnotEmptyResponse(1) only — no SessionResumed
        let events = log.events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::KnotEmptyResponse { attempt, .. } => {
                assert_eq!(*attempt, 1);
            }
            other => panic!("Expected KnotEmptyResponse, got {other:?}"),
        }
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LoomEvent::SessionResumed { .. })),
            "no SessionResumed without a session ID"
        );
    }

    /// Plan 078: when every attempt (initial + all 10 retries) returns an
    /// empty response, the terminal error is `AgentNoResponse` — **not**
    /// `Timeout` — with the full attempt count: the cause was never a
    /// deadline breach. No profile timeout → MAX_RETRIES is the bound.
    #[test]
    fn empty_response_exhausted_returns_no_response_error() {
        let responses: Vec<Result<AgentOutput, PortError>> = (0..11)
            .map(|_| Ok(ok_output_with_sid("", "sess-abc")))
            .collect();
        let runner = TestAgentRunner::new(responses);
        let log = TestLoomLog::default();

        let result = execute_no_budget(&runner, &log);

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::AgentNoResponse { message, .. } => {
                assert!(
                    message.contains("after 11 attempts"),
                    "message should count all attempts (1 + 10 retries), got: {message}"
                );
                assert!(
                    message.contains("exhausted"),
                    "message should mention exhaustion, got: {message}"
                );
            }
            other => panic!("Expected AgentNoResponse, got: {other:?}"),
        }

        // Runner called 11 times: initial + 10 retries
        assert_eq!(runner.call_count(), 11);

        // Loom-log: 11 × KnotEmptyResponse + 10 × SessionResumed
        let events = log.events();
        let empty_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::KnotEmptyResponse { .. }))
            .count();
        let resumed_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::SessionResumed { .. }))
            .count();
        assert_eq!(empty_count, 11, "expected 11 KnotEmptyResponse events");
        assert_eq!(resumed_count, 10, "expected 10 SessionResumed events");
    }

    /// Plan 078: when the **last** failure in the nudge loop is a genuine
    /// timeout (adapter kill mid-retry), the terminal error is
    /// `PortError::Timeout` — the exhaustion error reflects the cause.
    #[test]
    fn empty_response_retry_timeout_terminal_is_timeout() {
        // First attempt: empty response (with session ID) → nudge loop;
        // then genuine timeouts until exhaustion (no budget → 10 retries).
        let responses: Vec<Result<AgentOutput, PortError>> = std::iter::once(Ok(
            ok_output_with_sid("", "sess-abc"),
        ))
        .chain((0..10).map(|_| Err(err_timeout("sess-abc"))))
        .collect();
        let runner = TestAgentRunner::new(responses);
        let log = TestLoomLog::default();

        let result = execute_no_budget(&runner, &log);

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::Timeout { message, .. } => {
                assert!(
                    message.contains("exhausted"),
                    "message should mention exhaustion, got: {message}"
                );
            }
            other => panic!(
                "Expected Timeout (last failure was a real timeout), got: {other:?}"
            ),
        }

        // Loom-log: KnotEmptyResponse(1) + 10 × SessionResumed — the
        // in-loop timeouts do not log KnotEmptyResponse.
        let events = log.events();
        let empty_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::KnotEmptyResponse { .. }))
            .count();
        let resumed_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::SessionResumed { .. }))
            .count();
        assert_eq!(empty_count, 1, "expected 1 KnotEmptyResponse (first attempt only)");
        assert_eq!(resumed_count, 10, "expected 10 SessionResumed events");
    }

    // ── Plan 079: compaction visibility + terminal overflow ──────

    /// Plan 079: a successful compaction observed on the **first attempt**
    /// is logged as `ContextCompacted { attempt: 1 }` — no `SessionResumed`
    /// (nothing failed).
    #[test]
    fn compaction_logged_on_first_attempt() {
        let runner = TestAgentRunner::new(vec![Ok(ok_output_with_compactions(
            "done",
            "sess-abc",
            vec![comp_rec("overflow", Some(150000), true, None)],
        ))]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);
        assert!(
            result.is_ok(),
            "first-attempt success: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().stdout, "done");

        let events = log.events();
        assert_eq!(events.len(), 1, "exactly one loom-log event");
        match &events[0] {
            LoomEvent::ContextCompacted {
                reason,
                tokens_before,
                attempt,
                ..
            } => {
                assert_eq!(reason, "overflow");
                assert_eq!(*tokens_before, Some(150000));
                assert_eq!(*attempt, 1);
            }
            other => panic!("Expected ContextCompacted, got {other:?}"),
        }
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, LoomEvent::SessionResumed { .. })),
            "no SessionResumed on a successful first attempt"
        );
    }

    /// Plan 079: a successful compaction observed on a **retry** is logged
    /// after the `SessionResumed` that started the retry, with the
    /// `KnotEmptyResponse` attempt convention (attempt 2 = first retry).
    #[test]
    fn compaction_logged_on_retry_attempt() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Ok(ok_output_with_compactions(
                "done",
                "sess-abc",
                vec![comp_rec("threshold", Some(90000), true, None)],
            )),
        ]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);
        assert!(
            result.is_ok(),
            "retry success: {:?}",
            result.err()
        );

        let events = log.events();
        assert_eq!(events.len(), 2);
        match &events[0] {
            LoomEvent::SessionResumed { attempt, .. } => {
                assert_eq!(*attempt, 1);
            }
            other => panic!("Expected SessionResumed, got {other:?}"),
        }
        match &events[1] {
            LoomEvent::ContextCompacted { reason, attempt, .. } => {
                assert_eq!(reason, "threshold");
                assert_eq!(
                    *attempt, 2,
                    "attempt follows the KnotEmptyResponse convention"
                );
            }
            other => panic!("Expected ContextCompacted, got {other:?}"),
        }
    }

    /// Plan 079: a **failed** compaction (`error: Some(…)`) is not logged —
    /// only successful compactions mark context pressure. The non-empty
    /// stdout means the turn still produced a final response.
    #[test]
    fn failed_compaction_not_logged() {
        let runner = TestAgentRunner::new(vec![Ok(ok_output_with_compactions(
            "done",
            "sess-abc",
            vec![comp_rec(
                "overflow",
                None,
                false,
                Some("Context overflow recovery failed after one compact-and-retry attempt."),
            )],
        ))]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);
        assert!(
            result.is_ok(),
            "non-empty stdout is a success: {:?}",
            result.err()
        );

        assert!(
            log.events().is_empty(),
            "failed compactions are not logged, got: {:?}",
            log.events()
        );
    }

    /// Plan 079: a terminal context overflow on the **first attempt** is
    /// returned immediately — the error is not resumable, so the runner
    /// is called exactly once and nothing is logged (no `SessionResumed`).
    #[test]
    fn terminal_overflow_first_attempt_no_retry() {
        let runner =
            TestAgentRunner::new(vec![Err(err_context_limit("sess-ctx"))]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::ContextLimitReached { .. } => {}
            other => panic!("Expected ContextLimitReached, got: {other:?}"),
        }
        assert_eq!(
            runner.call_count(),
            1,
            "no retry for a non-resumable error"
        );
        assert!(
            log.events().is_empty(),
            "no loom-log events for a first-attempt terminal overflow, got: {:?}",
            log.events()
        );
    }

    /// Plan 079: a terminal context overflow **inside the retry loop**
    /// exits immediately — the loop does not clock up the remaining
    /// retries. The loom-log holds only the `SessionResumed` that
    /// preceded the failing attempt; the runner is called twice.
    #[test]
    fn terminal_overflow_in_loop_exits() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Err(err_context_limit("sess-abc")),
        ]);
        let log = TestLoomLog::default();

        let result = execute(&runner, &log, 120);

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::ContextLimitReached { .. } => {}
            other => panic!("Expected ContextLimitReached, got: {other:?}"),
        }
        assert_eq!(
            runner.call_count(),
            2,
            "exits on the terminal error — no further retries"
        );

        let events = log.events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::SessionResumed { attempt, .. } => {
                assert_eq!(*attempt, 1);
            }
            other => panic!("Expected SessionResumed, got {other:?}"),
        }
    }

    // ── inject_event_request Tests (Phase 2) ─────────────────────────

    #[test]
    fn inject_event_request_success_with_events() {
        let response = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "---\n",
            "Plan created.\n",
            "```",
        );
        let runner = TestAgentRunner::new(vec![Ok(ok_output(response))]);
        let log = TestLoomLog::default();

        let result = inject_event_request(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &Some("sess-abc".to_string()),
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "## Agent Events\n\nYou may emit: PlanCreated".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            None,
        );

        assert!(result.is_ok());
        let stdout = result.unwrap();
        assert!(stdout.contains("PlanCreated"));
    }

    #[test]
    fn inject_event_request_no_session_id_returns_err() {
        let runner = TestAgentRunner::new(vec![]);
        let log = TestLoomLog::default();

        let result = inject_event_request(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &None,
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "listener context".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn inject_event_request_runner_error_propagates() {
        let runner = TestAgentRunner::new(vec![Err(err_fatal())]);
        let log = TestLoomLog::default();

        let result = inject_event_request(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &Some("sess-abc".to_string()),
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "listener context".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn inject_event_request_repeats_listener_context() {
        let listener = concat!(
            "## Agent Events\n",
            "\n",
            "Events you may emit:\n",
            "- `PhaseReady` — phase complete, next phase ready\n",
            "- `ImplementationNote` — implementation discovery",
        )
        .to_string();

        let runner = TestAgentRunner::new(vec![Ok(ok_output("response"))]);
        let log = TestLoomLog::default();

        let _result = inject_event_request(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &Some("sess-abc".to_string()),
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            listener,
            "Modified".to_string(),
            Some("k1".to_string()),
            None,
        );

        let contexts = runner.contexts();
        let prompt = &contexts[0].prompt;
        // Prompt repeats the full listener context
        assert!(
            prompt.contains("Your previous response did not contain any agent event blocks"),
            "prompt should have preamble"
        );
        assert!(
            prompt.contains("## Agent Events"),
            "prompt should include listener context"
        );
        assert!(
            prompt.contains("`PhaseReady`"),
            "prompt should list PhaseReady from listener context"
        );
        assert!(
            prompt.contains("`ImplementationNote`"),
            "prompt should list ImplementationNote from listener context"
        );
    }

    #[test]
    fn inject_event_request_does_not_resend_profile_prompt() {
        let runner = TestAgentRunner::new(vec![Ok(ok_output("response"))]);
        let log = TestLoomLog::default();

        let _result = inject_event_request(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &Some("sess-abc".to_string()),
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "listener context".to_string(),
            "Modified".to_string(),
            Some("k1".to_string()),
            None,
        );

        let contexts = runner.contexts();
        // Profile prompt is not resent — session already has context
        assert!(
            contexts[0].profile_prompt.is_empty(),
            "profile prompt should be empty (not resent): {}",
            contexts[0].profile_prompt
        );
    }

    #[test]
    fn inject_event_request_uses_session_id_from_first_invocation() {
        let runner = TestAgentRunner::new(vec![Ok(ok_output("response"))]);
        let log = TestLoomLog::default();

        let _result = inject_event_request(
            &runner,
            &log,
            &make_loom_id(),
            &make_knot_id(),
            &make_strand_path(),
            &Some("sess-abc".to_string()),
            AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
            },
            "listener context".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            None,
        );

        let contexts = runner.contexts();
        assert_eq!(contexts.len(), 1);
        let extra_args = &contexts[0].agent_config.extra_args;
        assert!(
            extra_args.contains(&"--session-id".to_string()),
            "extra_args should contain --session-id: {:?}",
            extra_args
        );
        assert!(
            extra_args.contains(&"sess-abc".to_string()),
            "extra_args should contain session ID: {:?}",
            extra_args
        );
    }

    // ── Plan 081: inactivity kill restarts with the blocking-call note ──

    /// Plan 081: build an inactivity error with the given session ID
    /// (300s silent in a 300s window).
    fn err_inactivity(sid: Option<&str>) -> PortError {
        PortError::AgentInactivity {
            message: "no output for 300s (inactivity window 300s) (mock)"
                .to_string(),
            silent_secs: 300,
            window_secs: 300,
            blocked_call: None,
            session_id: sid.map(str::to_string),
        }
    }

    /// Plan 081: an inactivity kill with a captured session ID re-enters
    /// the same session; the retry prompt carries the inactivity restart
    /// note (with the secs values), NOT the bare final-response request.
    #[test]
    fn inactivity_retry_reenters_session_with_note() {
        let runner = TestAgentRunner::new(vec![
            Err(err_inactivity(Some("sess-inact"))),
            Ok(ok_output("done")),
        ]);
        let log = TestLoomLog::default();

        let result = execute_no_budget(&runner, &log);
        assert!(
            result.is_ok(),
            "should succeed on the retry: {:?}",
            result.err()
        );
        assert_eq!(runner.call_count(), 2);

        let contexts = runner.contexts();
        assert_eq!(contexts.len(), 2);

        // Second call re-enters the same session.
        let extra_args = &contexts[1].agent_config.extra_args;
        assert!(
            extra_args.contains(&"--session-id".to_string()),
            "retry should carry --session-id: {extra_args:?}"
        );
        assert!(
            extra_args.contains(&"sess-inact".to_string()),
            "retry should carry the session ID: {extra_args:?}"
        );

        // Retry prompt carries the inactivity note with the secs values
        // (300s silent, 300s window → "5-minute window" phrasing), and
        // NOT the bare 078 final-response request.
        let prompt = &contexts[1].prompt;
        assert!(
            prompt.contains("blocked for more than 300 seconds"),
            "retry prompt should carry the inactivity note: {prompt}"
        );
        assert!(
            prompt.contains("5-minute window"),
            "retry prompt should name the window in minutes: {prompt}"
        );
        assert!(
            !prompt.contains(FINAL_RESPONSE_REQUEST),
            "inactivity retry must not carry the bare 078 text: {prompt}"
        );

        // Loom-log: AgentInactivity { attempt: 1 } + SessionResumed { attempt: 1 }
        let events = log.events();
        let inactivity_attempt = events
            .iter()
            .find_map(|e| {
                if let LoomEvent::AgentInactivity { attempt, .. } = e {
                    Some(*attempt)
                } else {
                    None
                }
            });
        assert_eq!(
            inactivity_attempt,
            Some(1),
            "expected AgentInactivity {{ attempt: 1 }}, got: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, LoomEvent::SessionResumed { attempt: 1, .. })),
            "expected SessionResumed {{ attempt: 1 }}: {events:?}"
        );
    }

    /// Plan 081: an inactivity kill WITHOUT a session ID is still
    /// retried — the one deliberate gate exception (a fresh restart is
    /// safe: knots are idempotent). The retry prompt carries the note;
    /// no `--session-id` is passed.
    #[test]
    fn inactivity_retry_fresh_without_session() {
        let runner = TestAgentRunner::new(vec![
            Err(err_inactivity(None)),
            Ok(ok_output("done")),
        ]);
        let log = TestLoomLog::default();

        let result = execute_no_budget(&runner, &log);
        assert!(
            result.is_ok(),
            "should succeed on the fresh retry: {:?}",
            result.err()
        );
        assert_eq!(runner.call_count(), 2);

        let contexts = runner.contexts();
        assert_eq!(contexts.len(), 2);

        let extra_args = &contexts[1].agent_config.extra_args;
        assert!(
            !extra_args.contains(&"--session-id".to_string()),
            "fresh restart must not pass --session-id: {extra_args:?}"
        );
        assert!(
            contexts[1].prompt.contains("blocked for more than 300 seconds"),
            "retry prompt should carry the inactivity note: {}",
            contexts[1].prompt
        );
    }

    /// Plan 081 regression (078): a plain timeout retry still carries
    /// the `FINAL_RESPONSE_REQUEST` — only inactivity kills swap the
    /// note for the blocking-call text.
    #[test]
    fn inactivity_note_replaces_final_response_request() {
        let runner = TestAgentRunner::new(vec![
            Err(err_timeout("sess-abc")),
            Ok(ok_output("done")),
        ]);
        let log = TestLoomLog::default();

        let result = execute_no_budget(&runner, &log);
        assert!(
            result.is_ok(),
            "should succeed on the retry: {:?}",
            result.err()
        );

        let contexts = runner.contexts();
        assert_eq!(contexts.len(), 2);
        let prompt = &contexts[1].prompt;
        assert!(
            prompt.contains(FINAL_RESPONSE_REQUEST),
            "timeout retry should still carry the final-response request: {prompt}"
        );
        assert!(
            !prompt.contains("blocked for more than"),
            "timeout retry must not carry the inactivity note: {prompt}"
        );
    }

    /// Plan 081: when every attempt is an inactivity kill, the terminal
    /// error is `PortError::AgentInactivity` (cause-accurate exhaustion,
    /// per 077/078) — not `Timeout` and not `AgentNoResponse`. No profile
    /// timeout → MAX_RETRIES is the only bound: 11 agent calls, 11 ×
    /// AgentInactivity + 10 × SessionResumed in the loom-log.
    #[test]
    fn inactivity_exhausted_terminal() {
        let responses: Vec<Result<AgentOutput, PortError>> = (0..11)
            .map(|_| Err(err_inactivity(Some("sess-inact"))))
            .collect();
        let runner = TestAgentRunner::new(responses);
        let log = TestLoomLog::default();

        let result = execute_no_budget(&runner, &log);

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::AgentInactivity { message, .. } => {
                assert!(
                    message.contains("exhausted"),
                    "message should mention exhaustion: {message}"
                );
            }
            other => panic!("expected terminal AgentInactivity, got: {other:?}"),
        }

        // Runner called 11 times: initial + 10 retries
        assert_eq!(runner.call_count(), 11);

        // Loom-log: 11 × AgentInactivity + 10 × SessionResumed
        let events = log.events();
        let inactivity_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::AgentInactivity { .. }))
            .count();
        let resumed_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::SessionResumed { .. }))
            .count();
        assert_eq!(
            inactivity_count, 11,
            "expected 11 AgentInactivity events, got: {events:?}"
        );
        assert_eq!(
            resumed_count, 10,
            "expected 10 SessionResumed events, got: {events:?}"
        );
    }
}
