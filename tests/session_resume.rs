//! Application-level tests for session-resume retry on invocation failure.
//!
//! Verifies: session ID capture, --session-id passthrough, the
//! final-response request prompt append (plan 078), budget tracking, retry
//! delay, exhaustion, and non-resumable errors. All tests use mocked ports
//! (\`MockAgentRunner\`, \`TrackingTieOffSink\` etc.) — no \`start_knot()\`, no
//! \`TEST_MUTEX\`, no PATH manipulation.
//!
//! One adapter test (\`session_resume_adapter_stdio_no_retry\`) verifies that
//! the \`PiStdioAgentRunner\` adapter does NOT capture session_id from stdout,
//! confirming that stdio mode cannot support session resume.

mod helpers;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use helpers::ProcessStrandBuilder;
use knot::adapters::pi_stdio::PiStdioAgentRunner;
use knot::application::in_memory_event_queue::InMemoryEventQueue;
use knot::application::ports::{
    AgentInvocationMetadata, AgentOutput, AgentRunner, PortError,
    StrandEventQueue,
};
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{KnotId, LoomId, StrandPath, TieOffStatus};
use knot::domain::events::{LoomEvent, RigLogEvent, StrandQueueAccessor};
use knot::domain::pending_event::{PendingEvent, PendingEventId};
use knot::domain::value_objects::AgentProfile;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a knot with the given ID and "fast" profile ref.
fn build_knot(id: &str) -> knot::domain::entities::Knot {
    build_knot_with_profile(id, "fast")
}

/// Build a loom with the given ID and knots.
fn build_loom(
    id: &str,
    knots: Vec<knot::domain::entities::Knot>,
) -> knot::domain::entities::Loom {
    knot::domain::entities::Loom {
        id: LoomId(id.to_string()),
        knots,
    }
}

/// Build a profile with a custom timeout (in seconds).
fn build_profile_with_timeout(timeout_secs: u64) -> AgentProfile {
    default_profile().with_timeout(Some(timeout_secs))
}

/// Build a StrandEvent::Created for the given loom/knot/strand.
fn created_event(
    loom_id: &str,
    knot_id: &str,
    strand_path: PathBuf,
) -> knot::domain::events::StrandEvent {
    knot::domain::events::StrandEvent::Created {
        loom_id: LoomId(loom_id.to_string()),
        knot_id: KnotId(knot_id.to_string()),
        strand_path: StrandPath(strand_path),
    }
}

/// Create a real strand file on disk (needed for Created events).
fn create_strand_file(
    dir: &tempfile::TempDir,
    name: &str,
    content: &str,
) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// Build an AgentOutput with session_id metadata.
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

/// Build a resumable timeout error with session_id.
fn err_timeout(sid: &str) -> PortError {
    PortError::Timeout {
        message: "timed out".to_string(),
        session_id: Some(sid.to_string()),
    }
}

/// Build an inactivity error (plan 081) with the given session ID
/// (300s silent in a 300s window).
fn err_inactivity(sid: Option<&str>) -> PortError {
    PortError::AgentInactivity {
        message: "no output for 300s (inactivity window 300s) (mock)".to_string(),
        silent_secs: 300,
        window_secs: 300,
        blocked_call: None,
        session_id: sid.map(str::to_string),
    }
}

/// Build a non-resumable (fatal) error.
fn err_fatal() -> PortError {
    PortError::CommandNotFound("pi not found".to_string())
}

// ── Application Tests: Session Resume ───────────────────────────────────

