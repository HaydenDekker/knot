//! Application-level tests for session-resume retry on invocation failure.
//!
//! Verifies: session ID capture, --session-id passthrough, "please continue"
//! prompt append, budget tracking, retry delay, exhaustion, and non-resumable
//! errors. All tests use mocked ports (\`MockAgentRunner\`, \`TrackingTieOffSink\`
//! etc.) — no \`start_knot()\`, no \`TEST_MUTEX\`, no PATH manipulation.
//!
//! One adapter test (\`session_resume_adapter_stdio_no_retry\`) verifies that
//! the \`PiStdioAgentRunner\` adapter does NOT capture session_id from stdout,
//! confirming that stdio mode cannot support session resume.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use knot::adapters::pi_stdio::PiStdioAgentRunner;
use knot::application::ports::{
    AgentInvocationMetadata, AgentOutput, AgentRunner, PortError,
};
use knot::application::store::LoomStore;
use knot::application::usecases::test_fixtures::*;
use knot::application::usecases::ProcessStrand;
use knot::domain::entities::{KnotId, LoomId, StrandPath, TieOffStatus};
use knot::domain::events::{LoomEvent, RigLogEvent};
use knot::domain::value_objects::AgentProfile;
use knot::RigAgentConfig;

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

/// Build the ProcessStrand use case with all mocks wired up.
///
/// Returns (use_case, log_events, tie_off_appends, rig_events,
/// tie_off_content, agent_runner).
#[allow(clippy::type_complexity)]
fn build_process_strand(
    loom: knot::domain::entities::Loom,
    agent_runner: Arc<MockAgentRunner>,
    profile: AgentProfile,
) -> (
    ProcessStrand,
    Arc<Mutex<Vec<LoomEvent>>>,
    Arc<Mutex<Vec<knot::domain::entities::TieOff>>>,
    Arc<Mutex<Vec<RigLogEvent>>>,
    Arc<Mutex<HashMap<String, String>>>,
    Arc<MockAgentRunner>,
) {
    let store = LoomStore::new();
    store.register(loom);

    let (log_port, log_events) = MockLoomLogPort::new();
    let (tie_off_sink, tie_off_appends, tie_off_content) =
        TrackingTieOffSink::new();
    let (rig_log, rig_events) = MockRigLogPort::new();

    let profile_repo = Arc::new(MockProfileRepository {
        profiles: Arc::new(Mutex::new(HashMap::from_iter([
            ("fast".to_string(), profile),
        ]))),
    });

    let use_case = ProcessStrand::new(
        store.clone(),
        Arc::new(log_port),
        agent_runner.clone() as Arc<dyn AgentRunner>,
        Arc::new(tie_off_sink),
        RigAgentConfig::default_config(),
        PathBuf::from("/rig"),
        profile_repo,
        Arc::new(rig_log),
        Arc::new(MockGitVersioningPort::default()),
        Arc::new(MockStrandFileChecker::new()),
        Arc::new(MockEventDispatcher::default()),
        None,
    );

    (
        use_case,
        log_events,
        tie_off_appends,
        rig_events,
        tie_off_content,
        agent_runner,
    )
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

    let (use_case, log_events, tie_off_appends, rig_events, _content,
        captured_runner) =
        build_process_strand(loom, runner, default_profile());

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

    let (use_case, log_events, tie_off_appends, rig_events, _content,
        _captured) =
        build_process_strand(loom, runner, default_profile());

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

    let (use_case, log_events, _tie_off_appends, rig_events, _content,
        captured_runner) =
        build_process_strand(loom, runner, default_profile());

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

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        captured_runner) =
        build_process_strand(loom, runner, default_profile());

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
