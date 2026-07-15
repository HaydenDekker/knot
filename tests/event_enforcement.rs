//! Integration tests for event enforcement (Phase 4).
//!
//! Verifies the full strand-processing pipeline when agent events are
//! expected but not emitted. Uses mocked ports so tests run fully
//! parallel and in sub-millisecond time.
//!
//! The enforcement flow:
//! 1. Knot completes successfully with no event blocks in tie-off
//! 2. `KnotEventsMissing` logged to loom-log
//! 3. Follow-up re-entry attempted (if session ID available)
//! 4. Follow-up response parsed for events → dispatched if found
//! 5. If still no events, second `KnotEventsMissing` logged

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use knot::application::ports::{
    AgentOutput, AgentRunner,
};
use knot::application::ports::AgentInvocationMetadata;
use knot::application::store::LoomStore;
use knot::application::usecases::ProcessStrand;
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{
    Knot, KnotId, Loom, LoomId, StrandPath, TieOff, TieOffStatus,
};
use knot::domain::events::{LoomEvent, StrandEvent};
use knot::domain::value_objects::StrandSource;
use knot::RigAgentConfig;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a producer knot with the given ID and "fast" profile.
fn build_producer_knot(id: &str) -> Knot {
    build_knot_with_profile(id, "fast")
}

/// Build a consumer knot that listens for events from a producer.
fn build_consumer_knot(
    id: &str,
    producer_knot: &str,
    event_id: &str,
) -> Knot {
    KnotBuilder::new(id)
        .with_instructions("consume events")
        .with_strand_source(StrandSource::EventUri {
            producer_knot: producer_knot.to_string(),
            event_id: event_id.to_string(),
        })
        .build()
}

/// Build a loom with the given ID and knots.
fn build_loom(id: &str, knots: Vec<Knot>) -> Loom {
    Loom {
        id: LoomId(id.to_string()),
        knots,
    }
}

/// Build a successful agent output with session ID and given stdout.
fn success_output_with_session(stdout: &str, session_id: &str) -> AgentOutput {
    AgentOutput {
        stdout: stdout.to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: Some(AgentInvocationMetadata {
            session_id: Some(session_id.to_string()),
            token_usage: None,
        }),
    }
}

/// Build a successful agent output without session ID (stdio-style).
fn success_output_no_session(stdout: &str) -> AgentOutput {
    AgentOutput {
        stdout: stdout.to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    }
}

/// Build the ProcessStrand use case with all mocks wired up, returning
/// handles for inspection.
///
/// Supports multiple looms (for event consumers in a different loom).
#[allow(clippy::type_complexity)]
fn build_process_strand(
    looms: Vec<Loom>,
    agent_runner: Arc<MockAgentRunner>,
) -> (
    ProcessStrand,
    Arc<Mutex<Vec<LoomEvent>>>,
    Arc<Mutex<Vec<TieOff>>>,
    Arc<Mutex<Vec<knot::domain::events::RigLogEvent>>>,
    Arc<Mutex<HashMap<String, String>>>,
    Arc<MockAgentRunner>,
    Arc<MockEventDispatcher>,
) {
    let store = LoomStore::new();
    for loom in &looms {
        store.register(loom.clone());
    }

    let (log_port, log_events) = MockLoomLogPort::new();
    let (tie_off_sink, tie_off_appends, tie_off_content) =
        TrackingTieOffSink::new();
    let (rig_log, rig_events) = MockRigLogPort::new();
    let (git_port, _git_commits) = MockGitVersioningPort::new();
    let git_port = Arc::new(git_port);
    let file_checker = Arc::new(MockStrandFileChecker::new());
    let event_dispatcher = Arc::new(MockEventDispatcher::default());

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
        git_port.clone(),
        file_checker.clone(),
        event_dispatcher.clone(),
        None,
    );

    (
        use_case,
        log_events,
        tie_off_appends,
        rig_events,
        tie_off_content,
        agent_runner,
        event_dispatcher,
    )
}

/// Build a `StrandEvent::Created` for the given loom/knot/strand.
fn created_event(
    loom_id: &str,
    knot_id: &str,
    strand_path: PathBuf,
) -> StrandEvent {
    StrandEvent::Created {
        loom_id: LoomId(loom_id.to_string()),
        knot_id: KnotId(knot_id.to_string()),
        strand_path: StrandPath(strand_path),
    }
}

/// Create a real strand file on disk.
fn create_strand_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// Count loom-log events of a specific variant.
fn count_event_type(events: &[LoomEvent], variant: &str) -> usize {
    events.iter()
        .filter(|e| match_event_type(e, variant))
        .count()
}