/// First invocation fails (timeout with session_id), retry succeeds.
/// Verifies: 2 agent calls, --session-id injected in second call,
/// SessionResumed + KnotCompleted in loom-log, no KnotFailed,
/// tie-off contains the resumed response.
#[test]
fn test_session_resume_success() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = Arc::new(MockAgentRunner::new_sequence(vec![
        Err(err_timeout("sess-resume")),
        Ok(ok_output_with_sid("resumed response", "sess-resume")),
    ]));

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        rig_events,
        tie_off_content: _content,
        agent_runner: captured_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(default_profile())
        .build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Verify 2 agent calls
    let contexts = captured_runner.get_captured_contexts();
    assert_eq!(contexts.len(), 2, "should have 2 agent calls");

    // Second call should have --session-id in extra_args
    let retry_ctx = &contexts[1];
    let args = &retry_ctx.agent_config.extra_args;
    assert!(
        args.contains(&"--session-id".to_string()),
        "retry should have --session-id in extra_args: {:?}",
        args
    );
    assert!(
        args.contains(&"sess-resume".to_string()),
        "retry should have session ID value in extra_args: {:?}",
        args
    );

    // Loom-log: SessionResumed + KnotCompleted, no KnotFailed
    let events = log_events.lock().unwrap();
    let has_resumed = events.iter()
        .any(|e| matches!(e, LoomEvent::SessionResumed { .. }));
    let has_completed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    let has_failed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotFailed { .. }));

    assert!(has_resumed, "should have SessionResumed");
    assert!(has_completed, "should have KnotCompleted");
    assert!(!has_failed, "should NOT have KnotFailed");

    // SessionResumed should have correct session_id and attempt
    let resumed = events.iter()
        .find_map(|e| {
            if let LoomEvent::SessionResumed {
                session_id, attempt, ..
            } = e {
                Some((session_id.clone(), *attempt))
            } else {
                None
            }
        });
    assert_eq!(
        resumed,
        Some(("sess-resume".to_string(), 1)),
        "SessionResumed should have session_id=sess-resume, attempt=1"
    );

    // Tie-off contains the resumed response
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "should have 1 tie-off append");
    assert_eq!(appends[0].status, TieOffStatus::Produced);
    assert!(
        appends[0].content.contains("resumed response"),
        "tie-off should contain resumed response: {}",
        appends[0].content
    );

    // No rig-log events on success
    let rig = rig_events.lock().unwrap();
    assert!(rig.is_empty(), "rig-log should be empty on success");
}

/// First fails, retry succeeds → loom-log has SessionResumed + KnotCompleted
/// + StrandProcessed, no KnotFailed. Transparent to the outer flow: the
/// tie-off is written normally with the retry's output.
#[test]
fn test_session_resume_transparent_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = Arc::new(MockAgentRunner::new_sequence(vec![
        Err(err_timeout("sess-transparent")),
        Ok(ok_output_with_sid("transparent success", "sess-transparent")),
    ]));

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        rig_events,
        agent_runner: _captured,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(default_profile())
        .build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    let events = log_events.lock().unwrap();

    // Should have: SessionResumed, KnotCompleted, StrandProcessed
    // Must NOT have: KnotFailed
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::SessionResumed { .. })),
        "should have SessionResumed"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::StrandProcessed { .. })),
        "should have StrandProcessed"
    );
    assert!(
        !events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. })),
        "should NOT have KnotFailed"
    );

    // StrandProcessed should have no error
    let processed = events.iter()
        .find_map(|e| {
            if let LoomEvent::StrandProcessed { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        });
    assert!(
        processed == Some(None),
        "StrandProcessed should have no error"
    );

    // Tie-off contains the success content
    let appends = tie_off_appends.lock().unwrap();
    assert!(
        appends[0].content.contains("transparent success"),
        "tie-off should contain success output"
    );

    // No rig-log events
    let rig = rig_events.lock().unwrap();
    assert!(rig.is_empty(), "rig-log should be empty");
}

/// All retry attempts fail → retries exhausted → KnotFailed in loom-log,
/// TimeoutExceeded in rig-log. Verifies max retry count (10).
#[test]
fn test_session_resume_exhausted() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    // 11 errors: 1 initial + 10 retries (MAX_RETRIES)
    let responses: Vec<Result<AgentOutput, PortError>> = (0..11)
        .map(|_| Err(err_timeout("sess-exhausted")))
        .collect();

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = Arc::new(MockAgentRunner::new_sequence(responses));

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends: _tie_off_appends,
        rig_events,
        agent_runner: captured_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(default_profile())
        .build();

    // Set zero retry delay for fast test execution
    unsafe { std::env::set_var("KNOT_RETRY_DELAY_MS", "0"); }
    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();
    unsafe { std::env::remove_var("KNOT_RETRY_DELAY_MS"); }

    // 11 calls: 1 initial + 10 retries
    let contexts = captured_runner.get_captured_contexts();
    assert_eq!(contexts.len(), 11, "should have 11 agent calls (1 + 10 retries)");

    // Loom-log: KnotFailed + 10 SessionResumed events
    let events = log_events.lock().unwrap();
    let resumed_count = events.iter()
        .filter(|e| matches!(e, LoomEvent::SessionResumed { .. }))
        .count();
    assert_eq!(
        resumed_count, 10,
        "should have 10 SessionResumed events (MAX_RETRIES)"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. })),
        "should have KnotFailed"
    );

    // KnotFailed error should mention exhaustion
    let failed = events.iter()
        .find_map(|e| {
            if let LoomEvent::KnotFailed { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        });
    assert!(
        failed.as_ref().map(|e| e.contains("exhausted")).unwrap_or(false),
        "KnotFailed should mention exhausted"
    );

    // Rig-log: TimeoutExceeded
    let rig = rig_events.lock().unwrap();
    assert!(
        rig.iter().any(|e| matches!(e, RigLogEvent::TimeoutExceeded { .. })),
        "should have TimeoutExceeded in rig-log"
    );
}

