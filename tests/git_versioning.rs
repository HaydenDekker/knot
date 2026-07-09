//! Application-level integration tests for git versioning.
//!
//! Verifies that git commits are created after successful strand
//! processing when the knot has `git-versioned: true`, by constructing
//! `ProcessStrand` with `MockGitVersioningPort`.
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
    Knot, KnotId, Loom, LoomId, StrandPath,
};
use knot::domain::events::{LoomEvent, StrandEvent};
use knot::RigAgentConfig;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a knot with git-versioned: true.
fn build_knot_git_on(id: &str) -> Knot {
    let mut knot = build_knot_with_profile(id, "fast");
    knot.git_versioned = true;
    knot
}

/// Build a knot with git-versioned: false.
fn build_knot_git_off(id: &str) -> Knot {
    let mut knot = build_knot_with_profile(id, "fast");
    knot.git_versioned = false;
    knot
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
/// tie_off_content, agent_runner, git_port, git_commits).
#[allow(clippy::type_complexity)]
fn build_process_strand(
    loom: Loom,
    agent_runner: Arc<MockAgentRunner>,
) -> (
    ProcessStrand,
    Arc<Mutex<Vec<LoomEvent>>>,
    Arc<Mutex<Vec<knot::domain::entities::TieOff>>>,
    Arc<Mutex<Vec<knot::domain::events::RigLogEvent>>>,
    Arc<Mutex<HashMap<String, String>>>,
    Arc<MockAgentRunner>,
    Arc<MockGitVersioningPort>,
    Arc<Mutex<Vec<(knot::domain::entities::LoomId, knot::domain::entities::KnotId, String, String, String)>>>,
) {
    let store = LoomStore::new();
    store.register(loom);

    let (log_port, log_events) = MockLoomLogPort::new();
    let (tie_off_sink, tie_off_appends, tie_off_content) =
        TrackingTieOffSink::new();
    let (rig_log, rig_events) = MockRigLogPort::new();
    let (git_port, git_commits) = MockGitVersioningPort::new();
    let git_port = Arc::new(git_port);

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
        Arc::new(MockStrandFileChecker::new()),
        Arc::new(MockEventDispatcher::default()),
    );

    (
        use_case,
        log_events,
        tie_off_appends,
        rig_events,
        tie_off_content,
        agent_runner,
        git_port,
        git_commits,
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

/// Create a real strand file on disk.
fn create_strand_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ── Git versioning: commit on success ────────────────────────────────────

/// Git commit is created after successful processing when git-versioned is true.
///
/// Verifies: loom_id, knot_id, strand_path, event_type, and tie-off content
/// are all passed correctly to the git port.
#[test]
fn git_commit_created_after_processing() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot_git_on("review")]);
    let runner = success_runner("review output");

    let (use_case, _log_events, _tie_off_appends, _rig_events, _content,
        _captured, _git_port, git_commits) = build_process_strand(loom, runner);

    use_case.execute(created_event("review-loom", "review", strand_path.clone()))
        .unwrap();

    // Verify git commit was called
    let commits = git_commits.lock().unwrap();
    assert_eq!(commits.len(), 1, "should have exactly 1 git commit");

    let (loom_id, knot_id, sp, event_type, tie_off_content) = &commits[0];
    assert_eq!(loom_id.0, "review-loom");
    assert_eq!(knot_id.0, "review");
    assert!(
        sp.contains("feature.md"),
        "strand_path should reference feature.md, got: {}",
        sp
    );
    assert_eq!(*event_type, "Created");
    assert!(
        tie_off_content.contains("review output"),
        "tie-off content should contain agent output"
    );
}

/// Git commit is NOT created when git-versioned is false.
#[test]
fn no_git_commit_when_not_versioned() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot_git_off("review")]);
    let runner = success_runner("review output");

    let (use_case, _log_events, _tie_off_appends, _rig_events, _content,
        _captured, _git_port, git_commits) = build_process_strand(loom, runner);

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Verify git commit was NOT called
    let commits = git_commits.lock().unwrap();
    assert!(
        commits.is_empty(),
        "should have no git commits when git-versioned is false"
    );
}

/// Git commit is NOT created when processing fails (agent error).
#[test]
fn no_git_commit_on_processing_failure() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot_git_on("review")]);
    let runner = failure_runner("crash");

    let (use_case, _log_events, _tie_off_appends, _rig_events, _content,
        _captured, _git_port, git_commits) = build_process_strand(loom, runner);

    use_case.execute(created_event("review-loom", "review", strand_path))
        .unwrap();

    // Verify git commit was NOT called (only on success)
    let commits = git_commits.lock().unwrap();
    assert!(
        commits.is_empty(),
        "should have no git commits on processing failure"
    );
}

/// Multiple strand processings create multiple git commits.
#[test]
fn git_multiple_commits_for_multiple_strands() {
    let dir = tempfile::tempdir().unwrap();
    let strand1 = create_strand_file(&dir, "feature1.md", "feature 1");
    let strand2 = create_strand_file(&dir, "feature2.md", "feature 2");

    let loom = build_loom("review-loom", vec![build_knot_git_on("review")]);
    let runner = success_runner("review output");

    let (use_case, _log_events, _tie_off_appends, _rig_events, _content,
        _captured, _git_port, git_commits) = build_process_strand(loom, runner);

    use_case.execute(created_event("review-loom", "review", strand1))
        .unwrap();
    use_case.execute(created_event("review-loom", "review", strand2))
        .unwrap();

    let commits = git_commits.lock().unwrap();
    assert_eq!(commits.len(), 2, "should have 2 git commits");
    assert!(
        commits[0].2.contains("feature1.md"),
        "first commit should reference feature1.md"
    );
    assert!(
        commits[1].2.contains("feature2.md"),
        "second commit should reference feature2.md"
    );
}

// ── Git versioning: graceful error handling ──────────────────────────────

/// Git commit errors are handled gracefully: processing completes normally,
/// error is logged as a warning (not propagated to the caller).
#[test]
fn git_commit_error_is_handled_gracefully() {
    let dir = tempfile::tempdir().unwrap();
    let strand_path = create_strand_file(&dir, "feature.md", "content");

    let loom = build_loom("review-loom", vec![build_knot_git_on("review")]);
    let runner = success_runner("review output");

    let (use_case, log_events, tie_off_appends, _rig_events, _content,
        _captured, git_port, _git_commits) = build_process_strand(loom, runner);

    // Force git port to return an error
    git_port.set_error(PortError::GitCommitFailed(
        "not a git repository".to_string(),
    ));

    // Processing should still succeed
    let result = use_case.execute(created_event("review-loom", "review", strand_path));
    assert!(result.is_ok(), "processing should succeed despite git error");

    // Verify processing completed normally
    let events = log_events.lock().unwrap();
    let has_completed = events.iter()
        .any(|e| matches!(e, LoomEvent::KnotCompleted { .. }));
    assert!(
        has_completed,
        "should have KnotCompleted despite git error"
    );

    // Verify tie-off was written
    let appends = tie_off_appends.lock().unwrap();
    assert_eq!(appends.len(), 1, "tie-off should be written despite git error");
}