/// Check if a loom event matches a variant name.
fn match_event_type(event: &LoomEvent, variant: &str) -> bool {

    let json = serde_json::to_value(event).unwrap_or_default();
    json.as_object().map(|obj| obj.contains_key(variant)).unwrap_or(false)
}

// ── Integration Tests ────────────────────────────────────────────────────

/// Full enforcement flow: knot with event consumers, agent produces no
/// events → KnotEventsMissing logged → follow-up re-enters session with
/// `--session-id` → follow-up response contains events → events dispatched
/// to consumer knots.
///
/// This verifies the complete enforcement pipeline end-to-end:
/// detection → logging → follow-up → parsing → dispatch.
#[test]
fn test_event_enforcement_with_real_pi() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path =
        create_strand_file(&dir, "feature.md", "new feature request");

    // Producer loom with a knot that produces events
    let producer_knot = build_producer_knot("plan-creator");
    // Consumer loom with a knot listening for PlanCreated events
    let consumer_knot = build_consumer_knot("plan-validator", "plan-creator", "PlanCreated");

    let producer_loom = build_loom("planning-loom", vec![producer_knot]);
    let consumer_loom = build_loom("validation-loom", vec![consumer_knot]);

    // Runner: first call returns body with no events, second call
    // (follow-up) returns event blocks.
    let initial_output = success_output_with_session(
        "Plan created and reviewed. No issues found.",
        "sess-abc123",
    );
    let followup_output = success_output_with_session(
        concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "description: Feature plan created\n",
            "---\n\n",
            "Plan created for feature.\n",
            "```",
        ),
        "sess-abc123",
    );
    let runner = Arc::new(MockAgentRunner::new_sequence(vec![
        Ok(initial_output),
        Ok(followup_output),
    ]));

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured, event_dispatcher) =
        build_process_strand(vec![producer_loom, consumer_loom], runner);

    let event = created_event(
        "planning-loom",
        "plan-creator",
        strand_path,
    );

    let result = use_case.execute(event);
    assert!(result.is_ok());

    // Verify loom-log has the full sequence:
    // KnotProcessing → KnotEventsMissing → KnotCompleted → StrandProcessed
    let events = log_events.lock().unwrap();

    // KnotProcessing
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotProcessing { .. })),
        "should have KnotProcessing"
    );

    // KnotEventsMissing (first) — enforcement detected missing events
    let missing_count = count_event_type(&events, "KnotEventsMissing");
    assert_eq!(
        missing_count, 1,
        "should have exactly 1 KnotEventsMissing (follow-up produced events)"
    );

    // Verify KnotEventsMissing has correct fields
    let missing_event = events.iter().find(|e| {
        matches!(e, LoomEvent::KnotEventsMissing { .. })
    });
    if let Some(LoomEvent::KnotEventsMissing {
        loom_id,
        knot_id,
        expected_events,
        ..
    }) = missing_event
    {
        assert_eq!(loom_id.0, "planning-loom");
        assert_eq!(knot_id.0, "plan-creator");
        assert!(
            expected_events.iter().any(|e| e == "PlanCreated"),
            "expected_events should contain PlanCreated: {:?}",
            expected_events
        );
    } else {
        panic!("Expected KnotEventsMissing event");
    }

    // KnotCompleted (strand still completes successfully)
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted"
    );

    // StrandProcessed with no error
    let processed = events.iter().find(|e| {
        matches!(e, LoomEvent::StrandProcessed { .. })
    });
    if let Some(LoomEvent::StrandProcessed { error, .. }) = processed {
        assert!(
            error.is_none(),
            "StrandProcessed should have no error: {:?}",
            error
        );
    }

    // Follow-up dispatch doesn't log EventsDispatched to loom-log
    // (only the initial dispatch path does). Verify through dispatcher.

    // Verify the event dispatcher was called with the correct event
    let dispatches = event_dispatcher.get_dispatches();
    assert!(
        !dispatches.is_empty(),
        "event dispatcher should have been called"
    );
    let (event, consumer_knot_id, consumer_loom_id, _rig_dir) = &dispatches[0];
    assert_eq!(event.event_id, "PlanCreated");
    assert_eq!(*consumer_knot_id, "plan-validator");
    assert_eq!(*consumer_loom_id, "validation-loom");
}