/// Non-resumable error (CommandNotFound) → no retry attempted,
/// KnotFailed in loom-log, no SessionResumed.
#[test]
fn test_session_resume_non_resumable_error() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = Arc::new(MockAgentRunner::new_sequence(vec![Err(err_fatal())]));

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends: _tie_off_appends,
        rig_events: _rig_events,
        agent_runner: captured_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(default_profile())
        .build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Only 1 call (no retry)
    let contexts = captured_runner.get_captured_contexts();
    assert_eq!(contexts.len(), 1, "should have only 1 agent call (no retry)");

    // Loom-log: KnotFailed, no SessionResumed
    let events = log_events.lock().unwrap();
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. })),
        "should have KnotFailed"
    );
    assert!(
        !events.iter().any(|e| matches!(e, LoomEvent::SessionResumed { .. })),
        "should NOT have SessionResumed for non-resumable error"
    );
}

// ── Plan 078: Empty First Response Requests the Final Response ────────

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Build a `PendingEvent` with a unique ID for the given loom/knot/strand.
fn make_pending(
    loom_id: &str,
    knot_id: &str,
    strand_path: &Path,
) -> PendingEvent {
    PendingEvent {
        id: PendingEventId(format!(
            "1000000000000-{:04x}",
            EVENT_COUNTER.fetch_add(1, Ordering::SeqCst)
        )),
        kind: "Created".to_string(),
        loom_id: loom_id.to_string(),
        knot_id: knot_id.to_string(),
        strand_path: strand_path.to_string_lossy().into_owned(),
        queued_at: "2026-01-01T00:00:00".to_string(),
    }
}

/// First invocation ends abruptly (exit 0, empty final response, session
/// ID captured). Plan 078: Knot re-enters the session and requests the
/// final response; the nudged response is the tie-off — the strand
/// succeeds transparently.
/// Verifies: 2 agent calls, --session-id injected in second call,
/// KnotEmptyResponse + SessionResumed + KnotCompleted in loom-log,
/// no KnotFailed, tie-off contains the resumed response, rig-log empty.
#[test]
fn test_empty_response_requests_final_response() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = Arc::new(MockAgentRunner::new_sequence(vec![
        Ok(ok_output_with_sid("", "sess-empty")),
        Ok(ok_output_with_sid("final response after nudge", "sess-empty")),
    ]));

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        rig_events,
        tie_off_content: _content,
        agent_runner: captured_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(default_profile())
        .build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Verify 2 agent calls
    let contexts = captured_runner.get_captured_contexts();
    assert_eq!(contexts.len(), 2, "should have 2 agent calls");

    // Second call should have --session-id in extra_args
    let retry_ctx = &contexts[1];
    let args = &retry_ctx.agent_config.extra_args;
    assert!(
        args.contains(&"--session-id".to_string()),
        "retry should have --session-id in extra_args: {:?}",
        args
    );
    assert!(
        args.contains(&"sess-empty".to_string()),
        "retry should have session ID value in extra_args: {:?}",
        args
    );

    // Loom-log: KnotEmptyResponse + SessionResumed + KnotCompleted,
    // no KnotFailed
    let events = log_events.lock().unwrap();
    let has_empty = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotEmptyResponse { .. }));
    let has_resumed = events.iter()
        .any(|e| matches!(e, LoomEvent::SessionResumed { .. }));
    let has_completed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    let has_failed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotFailed { .. }));

    assert!(has_empty, "should have KnotEmptyResponse for the empty first turn");
    assert!(has_resumed, "should have SessionResumed for the nudge");
    assert!(has_completed, "should have KnotCompleted");
    assert!(!has_failed, "should NOT have KnotFailed");

    // SessionResumed should have correct session_id and attempt
    let resumed = events.iter()
        .find_map(|e| {
            if let LoomEvent::SessionResumed {
                session_id, attempt, ..
            } = e {
                Some((session_id.clone(), *attempt))
            } else {
                None
            }
        });
    assert_eq!(
        resumed,
        Some(("sess-empty".to_string(), 1)),
        "SessionResumed should have session_id=sess-empty, attempt=1"
    );

    // Tie-off contains the resumed response
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "should have 1 tie-off append");
    assert_eq!(appends[0].status, TieOffStatus::Produced);
    assert!(
        appends[0].content.contains("final response after nudge"),
        "tie-off should contain the nudged response: {}",
        appends[0].content
    );

    // No rig-log events on success
    let rig = rig_events.lock().unwrap();
    assert!(rig.is_empty(), "rig-log should be empty on success");
}

