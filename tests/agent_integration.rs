//! Application-level integration tests for agent execution.
//!
//! Verifies agent invocation, tie-off output, error handling, and
//! state transitions by constructing `ProcessStrand` with mocked ports
//! (`TrackingTieOffSink`, `MockAgentRunner`, `MockLoomLogPort` etc.).
//!
//! No `start_knot()` calls, no `TEST_MUTEX`, no PATH manipulation —
//! all ports are mocked, tests run fully parallel, and complete in
//! sub-millisecond time.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use knot::application::ports::{
    AgentOutput, AgentRunner, PortError,
};
use knot::application::store::LoomStore;
use knot::application::usecases::ProcessStrand;
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{
    Knot, KnotId, Loom, LoomId, StrandPath, TieOff, TieOffStatus,
};
use knot::domain::events::{LoomEvent, RigLogEvent};
use knot::RigAgentConfig;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a knot with the given ID and "fast" profile ref.
fn build_knot(id: &str) -> Knot {
    build_knot_with_profile(id, "fast")
}

/// Build a loom with the given ID and knots.
fn build_loom(id: &str, knots: Vec<Knot>) -> Loom {
    Loom {
        id: LoomId(id.to_string()),
        knots,
    }
}

/// Build the ProcessStrand use case with all mocks wired up.
///
/// Returns a tuple of (use_case, log_events, tie_off_appends, rig_events,
/// tie_off_content, agent_runner).
#[allow(clippy::type_complexity)]
fn build_process_strand(
    loom: Loom,
    agent_runner: Arc<MockAgentRunner>,
) -> (
    ProcessStrand,
    Arc<Mutex<Vec<LoomEvent>>>,
    Arc<Mutex<Vec<TieOff>>>,
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
            ("fast".to_string(), default_profile()),
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

/// Build a `StrandEvent::Created` for the given loom/knot/strand.
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

/// Build a `StrandEvent::Deleted` for the given loom/knot/strand.
fn deleted_event(
    loom_id: &str,
    knot_id: &str,
    strand_path: PathBuf,
) -> knot::domain::events::StrandEvent {
    knot::domain::events::StrandEvent::Deleted {
        loom_id: LoomId(loom_id.to_string()),
        knot_id: KnotId(knot_id.to_string()),
        strand_path: StrandPath(strand_path),
    }
}

/// Build a successful agent output mock runner.
fn success_runner(output: &str) -> Arc<MockAgentRunner> {
    Arc::new(MockAgentRunner::new(Ok(AgentOutput {
        stdout: output.to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    })))
}

/// Build a failing agent execution mock runner.
fn failure_runner(message: &str) -> Arc<MockAgentRunner> {
    Arc::new(MockAgentRunner::new(Err(
        PortError::AgentExecutionFailed {
            message: message.to_string(),
            session_id: None,
        },
    )))
}

/// Create a real strand file on disk (needed for Created/Modified events
/// which check file existence via `StrandPath::should_process`).
fn create_strand_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ── Test 1: agent_execution_produces_tie_off ─────────────────────────────

/// `ProcessStrand` with `TrackingTieOffSink`: on success, tie-off is
/// appended with the agent's stdout content.
#[test]
fn agent_execution_produces_tie_off() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "new feature");

    let loom = build_loom("test-loom", vec![build_knot("review")]);
    let runner = success_runner("agent response here");

    let (use_case, _log_events, tie_off_appends, _rig_events, _content,
        _captured) = build_process_strand(loom, runner);

    let event = created_event("test-loom", "review", strand_path);

    let result = use_case.execute(event);
    assert!(result.is_ok());

    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "should have 1 tie-off append");
    let tie_off = &appends[0];
    assert!(
        tie_off.content.contains("agent response here"),
        "tie-off content should contain agent output"
    );
    assert_eq!(tie_off.status, TieOffStatus::Produced);
}

// ── Test 2: agent_execution_append_mode_tie_offs ─────────────────────────

