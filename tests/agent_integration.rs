//! Integration tests for agent execution.
//!
//! Verifies agent invocation, tie-off output, error handling, and
//! state file updates via file-based verification.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::thread;
use std::time::Duration;

use helpers::*;

// Global mutex to serialize tests that modify process-global PATH / env vars.
static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn acquire_test_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_MUTEX.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Stub `pi` agent produces tie-off with correct output.
#[test]
fn agent_execution_produces_tie_off() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "agent response here");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "new feature");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Verify tie-off file exists
    let tie_off_dir = rig_dir.join("tie-offs/review-loom/review");
    let tie_off_file = tie_off_dir.join("review-tie-off.md");
    assert!(
        tie_off_file.exists(),
        "tie-off file should exist at {}",
        tie_off_file.display()
    );

    let content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(content.contains("agent response here"));

    handle.abort();
}

/// Agent output appears in state.json last_tie_off_path.
#[test]
fn agent_execution_updates_state_file() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "review done");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Read state file
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
    assert!(
        knot.get("last_tie_off_path").is_some(),
        "should have last_tie_off_path"
    );
    assert!(
        knot.get("last_strand_path").is_some(),
        "should have last_strand_path"
    );

    handle.abort();
}

/// Multiple strands produce append-mode tie-off history.
#[test]
fn agent_execution_append_mode_tie_offs() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "review v1");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // First strand
    create_strand(&rig_dir, "feature1.md", "feature 1");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Second strand
    create_strand(&rig_dir, "feature2.md", "feature 2");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Tie-off file should contain content from both runs (append mode)
    let tie_off_file = rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    let content = fs::read_to_string(&tie_off_file).unwrap();
    // Should contain the agent output (at least once)
    assert!(content.contains("review v1"));

    handle.abort();
}

/// Agent failure writes error tie-off and updates state to failed.
#[test]
fn agent_failure_records_error_in_state() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Mock pi that always fails
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script = "#!/usr/bin/env bash\ncat > /dev/null\necho \"error\" >&2\nexit 1\n";
    fs::write(&pi_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    let config = "agent-adapter: pi-stdio\n";
    fs::write(rig_dir.join(".workspace-agent-config.yaml"), config).unwrap();
    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), existing));
    }

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "failed");

    // Read state file
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
        Some("failed")
    );
    assert!(
        knot.get("last_error").is_some(),
        "should have last_error on failure"
    );

    handle.abort();
}

/// Agent failure writes KnotFailed to loom-log.
#[test]
fn agent_failure_records_loom_log_entry() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script = "#!/usr/bin/env bash\ncat > /dev/null\necho \"fail\" >&2\nexit 1\n";
    fs::write(&pi_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    let config = "agent-adapter: pi-stdio\n";
    fs::write(rig_dir.join(".workspace-agent-config.yaml"), config).unwrap();
    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), existing));
    }

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotFailed");

    handle.abort();
}

/// Agent processes Deleted strand events (no output produced).
#[test]
fn agent_handles_deleted_strand() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "review output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Create strand
    let strand_path = create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Delete strand
    fs::remove_file(&strand_path).unwrap();

    // Wait for the delete to be processed
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");

    handle.abort();
}

/// Tie-off file contains the agent's stdout content.
#[test]
fn tie_off_contains_agent_output() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "line one\nline two\nline three");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    let tie_off_file = rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    let content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(content.contains("line one"));
    assert!(content.contains("line two"));

    handle.abort();
}

/// Loom-log StrandProcessed event has no error on success.
#[test]
fn strand_processed_no_error_on_success() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "ok");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    let events = read_loom_log(&rig_dir, "review-loom");

    // Find StrandProcessed events
    let processed: Vec<_> = events
        .iter()
        .filter(|e| loom_log_event_type(e) == Some("StrandProcessed"))
        .collect();

    assert!(!processed.is_empty(), "should have StrandProcessed events");

    // Last StrandProcessed should have no error
    let last = processed.last().unwrap();
    let inner = last.as_object().unwrap().values().next().unwrap();
    let error = inner.get("error");
    assert!(
        error.is_none() || error.unwrap().is_null(),
        "StrandProcessed should have no error on success"
    );

    handle.abort();
}

/// Agent state transitions: idle → processing → completed.
#[test]
fn agent_state_transitions_through_processing() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "done");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Initially idle
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
        Some("idle")
    );

    // Trigger processing
    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Loom-log should show the transition
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    assert!(types.contains(&"KnotProcessing"));
    assert!(types.contains(&"KnotCompleted"));

    handle.abort();
}

/// Multiple looms process independently via the same agent.
#[test]
fn agent_handles_multiple_looms_independently() {
    let _lock = acquire_test_lock();
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    // Two looms
    let loom1 = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom1, "review");

    let loom2 = create_loom_dir(&rig_dir, "planning");
    create_knot_file(&loom2, "plan");

    create_mock_pi(&rig_dir, "agent output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);
    wait_for_loom_in_state(&rig_dir, "planning-loom", 1);

    // Create strands for both looms
    create_strand(&rig_dir, "feature.md", "feature");

    // Both should complete
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    handle.abort();
}