/// First invocation ends abruptly and every nudge also returns an empty
/// response → retries exhausted (10) → the terminal error is the cause:
/// `AgentNoResponse` — a failed tie-off (NOT a timeout, so no rig-log
/// event), with the full attempt count in the content. The queued event
/// is still consumed exactly once (late-removal invariant).
#[test]
fn test_empty_response_exhausted_writes_failed_tieoff() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    // 12 empty responses: 1 initial + 10 retries (MAX_RETRIES) + spare.
    let responses: Vec<Result<AgentOutput, PortError>> = (0..12)
        .map(|_| Ok(ok_output_with_sid("", "sess-exhausted")))
        .collect();

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = Arc::new(MockAgentRunner::new_sequence(responses));
    let queue = Arc::new(InMemoryEventQueue::new());

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        rig_events,
        agent_runner: captured_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(default_profile()) // no profile timeout budget
        .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
        .build();

    let pending = make_pending("review-loom", "review", &strand_path);
    queue.push(pending.clone());

    // Set zero retry delay for fast test execution
    unsafe { std::env::set_var("KNOT_RETRY_DELAY_MS", "0"); }
    use_case.execute_with_pending(&pending).unwrap();
    unsafe { std::env::remove_var("KNOT_RETRY_DELAY_MS"); }

    // 11 calls: 1 initial + 10 retries
    let contexts = captured_runner.get_captured_contexts();
    assert_eq!(
        contexts.len(),
        11,
        "should have 11 agent calls (1 + 10 retries)"
    );

    // Loom-log: 11 × KnotEmptyResponse + 10 × SessionResumed + KnotFailed
    let events = log_events.lock().unwrap();
    let empty_count = events.iter()
        .filter(|e| matches!(e, LoomEvent::KnotEmptyResponse { .. }))
        .count();
    assert_eq!(
        empty_count, 11,
        "should have 11 KnotEmptyResponse events (initial + 10 retries)"
    );
    let resumed_count = events.iter()
        .filter(|e| matches!(e, LoomEvent::SessionResumed { .. }))
        .count();
    assert_eq!(
        resumed_count, 10,
        "should have 10 SessionResumed events (MAX_RETRIES)"
    );

    // KnotFailed error should mention the cause and the attempt count
    let failed = events.iter()
        .find_map(|e| {
            if let LoomEvent::KnotFailed { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        });
    let failed = failed.expect("KnotFailed should be logged on exhaustion");
    assert!(
        failed.contains("no final response"),
        "KnotFailed should mention 'no final response', got: {failed}"
    );
    assert!(
        failed.contains("after 11 attempts"),
        "KnotFailed should count all attempts (1 + 10 retries), got: {failed}"
    );

    // Rig-log: empty — an exhausted empty-response nudge is a failure,
    // not a timeout (no TimeoutExceeded).
    let rig = rig_events.lock().unwrap();
    assert!(
        rig.is_empty(),
        "rig-log should be empty (not a timeout): {:?}",
        rig
    );

    // Tie-off: appended Failed with the cause in the content
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "should have 1 tie-off append");
    assert_eq!(appends[0].status, TieOffStatus::Failed);
    assert!(
        appends[0].content.contains("no final response"),
        "tie-off content should contain 'no final response': {}",
        appends[0].content
    );
    assert!(
        appends[0].content.contains("after 11 attempts"),
        "tie-off content should count all attempts: {}",
        appends[0].content
    );

    // Late-removal invariant: the queued event is consumed exactly once
    // even when the knot fails.
    assert!(
        queue.is_empty(),
        "queued event must be removed on failure (late removal)"
    );
}