/// Multiple `ProcessStrand` calls append to the same tie-off sink,
/// producing a history of agent outputs.
#[test]
fn agent_execution_append_mode_tie_offs() {
    let dir = tempfile::tempdir().unwrap();
    let strand1 = create_strand_file(&dir, "feature1.md", "feature 1");
    let strand2 = create_strand_file(&dir, "feature2.md", "feature 2");

    let loom = build_loom("test-loom", vec![build_knot("review")]);
    let runner = success_runner("review v1");

    let (use_case, _log_events, tie_off_appends, _rig_events,
        tie_off_content, _captured) = build_process_strand(loom, runner);

    // First strand
    use_case
        .execute(created_event("test-loom", "review", strand1))
        .unwrap();

    // Second strand — same knot, same tie-off path
    use_case
        .execute(created_event("test-loom", "review", strand2))
        .unwrap();

    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 2, "should have 2 tie-off appends");
    assert!(
        appends[0].content.contains("review v1"),
        "first append should contain agent output"
    );
    assert!(
        appends[1].content.contains("review v1"),
        "second append should contain agent output"
    );

    // Content map tracks the latest write
    let content = tie_off_content.lock().unwrap();
    let latest = content
        .get("/rig/tie-offs/test-loom/tie-off-review.md")
        .expect("tie-off path should be in content map");
    assert!(
        latest.contains("review v1"),
        "tie-off content should contain agent output"
    );
}

// ── Test 3: agent_execution_updates_state_file ───────────────────────────

/// On successful processing, loom-log records KnotCompleted with
/// `tie_off_path` and StrandProcessed — the data the state writer
/// derives `last_tie_off_path` and `last_strand_path` from.
#[test]
fn agent_execution_updates_state_file() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("test-loom", vec![build_knot("review")]);
    let runner = success_runner("review done");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured) = build_process_strand(loom, runner);

    use_case
        .execute(created_event("test-loom", "review", strand_path))
        .unwrap();

    let events = log_events.lock().unwrap();

    // Verify KnotCompleted has tie_off_path
    let completed: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let LoomEvent::KnotCompleted {
                tie_off_path, strand_path, ..
            } = e
            {
                Some((tie_off_path.clone(), strand_path.clone()))
            } else {
                None
            }
        })
        .collect();

    assert!(!completed.is_empty(), "should have KnotCompleted event");
    let (tie_off_path, strand_path) = &completed[0];
    assert!(
        tie_off_path.0.to_string_lossy().contains("tie-off-review.md"),
        "tie_off_path should reference tie-off-review.md"
    );
    assert!(
        strand_path.0.file_name().map(|f| f.to_string_lossy())
            == Some("feature.md".into()),
        "strand_path should reference feature.md"
    );
}

// ── Test 4: agent_failure_records_error_in_state ────────────────────────

/// On agent failure, loom-log records KnotFailed with error message
/// and StrandProcessed with error — the data the state writer derives
/// `status: "failed"` and `last_error` from.
#[test]
fn agent_failure_records_error_in_state() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("test-loom", vec![build_knot("review")]);
    let runner = failure_runner("agent crash");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured) = build_process_strand(loom, runner);

    use_case
        .execute(created_event("test-loom", "review", strand_path))
        .unwrap();

    let events = log_events.lock().unwrap();

    // Verify KnotFailed has error
    let failed: Vec<String> = events
        .iter()
        .filter_map(|e| {
            if let LoomEvent::KnotFailed { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        })
        .collect();

    assert!(!failed.is_empty(), "should have KnotFailed event");
    assert!(
        failed[0].contains("crash"),
        "error should contain crash detail"
    );

    // Verify StrandProcessed has error
    let processed_errors: Vec<String> = events
        .iter()
        .filter_map(|e| {
            if let LoomEvent::StrandProcessed { error, .. } = e {
                error.clone()
            } else {
                None
            }
        })
        .collect();

    assert!(!processed_errors.is_empty(), "should have StrandProcessed with error");
    assert!(
        processed_errors[0].contains("crash"),
        "StrandProcessed error should contain crash detail"
    );
}

