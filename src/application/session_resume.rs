//! Session-resume retry module.
//!
//! When an agent invocation fails with a resumable error (timeout, mid-stream
//! failure) and a session ID was captured, this module retries the invocation
//! using `--session-id <id>` to continue the same Pi session. Retries are
//! limited to 10 attempts or the profile's overall timeout budget, whichever
//! comes first.

use std::time::{Duration, Instant};

use crate::application::ports::{
    AgentOutput, AgentRunner, ExecutionContext,
    LoomLogPort, PortError,
};
use crate::domain::entities::{KnotId, LoomId, StrandPath};
use crate::domain::events::LoomEvent;

/// Maximum number of retry attempts (not counting the initial attempt).
const MAX_RETRIES: u32 = 10;

/// Default delay between retry attempts to allow transient errors to recover.
const RETRY_DELAY: Duration = Duration::from_secs(10);

/// Minimum remaining time (seconds) required to attempt a retry.
///
/// If less than this amount of budget remains, the loop bails rather
/// than starting an attempt that is almost certain to time out.
const MIN_REMAINING_SECS: u64 = 5;

/// Timestamp helper for loom-log events.
fn format_timestamp() -> String {
    crate::adapters::logging::format_timestamp()
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Attempt agent execution with automatic session-resume retry.
///
/// Returns [`Ok(AgentOutput)`] on success (first attempt or after N retries).
/// Returns [`Err(PortError)`] when retries are exhausted or the overall
/// timeout budget is expired.
///
/// `SessionResumed` events are appended to `loom_log` for each retry attempt.
pub fn execute_with_resume(
    agent_runner: &dyn AgentRunner,
    loom_log: &dyn LoomLogPort,
    loom_id: &LoomId,
    knot_id: &KnotId,
    strand_path: &StrandPath,
    session_id: &mut Option<String>,
    cli_args: Vec<String>,
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
        cli_args,
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
    mut cli_args: Vec<String>,
    mut prompt: String,
    _strand_file_ref: Option<StrandPath>,
    profile_prompt: String,
    event_type: String,
    knot_name: Option<String>,
    profile_timeout: Option<Duration>,
    retry_delay: Duration,
) -> Result<AgentOutput, PortError> {
    let start = Instant::now();

    // --- First attempt (no session ID) ---
    let ctx = build_retry_context(
        cli_args.clone(),
        prompt.clone(),
        strand_path.clone(),
        profile_prompt.clone(),
        event_type.clone(),
        knot_name.clone(),
        profile_timeout,
        start,
    );

    let result = agent_runner.execute(ctx);

    if let Ok(output) = &result {
        // Capture session_id from successful output metadata
        if let Some(ref metadata) = output.metadata {
            if let Some(ref sid) = metadata.session_id {
                *session_id = Some(sid.clone());
            }
        }
        return Ok(output.clone());
    }

    let mut first_error = result.unwrap_err();

    // Check if the first failure is resumable.
    // The session_id for retry comes from the error itself (captured by the
    // JSON adapter from Pi's first JSONL line before generation starts).
    // If the error carries no session_id, we cannot resume.
    let error_session_id = first_error.session_id().cloned();
    if !first_error.is_resumable() || error_session_id.is_none() {
        // Not resumable or no session_id — extract what we can and return
        if let Some(sid) = first_error.session_id() {
            *session_id = Some(sid.clone());
        }
        return Err(first_error);
    }

    // Capture session_id from error for retry.
    // At this point we know error_session_id is Some (checked above).
    *session_id = error_session_id;

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

        // Prepare cli_args and prompt for retry
        (cli_args, prompt) = prepare_retry(cli_args, prompt, session_id);

        // Log SessionResumed event
        loom_log.append(LoomEvent::SessionResumed {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            session_id: session_id.clone().unwrap_or_default(),
            attempt,
            timestamp: format_timestamp(),
        })?;

        // Build context with remaining time and execute
        let timeout = profile_timeout.as_ref().map(|t| t.saturating_sub(start.elapsed()));
        let ctx = build_retry_context(
            cli_args.clone(),
            prompt.clone(),
            strand_path.clone(),
            profile_prompt.clone(),
            event_type.clone(),
            knot_name.clone(),
            timeout,
            start,
        );

        match agent_runner.execute(ctx) {
            Ok(output) => {
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

// ── Helpers ────────────────────────────────────────────────────────────────

/// Build an [`ExecutionContext`] for a retry attempt with remaining timeout.
fn build_retry_context(
    cli_args: Vec<String>,
    prompt: String,
    strand_path: StrandPath,
    profile_prompt: String,
    event_type: String,
    knot_name: Option<String>,
    timeout: Option<Duration>,
    _start: Instant,
) -> ExecutionContext {
    ExecutionContext {
        cli_path: String::new(),
        cli_args,
        prompt,
        profile_prompt,
        strand_path,
        event_type,
        knot_name,
        timeout,
    }
}

/// Prepare `cli_args` and `prompt` for the next retry attempt.
///
/// Appends `--session-id <id>` to cli_args and `"please continue"` to the
/// prompt.
fn prepare_retry(
    mut cli_args: Vec<String>,
    mut prompt: String,
    session_id: &Option<String>,
) -> (Vec<String>, String) {
    if let Some(sid) = session_id {
        cli_args.push("--session-id".to_string());
        cli_args.push(sid.clone());
    }
    prompt.push_str("\n\nplease continue");
    (cli_args, prompt)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::AgentInvocationMetadata;
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

    /// In-memory loom log that records all appended events.
    #[derive(Default)]
    struct TestLoomLog {
        events: Arc<Mutex<Vec<LoomEvent>>>,
    }

    impl TestLoomLog {
        fn events(&self) -> Vec<LoomEvent> {
            self.events.lock().unwrap().clone()
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
            vec!["--goal".to_string(), "review".to_string()],
            "Review this document".to_string(),
            Some(make_strand_path()),
            "You are a reviewer.".to_string(),
            "Created".to_string(),
            Some("k1".to_string()),
            Some(Duration::from_secs(timeout_secs)),
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
            vec!["--goal".to_string(), "review".to_string()],
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
            vec!["--goal".to_string(), "review".to_string()],
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

        // Second context (retry) has --session-id appended
        let retry_ctx = &contexts[1];
        let args = &retry_ctx.cli_args;

        // Original args preserved
        assert!(args.contains(&"--goal".to_string()));
        assert!(args.contains(&"review".to_string()));

        // --session-id appended
        assert!(args.contains(&"--session-id".to_string()));
        assert!(args.contains(&"sess-abc".to_string()));
    }

    #[test]
    fn retry_appends_please_continue() {
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

        // Retry: prompt includes "please continue"
        assert!(
            contexts[1].prompt.contains("please continue"),
            "Retry prompt should contain 'please continue', got: {}",
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
            vec!["--goal".to_string(), "review".to_string()],
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
            vec!["--goal".to_string(), "review".to_string()],
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
}
