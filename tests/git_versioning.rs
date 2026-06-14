//! Integration tests for git versioning in the full event pipeline.
//!
//! Verifies that git commits are created during normal processing,
//! skipped when `git-versioned: false`, and processing continues
//! gracefully without a git repository.

mod helpers;

use std::fs;
use std::path::PathBuf;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

/// Knot content with `git-versioned: false` in frontmatter.
/// Returns (knot_content, strand_dir).
fn make_knot_content_with_dirs_git_disabled(
    project_root: &std::path::Path,
) -> (String, PathBuf) {
    let strand_dir = project_root.join("strands");
    fs::create_dir_all(&strand_dir).unwrap();
    let content = format!(
        "---\nname: review-knot\nagent-profile-ref: fast\nstrand-dir: \
         \"{}\"\ngit-versioned: false\nprompt-template:\n  input-bundling: \
         \"full-file\"\n  instructions: |\n    Review the goals section of \
         this PRD.\n---\n\n# Review Knot\n\nThis knot reviews PRD goals.\n",
        strand_dir.display()
    );
    (content, strand_dir)
}

// ── Test: Pipeline creates git commit ──────────────────────────────────────

/// Full pipeline test with a git repository.
///
/// Verifies that after a knot processes a strand successfully, a git
/// commit is created in the project root with the correct message
/// format containing knot id, strand name, event type, and tie-off
/// content in the body.
///
/// Setup: temp dir → git init → rig subdirectory as rig_dir → loom +
/// knot (git-versioned default: true) → mock agent → spawn server →
/// create strand → wait for completion → verify git commit.
#[tokio::test]
async fn event_pipeline_creates_git_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Initialise git at the project root (parent of rig_dir).
    assert!(
        init_git_repo(root),
        "should initialise git repo in temp directory"
    );

    // Rig subdirectory (this is what the server scans).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory with knot definition (git-versioned defaults to true).
    let loom_dir = rig.join("git-test-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(root);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Agent profile.
    create_fast_profile(&rig);

    // Mock agent script.
    let mock_agent = create_mock_agent(root, "agent processed this strand");

    let port = 32010;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: rig.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    // Count commits before processing.
    let commits_before =
        count_commits(root).expect("should count commits");

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand in the loom source directory.
    let strand_path = strand_dir.join("versioned-strand.md");
    fs::write(&strand_path, "versioned strand content").unwrap();

    // Wait for processing to complete.
    let status =
        poll_knot_status(&host_port, "git-test-loom", "review-knot", 60, 100)
            .await
            .expect("knot status should reach terminal state");
    assert_eq!(
        status["status"].as_str().unwrap(),
        "completed",
        "knot should complete successfully"
    );

    // Verify a new commit was created in the project root.
    let commits_after =
        count_commits(root).expect("should count commits after");
    assert!(
        commits_after > commits_before,
        "should have created at least one new commit (before={}, after={})",
        commits_before,
        commits_after
    );

    // Verify commit message format.
    let (subject, body) = get_latest_commit(root)
        .expect("should read latest commit");

    // Subject: "knot: <knot-id> — processed <strand-name> (<event-type>)"
    // Note: notify emits both Created and Modified for a file write;
    // the debounce may process either, so we accept both.
    assert!(
        subject.starts_with("knot: "),
        "subject should start with 'knot: ', got: {subject}"
    );
    assert!(
        subject.contains("review-knot"),
        "subject should contain knot id, got: {subject}"
    );
    assert!(
        subject.contains("versioned-strand.md"),
        "subject should contain strand name, got: {subject}"
    );
    assert!(
        subject.contains("Created") || subject.contains("Modified"),
        "subject should contain event type (Created or Modified), got: \
         {subject}"
    );

    // Body should contain the tie-off content (agent output).
    assert!(
        body.contains("agent processed this strand"),
        "commit body should contain tie-off content, got: {body}"
    );
}

// ── Test: Pipeline skips git when disabled ────────────────────────────────

/// Full pipeline test with `git-versioned: false` on the knot.
///
/// Verifies that when a knot has `git-versioned: false` in its
/// frontmatter, no git commit is created even though processing
/// completes successfully.
///
/// Setup: temp dir → git init → rig subdirectory → loom + knot with
/// `git-versioned: false` → mock agent → spawn server → create strand →
/// verify no new commit.
#[tokio::test]
async fn event_pipeline_skips_git_when_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Initialise git at the project root.
    assert!(
        init_git_repo(root),
        "should initialise git repo in temp directory"
    );

    // Rig subdirectory.
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom with knot that has git-versioned: false.
    let loom_dir = rig.join("no-git-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) =
        make_knot_content_with_dirs_git_disabled(root);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Agent profile.
    create_fast_profile(&rig);

    // Mock agent script.
    let mock_agent = create_mock_agent(root, "no-git-agent-output");

    let port = 32011;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: rig.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    // Count commits before processing.
    let commits_before =
        count_commits(root).expect("should count commits");

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand.
    let strand_path = strand_dir.join("no-git-strand.md");
    fs::write(&strand_path, "no git strand content").unwrap();

    // Wait for processing to complete.
    let status =
        poll_knot_status(&host_port, "no-git-loom", "review-knot", 60, 100)
            .await
            .expect("knot status should reach terminal state");
    assert_eq!(
        status["status"].as_str().unwrap(),
        "completed",
        "knot should complete successfully even with git-versioned: false"
    );

    // Verify tie-off was still written.
    let tie_off_path =
        rig.join("tie-offs/no-git-loom/review-knot/review-knot-tie-off.md");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist even with git-versioned: false"
    );

    // Verify NO new commit was created.
    let commits_after =
        count_commits(root).expect("should count commits after");
    assert_eq!(
        commits_after, commits_before,
        "should NOT have created a git commit when git-versioned: false \
         (before={}, after={})",
        commits_before,
        commits_after
    );
}

// ── Test: Pipeline continues without git ──────────────────────────────────

/// Full pipeline test without a git repository.
///
/// Verifies that processing completes normally when the project root
/// is not a git repository. Git versioning should gracefully skip
/// without failing the pipeline.
///
/// Setup: temp dir (NO git init) → rig subdirectory → loom + knot →
/// mock agent → spawn server → create strand → verify completion.
#[tokio::test]
async fn event_pipeline_continues_without_git() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // NO git init — plain directory.

    // Rig subdirectory.
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom with knot (git-versioned defaults to true, but there's no repo).
    let loom_dir = rig.join("no-repo-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(root);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Agent profile.
    create_fast_profile(&rig);

    // Mock agent script.
    let mock_agent = create_mock_agent(root, "no-repo-agent-output");

    let port = 32012;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: rig.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand.
    let strand_path = strand_dir.join("no-repo-strand.md");
    fs::write(&strand_path, "no repo strand content").unwrap();

    // Wait for processing to complete.
    let status =
        poll_knot_status(&host_port, "no-repo-loom", "review-knot", 60, 100)
            .await
            .expect("knot status should reach terminal state");
    assert_eq!(
        status["status"].as_str().unwrap(),
        "completed",
        "knot should complete successfully without a git repo"
    );
    assert!(
        status["last_error"].is_null(),
        "knot should have no error when git is unavailable"
    );

    // Verify tie-off was still written.
    let tie_off_path =
        rig.join("tie-offs/no-repo-loom/review-knot/review-knot-tie-off.md");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist even without a git repo"
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("no-repo-agent-output"),
        "tie-off should contain agent output, got: {content}"
    );
}