// ── Test 5: agent_failure_records_loom_log_entry ─────────────────────────

/// On agent failure, `MockLoomLogPort` captures KnotFailed event
/// with the error message.
#[test]
fn agent_failure_records_loom_log_entry() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("test-loom", vec![build_knot("review")]);
    let runner = failure_runner("execution failed");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured) = build_process_strand(loom, runner);

    use_case
        .execute(created_event("test-loom", "review", strand_path))
        .unwrap();

    let events = log_events.lock().unwrap();

    // Should have KnotProcessing → KnotFailed → StrandProcessed
    assert!(
        events.len() >= 3,
        "should have at least 3 loom-log events on failure"
    );

    // Find KnotFailed
    let has_failed = events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. }));
    assert!(has_failed, "should have KnotFailed event");

    // Find the error in KnotFailed
    for event in events.iter() {
        if let LoomEvent::KnotFailed { error, .. } = event {
            assert!(
                error.contains("execution failed"),
                "KnotFailed error should contain original message"
            );
        }
    }
}

// ── Test 6: tie_off_contains_agent_output ────────────────────────────────

/// Tie-off content matches the agent's stdout exactly (multi-line).
#[test]
fn tie_off_contains_agent_output() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("test-loom", vec![build_knot("review")]);
    let runner = success_runner("line one\nline two\nline three");

    let (use_case, _log_events, tie_off_appends, _rig_events, _content,
        _captured) = build_process_strand(loom, runner);

    use_case
        .execute(created_event("test-loom", "review", strand_path))
        .unwrap();

    let appends = tie_off_appends.lock().unwrap();
    let tie_off = &appends[0];
    assert!(
        tie_off.content.contains("line one"),
        "tie-off should contain first line"
    );
    assert!(
        tie_off.content.contains("line two"),
        "tie-off should contain second line"
    );
}

// ── Test 7: agent_handles_deleted_strand ─────────────────────────────────

/// Deleted strand events are processed: KnotCompleted is logged
/// and the agent is invoked with a deletion notice in the prompt
/// (no `@{strand_path}` in CLI args).
#[test]
fn agent_handles_deleted_strand() {
    let loom = build_loom("test-loom", vec![build_knot("review")]);
    let runner = success_runner("deletion summary");

    let (use_case, log_events, tie_off_appends, _rig_events, _content,
        captured) = build_process_strand(loom, runner);

    let event = deleted_event(
        "test-loom",
        "review",
        PathBuf::from("strands/feature.md"),
    );

    let result = use_case.execute(event);
    assert!(result.is_ok());

    // Agent was invoked
    let ctx = captured.get_captured_ctx().expect("ctx should be captured");
    assert!(
        ctx.prompt.contains("This file was deleted"),
        "prompt should contain deletion notice: {}",
        ctx.prompt
    );
    // No @file reference for deleted events
    let has_at_ref = ctx.agent_config.extra_args
        .iter()
        .any(|arg| arg.starts_with('@'));
    assert!(
        !has_at_ref,
        "Deleted events must NOT contain @file reference"
    );

    // Loom-log has KnotCompleted and StrandProcessed
    let events = log_events.lock().unwrap();
    let has_completed = events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    assert!(has_completed, "should have KnotCompleted for deleted strand");

    // Tie-off was written
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "tie-off should be appended for deleted strand");
}

// ── Test 8: agent_handles_multiple_looms_independently ──────────────────

