//! Application-level tests for agent timeout handling.
//!
//! Verifies that profile-level timeout configuration is respected
//! and timeout events are recorded. Uses mocked ports
//! (\`MockAgentRunner\` returning \`PortError::Timeout\`) — no
//! \`start_knot()\`, no \`TEST_MUTEX\`, no PATH manipulation.

mod helpers;

use std::path::PathBuf;
use std::sync::Arc;

use helpers::ProcessStrandBuilder;
use knot::application::ports::{AgentOutput, PortError};
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{KnotId, LoomId, StrandPath, TieOffStatus};
use knot::domain::events::{LoomEvent, RigLogEvent};
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

/// Build a profile with a custom timeout (in seconds).
fn build_profile_with_timeout(timeout_secs: u64) -> AgentProfile {
    default_profile().with_timeout(Some(timeout_secs))
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Profile timeout is passed through to the agent runner context.
///
/// Verifies that the profile's `timeout` field resolves to
/// `session_timeout()` and is passed as `ctx.timeout` when the
/// agent is invoked.
#[test]
fn profile_timeout_is_passed_to_runner() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    // Profile with 60s timeout
    let profile = build_profile_with_timeout(60);

    let loom = build_loom("review-loom", vec![build_knot("review")]);

    let runner = MockAgentRunner::new(Ok(AgentOutput {
        stdout: "mock output".to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    }));

    let helpers::ProcessStrandResult {
        strand: use_case,
        agent_runner: runner_arc,
        ..
    } = ProcessStrandBuilder::new(loom, Arc::new(runner))
        .with_profile(profile)
        .build();

    use_case
        .execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Verify the captured context has the correct timeout
    let contexts = runner_arc.get_captured_contexts();
    assert!(!contexts.is_empty(), "should have at least 1 context");
    let ctx = &contexts[0];

    // Profile timeout of 60s should be passed to the runner
    assert!(
        ctx.timeout.is_some(),
        "profile timeout should be passed to runner context"
    );
    assert_eq!(
        ctx.timeout.unwrap().as_secs(),
        60,
        "timeout should match profile's 60s timeout"
    );
}

/// On timeout error (\`PortError::Timeout\`):
/// - loom-log receives KnotProcessing → KnotFailed → StrandProcessed
/// - rig-log receives TimeoutExceeded
/// - tie-off is NOT appended (preserved unchanged)
#[test]
fn profile_timeout_results_in_timeout_events() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let profile = build_profile_with_timeout(30);

    let loom = build_loom("review-loom", vec![build_knot("review")]);

    // Agent runner that returns a timeout error
    let timeout_err = PortError::Timeout {
        message: "session exceeded timeout".to_string(),
        session_id: None,
    };
    let runner = Arc::new(MockAgentRunner::new(Err(timeout_err)));

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        rig_events,
        agent_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(profile)
        .build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // execute() always returns Ok (errors are logged, not propagated)

    // Loom-log: KnotProcessing → KnotFailed → StrandProcessed
    let events = log_events.lock().unwrap();
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotProcessing { .. })),
        "should have KnotProcessing"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. })),
        "should have KnotFailed"
    );
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::StrandProcessed { .. })),
        "should have StrandProcessed"
    );

    // KnotFailed error should mention timeout
    let failed = events.iter()
        .find_map(|e| {
            if let LoomEvent::KnotFailed { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        });
    assert!(
        failed.as_ref().map(|e| e.contains("timeout")).unwrap_or(false),
        "KnotFailed should mention timeout"
    );

    // StrandProcessed should have an error
    let processed_error = events.iter()
        .find_map(|e| {
            if let LoomEvent::StrandProcessed { error, .. } = e {
                error.clone()
            } else {
                None
            }
        });
    assert!(
        processed_error.as_ref().map(|e| !e.is_empty()).unwrap_or(false),
        "StrandProcessed should have an error"
    );

    // Rig-log: TimeoutExceeded
    let rig = rig_events.lock().unwrap();
    assert!(
        rig.iter().any(|e| matches!(e, RigLogEvent::TimeoutExceeded { .. })),
        "should have TimeoutExceeded in rig-log"
    );

    // Tie-off: NOT appended (preserved unchanged)
    let appends = tie_off_appends.lock().unwrap();
    assert!(
        appends.is_empty(),
        "tie-off should NOT be appended on timeout"
    );
}

/// On non-timeout error (e.g., AgentExecutionFailed):
/// - loom-log receives KnotProcessing → KnotFailed → StrandProcessed
/// - rig-log does NOT receive TimeoutExceeded
/// - tie-off IS appended with error content
#[test]
fn profile_non_timeout_error_writes_tieoff() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let profile = build_profile_with_timeout(30);

    let loom = build_loom("review-loom", vec![build_knot("review")]);

    let agent_err = PortError::AgentExecutionFailed {
        message: "agent crash".to_string(),
        session_id: None,
    };
    let runner = Arc::new(MockAgentRunner::new(Err(agent_err)));

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        rig_events,
        agent_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(profile)
        .build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Loom-log: KnotFailed
    let events = log_events.lock().unwrap();
    let failed = events.iter()
        .find_map(|e| {
            if let LoomEvent::KnotFailed { error, .. } = e {
                Some(error.clone())
            } else {
                None
            }
        });
    assert!(
        failed.as_ref().map(|e| e.contains("crash")).unwrap_or(false),
        "KnotFailed should mention crash"
    );

    // Rig-log: NO TimeoutExceeded (only timeout writes to rig-log)
    let rig = rig_events.lock().unwrap();
    assert!(
        rig.is_empty(),
        "rig-log should NOT receive event for non-timeout errors"
    );

    // Tie-off: IS appended with error content
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "tie-off should be appended");
    assert_eq!(appends[0].status, TieOffStatus::Failed);
    assert!(
        appends[0].content.contains("crash"),
        "tie-off content should contain error detail"
    );
}

/// On successful execution (within timeout):
/// - loom-log receives KnotProcessing → KnotCompleted → StrandProcessed
/// - rig-log receives NO events
/// - tie-off IS appended with agent output
#[test]
fn profile_timeout_success_no_timeout_events() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let profile = build_profile_with_timeout(120);

    let loom = build_loom("review-loom", vec![build_knot("review")]);

    let output = Ok(AgentOutput {
        stdout: "agent output".to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    });
    let runner = Arc::new(MockAgentRunner::new(output));

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        rig_events,
        agent_runner,
        ..
    } = ProcessStrandBuilder::new(loom, runner)
        .with_profile(profile)
        .build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Loom-log: KnotCompleted, no KnotFailed
    let events = log_events.lock().unwrap();
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotCompleted { .. })),
        "should have KnotCompleted"
    );
    assert!(
        !events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. })),
        "should NOT have KnotFailed"
    );

    // Rig-log: empty
    let rig = rig_events.lock().unwrap();
    assert!(rig.is_empty(), "rig-log should be empty on success");

    // Tie-off: appended with success content
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1);
    assert_eq!(appends[0].status, TieOffStatus::Produced);
    assert_eq!(appends[0].content, "agent output");
}
