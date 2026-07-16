//! Application-level integration tests for rig-log event recording.
//!
//! Verifies that operational events (timeouts) are written to the rig-log
//! by constructing `ProcessStrand` with `MockRigLogPort`.
//!
//! No `start_knot()` calls, no `TEST_MUTEX`, no PATH manipulation —
//! all ports are mocked, tests run fully parallel, and complete in
//! sub-millisecond time.
//!
//! Queue idle detection is an infrastructure concern handled by the
//! debounce engine and verified by composition (smoke) tests.

mod helpers;

use std::path::PathBuf;
use std::sync::Arc;

use helpers::ProcessStrandBuilder;
use knot::application::ports::{AgentOutput, PortError};
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{Knot, KnotId, Loom, LoomId, StrandPath};
use knot::domain::events::{LoomEvent, RigLogEvent, StrandEvent};

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

/// Build a successful agent output mock runner.
fn success_runner(output: &str) -> Arc<MockAgentRunner> {
    Arc::new(MockAgentRunner::new(Ok(AgentOutput {
        stdout: output.to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    })))
}

/// Build a timeout error mock runner.
fn timeout_runner(message: &str) -> Arc<MockAgentRunner> {
    Arc::new(MockAgentRunner::new(Err(
        PortError::Timeout {
            message: message.to_string(),
            session_id: None,
        },
    )))
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

/// Create a real strand file on disk.
fn create_strand_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ── Rig-log: timeout event ──────────────────────────────────────────────

/// On agent timeout, `TimeoutExceeded` is appended to the rig-log with
/// correct loom_id, knot_id, strand_path, and error message.
#[test]
fn rig_log_timeout_exceeded_on_agent_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = timeout_runner("session exceeded 60s");

    let helpers::ProcessStrandResult {
        strand: use_case,
        rig_events,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    let rig = rig_events.lock().unwrap();
    assert_eq!(rig.len(), 1, "should have exactly 1 rig-log event on timeout");

    match &rig[0] {
        RigLogEvent::TimeoutExceeded {
            loom_id,
            knot_id,
            strand_path,
            error,
            ..
        } => {
            assert_eq!(loom_id.0, "review-loom");
            assert_eq!(knot_id.0, "review");
            assert!(
                strand_path.0.to_string_lossy().contains("feature.md"),
                "strand_path should reference feature.md"
            );
            assert!(
                error.contains("timeout") || error.contains("60s"),
                "error should contain timeout detail, got: {}",
                error
            );
        }
        other => panic!("expected TimeoutExceeded, got {:?}", other),
    }
}

/// On non-timeout agent failure (e.g., AgentExecutionFailed), NO rig-log
/// event is written. Only timeouts write to the rig-log.
#[test]
fn rig_log_no_event_on_non_timeout_error() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = failure_runner("agent crash");

    let helpers::ProcessStrandResult {
        strand: use_case,
        rig_events,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    let rig = rig_events.lock().unwrap();
    assert!(
        rig.is_empty(),
        "rig-log should NOT receive event for non-timeout errors"
    );
}

/// On successful processing, NO rig-log event is written.
#[test]
fn rig_log_no_event_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let runner = success_runner("ok output");

    let helpers::ProcessStrandResult {
        strand: use_case,
        rig_events,
        ..
    } = ProcessStrandBuilder::new(loom, runner).build();

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    let rig = rig_events.lock().unwrap();
    assert!(
        rig.is_empty(),
        "rig-log should be empty on successful processing"
    );
}