/// `pi-stdio` adapter simulation: no session ID captured, so enforcement
/// can only log the failure — no follow-up re-entry is attempted.
///
/// Verifies graceful degradation: KnotEventsMissing is logged, strand
/// completes successfully, no dispatch occurs.
#[test]
fn test_event_enforcement_stdio_no_reentry() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path =
        create_strand_file(&dir, "feature.md", "new feature request");

    let producer_knot = build_producer_knot("plan-creator");
    let consumer_knot = build_consumer_knot("plan-validator", "plan-creator", "PlanCreated");

    let producer_loom = build_loom("planning-loom", vec![producer_knot]);
    let consumer_loom = build_loom("validation-loom", vec![consumer_knot]);

    // Runner with no session ID (stdio-style) — no follow-up possible
    let output = success_output_no_session(
        "Plan created and reviewed. No issues found.",
    );
    let runner = Arc::new(MockAgentRunner::new(Ok(output)));

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        captured, event_dispatcher) =
        build_process_strand(vec![producer_loom, consumer_loom], runner);

    let event = created_event(
        "planning-loom",
        "plan-creator",
        strand_path,
    );

    let result = use_case.execute(event);
    assert!(result.is_ok());

    let events = log_events.lock().unwrap();

    // KnotEventsMissing logged
    let missing_count = count_event_type(&events, "KnotEventsMissing");
    assert_eq!(
        missing_count, 1,
        "should have exactly 1 KnotEventsMissing (no follow-up possible)"
    );

    // Strand still completes successfully
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted"
    );

    // No EventsDispatched (no follow-up was possible)
    assert!(
        !events.iter().any(|e| matches!(e, LoomEvent::EventsDispatched { .. })),
        "should NOT have EventsDispatched (no follow-up re-entry)"
    );

    // Event dispatcher was NOT called
    let dispatches = event_dispatcher.get_dispatches();
    assert!(
        dispatches.is_empty(),
        "event dispatcher should NOT have been called"
    );

    // Verify the runner was called only once (no follow-up re-entry)
    let contexts = captured.get_captured_contexts();
    assert_eq!(
        contexts.len(), 1,
        "agent should have been called only once (no follow-up)"
    );
}

/// Agent emits `event: None` — enforcement is NOT triggered.
///
/// `event: None` is a valid outcome meaning "no events occurred".
/// The enforcement check passes and no follow-up is attempted.
#[test]
fn test_event_enforcement_event_none_passes() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path =
        create_strand_file(&dir, "feature.md", "new feature request");

    let producer_knot = build_producer_knot("plan-creator");
    let consumer_knot = build_consumer_knot("plan-validator", "plan-creator", "PlanCreated");

    let producer_loom = build_loom("planning-loom", vec![producer_knot]);
    let consumer_loom = build_loom("validation-loom", vec![consumer_knot]);

    // Runner returns event: None (valid "no events" signal)
    let output = success_output_with_session(
        concat!(
            "Plan reviewed. No events to report.\n\n",
            "```markdown\n",
            "---\n",
            "event: None\n",
            "---\n",
            "```",
        ),
        "sess-abc123",
    );
    let runner = Arc::new(MockAgentRunner::new(Ok(output)));

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        captured, event_dispatcher) =
        build_process_strand(vec![producer_loom, consumer_loom], runner);

    let event = created_event(
        "planning-loom",
        "plan-creator",
        strand_path,
    );

    let result = use_case.execute(event);
    assert!(result.is_ok());

    let events = log_events.lock().unwrap();

    // No KnotEventsMissing (event: None passes enforcement)
    let missing_count = count_event_type(&events, "KnotEventsMissing");
    assert_eq!(
        missing_count, 0,
        "should have NO KnotEventsMissing when event: None is emitted"
    );

    // Strand completes normally
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted"
    );

    // No dispatch (event: None produces no events)
    assert!(
        !events.iter().any(|e| matches!(e, LoomEvent::EventsDispatched { .. })),
        "should NOT have EventsDispatched (event: None produces no events)"
    );

    // Runner called only once (no follow-up)
    let contexts = captured.get_captured_contexts();
    assert_eq!(
        contexts.len(), 1,
        "agent should have been called only once"
    );

    // Dispatcher NOT called
    let dispatches = event_dispatcher.get_dispatches();
    assert!(
        dispatches.is_empty(),
        "event dispatcher should NOT have been called"
    );
}