// ── Adapter Test: Stdio does not capture session_id ────────────────────

/// `PiStdioAgentRunner` with a mock that returns JSON containing session_id.
/// Because stdio mode reads stdout as plain text, session_id is NOT
/// extracted from the JSON-L stream. The error returned by the runner
/// has no session_id, confirming stdio cannot support session resume.
#[test]
fn session_resume_adapter_stdio_no_retry() {
    let dir = tempfile::tempdir().unwrap();

    // Mock pi that outputs JSON-L (including session) but exits with error.
    // In stdio mode, the runner reads stdout as plain text — it does NOT
    // parse JSON-L, so session_id is never extracted.
    let script = r#"#!/usr/bin/env bash
cat > /dev/null
echo '{"type":"session","id":"stdio-sess-xyz"}'
echo '{"type":"agent_end","usage":{"input":10,"output":10,"cache_read":0,"cache_write":0,"total":20},"messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"error output"}]}]}'
exit 1
"#;
    let mock_path = dir.path().join("mock-pi-stdio");
    std::fs::write(&mock_path, script).unwrap();
    std::fs::set_permissions(&mock_path, PermissionsExt::from_mode(0o755))
        .unwrap();

    let runner = PiStdioAgentRunner::with_cli_path_and_timeout(
        mock_path.to_string_lossy().to_string(),
        Duration::from_secs(10),
    );

    let ctx = knot::application::ports::ExecutionContext {
        agent_config: knot::domain::value_objects::AgentConfig {
            goal: "test".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            tools: vec![],
            extra_args: vec![],
            thinking_level: None,
        },
        prompt: "test prompt".to_string(),
        profile_prompt: String::new(),
        strand_path: StrandPath(PathBuf::from("test.md")),
        event_type: "Created".to_string(),
        knot_name: None,
        timeout: None,
    };

    let result = runner.execute(ctx);
    assert!(result.is_err(), "should error for non-zero exit");

    let err = result.unwrap_err();
    match &err {
        PortError::AgentExecutionFailed { session_id, .. } => {
            assert!(
                session_id.is_none(),
                "stdio adapter should NOT capture session_id, got: {:?}",
                session_id
            );
        }
        other => panic!("expected AgentExecutionFailed, got {:?}", other),
    }

    // Verify the error is resumable but has no session_id — so
    // is_session_resumable() would return false (requires session_id).
    assert!(
        err.is_resumable(),
        "AgentExecutionFailed should be resumable"
    );
    assert!(
        err.session_id().is_none(),
        "Error should have no session_id"
    );
}

// ── Plan 081: Inactivity Kill Restarts with the Blocking-Call Note ──────

/// First invocation is killed for inactivity (with a session ID); the
/// restarted session succeeds. Verifies: 2 agent calls, `--session-id`
/// on the second invocation, the inactivity restart note in the captured
/// retry prompt (not the bare 078 text), AgentInactivity + SessionResumed
/// + KnotCompleted in the loom-log, tie-off `Produced`, rig-log empty.
#[test]
fn test_inactivity_restarts_with_note() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = Arc::new(MockAgentRunner::new_sequence(vec![
        Err(err_inactivity(Some("sess-inact"))),
        Ok(ok_output_with_sid("final after note", "sess-inact")),
    ]));

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        rig_events,
        agent_runner: captured_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(default_profile())
        .build();

    unsafe { std::env::set_var("KNOT_RETRY_DELAY_MS", "0"); }
    use_case
        .execute(created_event("review-loom", "review", strand_path))
        .unwrap();
    unsafe { std::env::remove_var("KNOT_RETRY_DELAY_MS"); }

    // Verify 2 agent calls
    let contexts = captured_runner.get_captured_contexts();
    assert_eq!(contexts.len(), 2, "should have 2 agent calls");

    // Second call re-enters the same session
    let args = &contexts[1].agent_config.extra_args;
    assert!(
        args.contains(&"--session-id".to_string()),
        "retry should have --session-id in extra_args: {args:?}"
    );
    assert!(
        args.contains(&"sess-inact".to_string()),
        "retry should have the session ID in extra_args: {args:?}"
    );

    // The retry prompt carries the inactivity note (with the secs
    // values), not the bare final-response request.
    let prompt = &contexts[1].prompt;
    assert!(
        prompt.contains("blocked for more than 300 seconds"),
        "retry prompt should carry the inactivity note: {prompt}"
    );
    assert!(
        prompt.contains("5-minute window"),
        "retry prompt should name the window in minutes: {prompt}"
    );

    // Loom-log: AgentInactivity + SessionResumed + KnotCompleted,
    // no KnotFailed
    let events = log_events.lock().unwrap();
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::AgentInactivity { .. })),
        "should have AgentInactivity: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::SessionResumed { .. })),
        "should have SessionResumed: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted: {events:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. })),
        "should NOT have KnotFailed: {events:?}"
    );

    // Tie-off: Produced with the restarted session's output
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "should have 1 tie-off append");
    assert_eq!(appends[0].status, TieOffStatus::Produced);
    assert!(
        appends[0].content.contains("final after note"),
        "tie-off should contain the restarted response: {}",
        appends[0].content
    );

    // No rig-log events on success
    let rig = rig_events.lock().unwrap();
    assert!(rig.is_empty(), "rig-log should be empty on success: {rig:?}");
}

