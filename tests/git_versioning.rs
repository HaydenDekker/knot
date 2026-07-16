//! Application-level integration tests for git versioning.
//!
//! Verifies that git commits are created after successful strand
//! processing when the knot has `git-versioned: true`, by constructing
//! `ProcessStrand` with `MockGitVersioningPort`.
//!
//! No `start_knot()` calls, no `TEST_MUTEX`, no PATH manipulation —
//! all ports are mocked, tests run fully parallel, and complete in
//! sub-millisecond time.

mod helpers;

use std::path::PathBuf;
use std::sync::Arc;

use helpers::ProcessStrandBuilder;
use knot::application::ports::{AgentOutput, PortError};
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{Knot, KnotId, Loom, LoomId, StrandPath};
use knot::domain::events::{LoomEvent, StrandEvent};

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

    let result = ProcessStrandBuilder::new(loom, runner).with_tracking_git().build();
    let git_commits = result.git_commits.as_ref().expect("git_commits should be Some");
    let helpers::ProcessStrandResult {
        strand: use_case,
        ..
    } = result;

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

    let result = ProcessStrandBuilder::new(loom, runner).with_tracking_git().build();
    let git_commits = result.git_commits.as_ref().expect("git_commits should be Some");
    let helpers::ProcessStrandResult {
        strand: use_case,
        ..
    } = result;

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

    let result = ProcessStrandBuilder::new(loom, runner).with_tracking_git().build();
    let git_commits = result.git_commits.as_ref().expect("git_commits should be Some");
    let helpers::ProcessStrandResult {
        strand: use_case,
        ..
    } = result;

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

    let result = ProcessStrandBuilder::new(loom, runner).with_tracking_git().build();
    let git_commits = result.git_commits.as_ref().expect("git_commits should be Some");
    let helpers::ProcessStrandResult {
        strand: use_case,
        ..
    } = result;

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

    let result = ProcessStrandBuilder::new(loom, runner).with_tracking_git().build();
    let git_port = result.git_port.as_ref().expect("git_port should be Some");
    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        tie_off_appends,
        ..
    } = result;

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