/// Two separate `ProcessStrand` instances, each with their own loom,
/// process independently with isolated mock state.
#[test]
fn agent_handles_multiple_looms_independently() {
    let dir = tempfile::tempdir().unwrap();
    let strand1 = create_strand_file(&dir, "feature.md", "feature");
    let strand2 = create_strand_file(&dir, "task.md", "task");

    // Loom 1: review
    let loom1 = build_loom("review-loom", vec![build_knot("review")]);
    let runner1 = success_runner("review output");

    let (use_case1, log_events1, tie_off_appends1, _rig_events1,
        _content1, _captured1) = build_process_strand(loom1, runner1);

    // Loom 2: planning
    let loom2 = build_loom("planning-loom", vec![build_knot("plan")]);
    let runner2 = success_runner("planning output");

    let (use_case2, log_events2, tie_off_appends2, _rig_events2,
        _content2, _captured2) = build_process_strand(loom2, runner2);

    // Process strands in both looms independently
    use_case1
        .execute(created_event("review-loom", "review", strand1))
        .unwrap();

    use_case2
        .execute(created_event("planning-loom", "plan", strand2))
        .unwrap();

    // Verify loom 1: KnotCompleted in its log, review output in its tie-off
    let events1 = log_events1.lock().unwrap();
    let has_completed1 = events1.iter()
        .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    assert!(has_completed1, "loom 1 should have KnotCompleted");

    let appends1 = tie_off_appends1.lock().unwrap();
    assert_eq!(appends1.len(), 1);
    assert!(
        appends1[0].content.contains("review output"),
        "loom 1 tie-off should have review output"
    );

    // Verify loom 2: KnotCompleted in its log, planning output in its tie-off
    let events2 = log_events2.lock().unwrap();
    let has_completed2 = events2.iter()
        .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    assert!(has_completed2, "loom 2 should have KnotCompleted");

    let appends2 = tie_off_appends2.lock().unwrap();
    assert_eq!(appends2.len(), 1);
    assert!(
        appends2[0].content.contains("planning output"),
        "loom 2 tie-off should have planning output"
    );
}

// ── Test 9: agent_state_transitions_through_processing ──────────────────

/// Loom-log shows the state transition sequence:
/// KnotProcessing → KnotCompleted (on success).
#[test]
fn agent_state_transitions_through_processing() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("test-loom", vec![build_knot("review")]);
    let runner = success_runner("done");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured) = build_process_strand(loom, runner);

    use_case
        .execute(created_event("test-loom", "review", strand_path))
        .unwrap();

    let events = log_events.lock().unwrap();

    // Verify KnotProcessing is present
    let has_processing = events.iter().any(|e| {
        matches!(e, LoomEvent::KnotProcessing { .. })
    });
    assert!(has_processing, "should have KnotProcessing event");

    // Verify KnotCompleted is present
    let has_completed = events.iter().any(|e| {
        matches!(e, LoomEvent::KnotCompleted { .. })
    });
    assert!(has_completed, "should have KnotCompleted event");

    // Verify order: KnotProcessing before KnotCompleted
    let processing_idx = events.iter().position(|e| {
        matches!(e, LoomEvent::KnotProcessing { .. })
    });
    let completed_idx = events.iter().position(|e| {
        matches!(e, LoomEvent::KnotCompleted { .. })
    });

    assert!(
        processing_idx.is_some() && completed_idx.is_some(),
        "both KnotProcessing and KnotCompleted should exist"
    );
    assert!(
        processing_idx.unwrap() < completed_idx.unwrap(),
        "KnotProcessing should come before KnotCompleted"
    );
}

// ── Test 10: strand_processed_no_error_on_success ────────────────────────

/// StrandProcessed event has `error: None` on successful processing.
#[test]
fn strand_processed_no_error_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("test-loom", vec![build_knot("review")]);
    let runner = success_runner("ok");

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured) = build_process_strand(loom, runner);

    use_case
        .execute(created_event("test-loom", "review", strand_path))
        .unwrap();

    let events = log_events.lock().unwrap();

    // Find StrandProcessed events
    let processed: Vec<Option<String>> = events
        .iter()
        .filter_map(|e| {
            if let LoomEvent::StrandProcessed { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        })
        .collect();

    assert!(
        !processed.is_empty(),
        "should have StrandProcessed events"
    );

    // Last StrandProcessed should have no error
    let last_error = processed.last().unwrap();
    assert!(
        last_error.is_none(),
        "StrandProcessed should have no error on success, got: {:?}",
        last_error
    );
}
