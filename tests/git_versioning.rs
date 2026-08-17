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
use knot::application::ports::{AgentOutput, GitVersioningPort, PortError};
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

// ── Rig-aware versioner: real nested repos ────────────────────────────────
//
// Integration tests for the gitlink/stale-file guard: after `git add -A`,
// `commit()` runs `git reset -q -- <rig-dir>` so the rig (its own git
// repo, or tracked leftovers) can never enter a project commit.

use knot::adapters::outbound::FileSystemGitVersioner;

/// Helper: run a git command in `dir`, asserting success.
fn git(dir: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git should be available on test system");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Helper: run a git command in `dir`, returning stdout as a string.
fn git_stdout(dir: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git should be available on test system");
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Helper: create a git repo with a commit identity configured.
fn init_git_repo(dir: &std::path::Path) {
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.email", "test@test.com"]);
    git(dir, &["config", "user.name", "Test User"]);
}

/// Helper: names of all entries tracked at HEAD (includes gitlinks).
fn tracked_at_head(dir: &std::path::Path) -> Vec<String> {
    git_stdout(dir, &["ls-tree", "-r", "HEAD", "--name-only"])
        .lines()
        .map(|l| l.to_string())
        .collect()
}

/// Fresh rig (never tracked by the parent): after `commit()`, the rig is
/// not committed as a gitlink and no rig paths enter the project commit.
/// The rig's working tree is untouched — the reset only unstages.
#[test]
fn git_commit_excludes_fresh_rig_gitlink() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_git_repo(root);
    std::fs::write(root.join("README.md"), "project").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "initial"]);

    // Fresh rig: its own git repo with one commit — without a commit the
    // parent's `git add -A` would fail instead of staging a gitlink, so
    // the committed rig source is the realistic gitlink scenario.
    let rig = root.join("rig");
    let loom = rig.join("review-loom");
    std::fs::create_dir_all(&loom).unwrap();
    std::fs::write(loom.join("k.md"), "knot").unwrap();
    git(&rig, &["init", "-b", "main"]);
    git(&rig, &["config", "user.email", "rig@rig.com"]);
    git(&rig, &["config", "user.name", "Rig User"]);
    git(&rig, &["add", "-A"]);
    git(&rig, &["commit", "-m", "rig source"]);

    // One knot run: agent work in the project + rig source change.
    std::fs::write(root.join("README.md"), "project v2").unwrap();
    std::fs::write(loom.join("k.md"), "knot v2").unwrap();

    let versioner = FileSystemGitVersioner::new(root.to_path_buf(), rig.clone());
    let result = versioner.commit(
        &LoomId("review-loom".to_string()),
        &KnotId("k".to_string()),
        &StrandPath(PathBuf::from("input/s.md")),
        "Modified",
        "tie-off",
    );
    assert!(result.is_ok(), "commit should succeed: {:?}", result.err());

    // The rig must not be in the commit — neither as a gitlink nor files.
    let files = tracked_at_head(root);
    assert!(
        files.contains(&"README.md".to_string()),
        "project file committed: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f == "rig" || f.starts_with("rig/")),
        "rig must not enter the project commit: {files:?}"
    );

    // Nothing left staged (the gitlink was unstaged before committing).
    let staged = git_stdout(root, &["diff", "--cached", "--name-only"]);
    assert!(staged.trim().is_empty(), "nothing left staged: {staged}");

    // The rig's working tree is untouched — the reset only unstages.
    assert_eq!(
        std::fs::read_to_string(loom.join("k.md")).unwrap(),
        "knot v2"
    );
}

/// Pre-tracked rig (rig files already in the parent's history): after
/// `commit()`, modified tracked rig files and new rig files are excluded
/// from the commit; the tracked rig file keeps its old committed content
/// (the reset unstages — it does not untrack).
#[test]
fn git_commit_excludes_pre_tracked_rig_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_git_repo(root);
    std::fs::write(root.join("README.md"), "project").unwrap();
    let loom = root.join("rig").join("review-loom");
    std::fs::create_dir_all(&loom).unwrap();
    std::fs::write(loom.join("k.md"), "knot").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "initial with rig tracked"]);

    // The rig later gets its own git repository (as ensure_rig_repo does).
    let rig = root.join("rig");
    git(&rig, &["init", "-b", "main"]);
    git(&rig, &["config", "user.email", "rig@rig.com"]);
    git(&rig, &["config", "user.name", "Rig User"]);
    git(&rig, &["add", "-A"]);
    git(&rig, &["commit", "-m", "rig source"]);

    // One knot run: project change + modified tracked rig file + new file.
    std::fs::write(root.join("README.md"), "project v2").unwrap();
    std::fs::write(loom.join("k.md"), "knot v2").unwrap();
    std::fs::write(loom.join("new.md"), "new knot").unwrap();

    let versioner =
        FileSystemGitVersioner::new(root.to_path_buf(), rig.clone());
    let result = versioner.commit(
        &LoomId("review-loom".to_string()),
        &KnotId("k".to_string()),
        &StrandPath(PathBuf::from("input/s.md")),
        "Modified",
        "tie-off",
    );
    assert!(result.is_ok(), "commit should succeed: {:?}", result.err());

    let files = tracked_at_head(root);
    assert!(
        files.contains(&"README.md".to_string()),
        "project change committed: {files:?}"
    );
    assert!(
        files.contains(&"rig/review-loom/k.md".to_string()),
        "pre-tracked rig file remains tracked (unstaged, not untracked): {files:?}"
    );
    assert!(
        !files.contains(&"rig/review-loom/new.md".to_string()),
        "new rig file must not be committed: {files:?}"
    );
    let committed = git_stdout(root, &["show", "HEAD:rig/review-loom/k.md"]);
    assert_eq!(
        committed, "knot",
        "committed rig file must keep its old content: {committed:?}"
    );
}

/// Degenerate case: the rig dir is the repo root itself. The exclusion
/// must be skipped — an empty pathspec would unstage everything — and the
/// commit proceeds with all changes intact.
#[test]
fn git_commit_skips_rig_exclusion_when_rig_is_repo_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_git_repo(root);
    std::fs::write(root.join("README.md"), "project").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "initial"]);

    std::fs::write(root.join("README.md"), "project v2").unwrap();

    let versioner =
        FileSystemGitVersioner::new(root.to_path_buf(), root.to_path_buf());
    let result = versioner.commit(
        &LoomId("l".to_string()),
        &KnotId("k".to_string()),
        &StrandPath(PathBuf::from("input/s.md")),
        "Modified",
        "tie-off",
    );
    assert!(result.is_ok(), "commit should succeed: {:?}", result.err());

    let files = tracked_at_head(root);
    assert!(
        files.contains(&"README.md".to_string()),
        "reset must not have unstaged everything: {files:?}"
    );
    let committed = git_stdout(root, &["show", "HEAD:README.md"]);
    assert_eq!(
        committed, "project v2",
        "the change must be committed when the exclusion is skipped"
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
