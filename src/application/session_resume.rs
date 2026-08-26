//! Session-resume retry module.
//!
//! When an agent invocation fails with a resumable error (timeout, mid-stream
//! failure) — or ends its turn abruptly without a final response (plan 078) —
//! and a session ID was captured, this module retries the invocation using
//! `--session-id <id>` to continue the same Pi session. The retry prompt is
//! the original prompt plus the final-response request
//! ([`FINAL_RESPONSE_REQUEST`]). Retries are limited to 10 attempts or the
//! profile's overall timeout budget, whichever comes first.

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

/// Timestamp helper for loom-log events.
fn format_timestamp() -> String {
    crate::adapters::logging::format_timestamp()
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
            return Ok(output);
        }
    } else {
        let err = result.unwrap_err();

        // Check if the first failure is resumable.
        // The session_id for retry comes from the error itself (captured by
        // the JSON adapter from Pi's first JSONL line before generation
        // starts). If the error carries no session_id, we cannot resume.
        let error_session_id = err.session_id().cloned();
        if !err.is_resumable() || error_session_id.is_none() {
            // Not resumable or no session_id — extract what we can and
            // return
            if let Some(sid) = err.session_id() {
                *session_id = Some(sid.clone());
            }
            return Err(err);
        }

        // Capture session_id from error for retry.
        // At this point we know error_session_id is Some (checked above).
        *session_id = error_session_id;
        first_error = err;
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
        // Append --session-id to extra_args and the final-response
        // request (plan 078) to the prompt.
        if let Some(sid) = session_id {
            agent_config.extra_args.push("--session-id".to_string());
            agent_config.extra_args.push(sid.clone());
        }
        prompt.push_str("\n\n");
        prompt.push_str(FINAL_RESPONSE_REQUEST);

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
            }
        }
    }

    // Exhausted all retries
    Err(PortError::Timeout {
        message: format!(
            "session resume exhausted {} retries{}",
            MAX_RETRIES,
            profile_timeout
                .map(|t| format!(" (overall timeout: {}s)", t.as_secs()))
                .unwrap_or_default(),
        ),
        session_id: session_id.clone(),
    })
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
}