/// Every attempt is an inactivity kill → retries exhausted → a deadline
/// did fire (`TimeoutSkipped`): no new tie-off section appended (prior
/// content intact), rig-log has `TimeoutExceeded` carrying the
/// inactivity cause, and the queued event is removed (late-removal
/// invariant).
#[test]
fn test_inactivity_exhausted_preserves_tieoff() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    // 12 inactivity kills: 1 initial + 10 retries (MAX_RETRIES) + spare.
    let responses: Vec<Result<AgentOutput, PortError>> = (0..12)
        .map(|_| Err(err_inactivity(Some("sess-inact"))))
        .collect();

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = Arc::new(MockAgentRunner::new_sequence(responses));
    let queue = Arc::new(InMemoryEventQueue::new());

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        rig_events,
        agent_runner: captured_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(default_profile()) // no profile timeout budget
        .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
        .build();

    let pending = make_pending("review-loom", "review", &strand_path);
    queue.push(pending.clone());

    // Set zero retry delay for fast test execution
    unsafe { std::env::set_var("KNOT_RETRY_DELAY_MS", "0"); }
    use_case.execute_with_pending(&pending).unwrap();
    unsafe { std::env::remove_var("KNOT_RETRY_DELAY_MS"); }

    // 11 calls: 1 initial + 10 retries
    let contexts = captured_runner.get_captured_contexts();
    assert_eq!(
        contexts.len(),
        11,
        "should have 11 agent calls (1 + 10 retries)"
    );

    // Tie-off: NO new section appended — prior content intact
    let appends = tie_off_appends.lock().unwrap();
    assert!(
        appends.is_empty(),
        "tie-off should NOT be appended on exhausted inactivity: {appends:?}"
    );

    // Rig-log: TimeoutExceeded carrying the inactivity cause
    let rig = rig_events.lock().unwrap();
    let exceeded = rig
        .iter()
        .find_map(|e| {
            if let RigLogEvent::TimeoutExceeded { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        })
        .expect("rig-log should have TimeoutExceeded: {rig:?}");
    assert!(
        exceeded.contains("inactivity"),
        "TimeoutExceeded should carry the inactivity cause: {exceeded}"
    );

    // Loom-log: KnotFailed (inactivity cause) + StrandProcessed { error }
    let events = log_events.lock().unwrap();
    let failed = events
        .iter()
        .find_map(|e| {
            if let LoomEvent::KnotFailed { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        })
        .expect("KnotFailed should be logged on exhaustion");
    assert!(
        failed.contains("inactivity"),
        "KnotFailed should carry the inactivity cause: {failed}"
    );
    let processed_error = events
        .iter()
        .find_map(|e| {
            if let LoomEvent::StrandProcessed { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        })
        .expect("StrandProcessed should be logged");
    assert!(
        processed_error.is_some(),
        "StrandProcessed should carry an error on exhaustion"
    );

    // Late-removal invariant: the queued event is consumed exactly once
    // even when the knot fails.
    assert!(
        queue.is_empty(),
        "queued event must be removed on failure (late removal)"
    );
}
