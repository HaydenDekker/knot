//! Integration tests for git versioning.
//!
//! Verifies that git commits are created after successful strand
//! processing when the knot has `git-versioned: true`.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::thread;
use std::time::Duration;

use helpers::*;

/// Git commit is created after successful processing (when git-versioned is true).
#[test]
fn git_commit_created_after_processing() {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path();
    let rig_dir = project_root.join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    // Initialize git repo
    init_git_repo(project_root);
    let initial_commits = count_commits(project_root);

    let loom_dir = create_loom_dir(&rig_dir, "review");
    // Create knot with git-versioned: true
    let content = make_git_versioned_knot("review", "fast", "./strands");
    fs::write(loom_dir.join("review.md"), content).unwrap();
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Wait for git commit to complete
    thread::sleep(Duration::from_millis(1000));

    let final_commits = count_commits(project_root);
    assert!(
        final_commits > initial_commits,
        "should have at least 1 new commit after processing"
    );

    handle.abort();
}

/// Git commit is NOT created when git-versioned is false.
#[test]
fn no_git_commit_when_not_versioned() {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path();
    let rig_dir = project_root.join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    init_git_repo(project_root);
    let initial_commits = count_commits(project_root);

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review"); // git-versioned defaults to false
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    thread::sleep(Duration::from_millis(1000));

    let final_commits = count_commits(project_root);
    assert_eq!(
        final_commits, initial_commits,
        "should have no new commits when git-versioned is false"
    );

    handle.abort();
}

/// State file is updated even when git versioning is disabled.
#[test]
fn state_updated_without_git_versioning() {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path();
    let rig_dir = project_root.join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // State should show completed status
    let state = read_state_file(&rig_dir).unwrap();
    let knot = state
        .get("looms")
        .and_then(|v| v.as_array())
        .unwrap().get(0).unwrap()
        .get("knots")
        .and_then(|v| v.as_array())
        .unwrap().get(0).unwrap();
    assert_eq!(
        knot.get("status").and_then(|v| v.as_str()),
        Some("completed")
    );

    handle.abort();
}

/// Create a knot definition with git-versioned: true.
fn make_git_versioned_knot(
    name: &str,
    agent_profile_ref: &str,
    strand_dir: &str,
) -> String {
    [
        "---",
        &format!("name: {name}"),
        &format!("agent-profile-ref: {agent_profile_ref}"),
        &format!("strand-dir: \"{strand_dir}\""),
        "git-versioned: true",
        "prompt-template:",
        "  instructions: |",
        &format!("    Test knot: {name}."),
        "---",
        "",
        &format!("# {name}"),
        "",
        "Test knot definition.",
        "",
    ].join("\n")
}