/// Knot with no consumer knots listening for events — enforcement
/// logic does not run at all.
///
/// When `listener_context` is empty (no consumers), the enforcement
/// check is skipped entirely.
#[test]
fn test_event_enforcement_no_consumers() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path =
        create_strand_file(&dir, "feature.md", "new feature request");

    // Producer knot with NO consumers in the store
    let producer_knot = build_producer_knot("plan-creator");
    let loom = build_loom("planning-loom", vec![producer_knot]);

    // Runner returns body with no events — but no consumers, so no enforcement
    let output = success_output_with_session(
        "Plan created. No issues.",
        "sess-abc123",
    );
    let runner = Arc::new(MockAgentRunner::new(Ok(output)));

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        captured, event_dispatcher) =
        build_process_strand(vec![loom], runner);

    let event = created_event(
        "planning-loom",
        "plan-creator",
        strand_path,
    );

    let result = use_case.execute(event);
    assert!(result.is_ok());

    let events = log_events.lock().unwrap();

    // No KnotEventsMissing (no consumers, enforcement skipped)
    let missing_count = count_event_type(&events, "KnotEventsMissing");
    assert_eq!(
        missing_count, 0,
        "should have NO KnotEventsMissing when there are no consumers"
    );

    // Strand completes normally
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted"
    );

    // No dispatch
    assert!(
        !events.iter().any(|e| matches!(e, LoomEvent::EventsDispatched { .. })),
        "should NOT have EventsDispatched"
    );

    // Runner called only once (no follow-up)
    let contexts = captured.get_captured_contexts();
    assert_eq!(
        contexts.len(), 1,
        "agent should have been called only once"
    );

    // Dispatcher NOT called
    let dispatches = event_dispatcher.get_dispatches();
    assert!(
        dispatches.is_empty(),
        "event dispatcher should NOT have been called"
    );
}

/// Full enforcement flow where follow-up also produces no events.
///
/// Verifies: two KnotEventsMissing logged (original + follow-up failure),
/// strand still completes, no dispatch.
#[test]
fn test_event_enforcement_followup_also_fails() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path =
        create_strand_file(&dir, "feature.md", "new feature request");

    let producer_knot = build_producer_knot("plan-creator");
    let consumer_knot = build_consumer_knot("plan-validator", "plan-creator", "PlanCreated");

    let producer_loom = build_loom("planning-loom", vec![producer_knot]);
    let consumer_loom = build_loom("validation-loom", vec![consumer_knot]);

    // Runner: both initial and follow-up produce no events
    let initial_output = success_output_with_session(
        "Plan created. No events emitted.",
        "sess-abc123",
    );
    let followup_output = success_output_with_session(
        "Sorry, I still don't have events to report.",
        "sess-abc123",
    );
    let runner = Arc::new(MockAgentRunner::new_sequence(vec![
        Ok(initial_output),
        Ok(followup_output),
    ]));

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured, event_dispatcher) =
        build_process_strand(vec![producer_loom, consumer_loom], runner);

    let event = created_event(
        "planning-loom",
        "plan-creator",
        strand_path,
    );

    let result = use_case.execute(event);
    assert!(result.is_ok());

    let events = log_events.lock().unwrap();

    // Two KnotEventsMissing: first for original missing events,
    // second for follow-up also missing events.
    let missing_count = count_event_type(&events, "KnotEventsMissing");
    assert_eq!(
        missing_count, 2,
        "should have 2 KnotEventsMissing (original + follow-up failure)"
    );

    // Strand still completes
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted"
    );

    // No dispatch
    assert!(
        !events.iter().any(|e| matches!(e, LoomEvent::EventsDispatched { .. })),
        "should NOT have EventsDispatched"
    );

    // Dispatcher NOT called
    let dispatches = event_dispatcher.get_dispatches();
    assert!(
        dispatches.is_empty(),
        "event dispatcher should NOT have been called"
    );
}

/// Multiple consumers listening for the same event — follow-up events
/// are dispatched to ALL matching consumers.
#[test]
fn test_event_enforcement_multiple_consumers() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path =
        create_strand_file(&dir, "feature.md", "new feature request");

    let producer_knot = build_producer_knot("plan-creator");
    // Two consumers in different looms, both listening for PlanCreated
    let consumer1 = build_consumer_knot("plan-validator", "plan-creator", "PlanCreated");
    let consumer2 = build_consumer_knot("plan-auditor", "plan-creator", "PlanCreated");

    let producer_loom = build_loom("planning-loom", vec![producer_knot]);
    let validation_loom = build_loom("validation-loom", vec![consumer1]);
    let audit_loom = build_loom("audit-loom", vec![consumer2]);

    let initial_output = success_output_with_session(
        "Plan created.",
        "sess-abc123",
    );
    let followup_output = success_output_with_session(
        concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "description: Feature plan created\n",
            "---\n\n",
            "Plan created.\n",
            "```",
        ),
        "sess-abc123",
    );
    let runner = Arc::new(MockAgentRunner::new_sequence(vec![
        Ok(initial_output),
        Ok(followup_output),
    ]));

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        _captured, event_dispatcher) =
        build_process_strand(
            vec![producer_loom, validation_loom, audit_loom],
            runner,
        );

    let event = created_event(
        "planning-loom",
        "plan-creator",
        strand_path,
    );

    let result = use_case.execute(event);
    assert!(result.is_ok());

    let events = log_events.lock().unwrap();

    // KnotEventsMissing logged (initial missing)
    let missing_count = count_event_type(&events, "KnotEventsMissing");
    assert_eq!(
        missing_count, 1,
        "should have 1 KnotEventsMissing (follow-up produced events)"
    );

    // Follow-up dispatch doesn't log EventsDispatched to loom-log.
    // Verify through dispatcher.

    // Event dispatcher called twice (once per consumer loom)
    let dispatches = event_dispatcher.get_dispatches();
    assert_eq!(
        dispatches.len(), 2,
        "event dispatcher should have been called for both consumers"
    );

    // Verify both consumer looms received the dispatch
    let loom_ids: Vec<_> = dispatches.iter().map(|(_, _, loom, _)| loom.as_str()).collect();
    assert!(
        loom_ids.contains(&"validation-loom"),
        "should dispatch to validation-loom: {:?}",
        loom_ids
    );
    assert!(
        loom_ids.contains(&"audit-loom"),
        "should dispatch to audit-loom: {:?}",
        loom_ids
    );
}

// ── Regression: existing pipeline tests still pass ────────────────────────

/// Regression: basic pipeline processing still works without enforcement.
/// No consumers → no enforcement → normal completion.
#[test]
fn test_event_enforcement_regression_basic_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path =
        create_strand_file(&dir, "feature.md", "new feature request");

    let knot = build_producer_knot("review");
    let loom = build_loom("review-loom", vec![knot]);

    let output = success_output_no_session("review output");
    let runner = Arc::new(MockAgentRunner::new(Ok(output)));

    let (use_case, log_events, tie_off_appends, _rig_events, _content,
        _captured, _dispatcher) =
        build_process_strand(vec![loom], runner);

    let event = created_event("review-loom", "review", strand_path);

    let result = use_case.execute(event);
    assert!(result.is_ok());

    // Normal pipeline events, no enforcement
    let events = log_events.lock().unwrap();
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotProcessing { .. })),
        "should have KnotProcessing"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::StrandProcessed { .. })),
        "should have StrandProcessed"
    );

    // No enforcement events
    let missing_count = count_event_type(&events, "KnotEventsMissing");
    assert_eq!(missing_count, 0, "should have no enforcement events");

    // Tie-off written
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "should have 1 tie-off append");
    assert_eq!(appends[0].status, TieOffStatus::Produced);
    assert!(
        appends[0].content.contains("review output"),
        "tie-off should contain agent output"
    );
}

/// Regression: event dispatch (non-enforcement) still works normally.
/// Agent produces events in the initial response → normal dispatch,
/// no enforcement triggered.
#[test]
fn test_event_enforcement_regression_normal_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path =
        create_strand_file(&dir, "feature.md", "new feature request");

    let producer_knot = build_producer_knot("plan-creator");
    let consumer_knot = build_consumer_knot("plan-validator", "plan-creator", "PlanCreated");

    let producer_loom = build_loom("planning-loom", vec![producer_knot]);
    let consumer_loom = build_loom("validation-loom", vec![consumer_knot]);

    // Agent produces events normally in the first response
    let output = success_output_with_session(
        concat!(
            "Plan created.\n\n",
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n\n",
            "Plan created.\n",
            "```",
        ),
        "sess-abc123",
    );
    let runner = Arc::new(MockAgentRunner::new(Ok(output)));

    let (use_case, log_events, _tie_off_appends, _rig_events, _content,
        captured, event_dispatcher) =
        build_process_strand(vec![producer_loom, consumer_loom], runner);

    let event = created_event(
        "planning-loom",
        "plan-creator",
        strand_path,
    );

    let result = use_case.execute(event);
    assert!(result.is_ok());

    let events = log_events.lock().unwrap();

    // Normal dispatch, no enforcement
    let missing_count = count_event_type(&events, "KnotEventsMissing");
    assert_eq!(
        missing_count, 0,
        "should have no KnotEventsMissing (events were present)"
    );

    // EventsDispatched from initial response
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::EventsDispatched { .. })),
        "should have EventsDispatched"
    );

    // Runner called only once (no follow-up)
    let contexts = captured.get_captured_contexts();
    assert_eq!(
        contexts.len(), 1,
        "agent should have been called only once"
    );

    // Dispatcher called
    let dispatches = event_dispatcher.get_dispatches();
    assert!(
        !dispatches.is_empty(),
        "event dispatcher should have been called"
    );
}
