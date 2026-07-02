//! Composition tests for multi-loom scenarios.
//!
//! Verifies isolation between looms and independent processing.
//! These tests spin up the full Knot runtime with mock agent via
//! \`cli_path\` injection — no \`TEST_MUTEX\`, no PATH manipulation,
//! each test uses a unique \`tempfile::tempdir()\`.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use helpers::*;

// ── Helper ─────────────────────────────────────────────────────────────

/// Create a mock \`pi\` binary and return its path.
/// Each test creates its own mock in its own tempdir — no shared state.
fn create_mock_pi_in_dir(rig_dir: &std::path::Path, response: &str) -> std::path::PathBuf {
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script = format!(
        "#!/usr/bin/env bash\n\
         # Mock pi for multi-loom test - consumes stdin, echoes response\n\
         cat > /dev/null\n\
         echo \"{response}\"\n\
         exit 0\n"
    );
    fs::write(&pi_path, script).unwrap();
    fs::set_permissions(&pi_path, PermissionsExt::from_mode(0o755)).unwrap();

    // Config selects pi-stdio adapter; cli_path points to the mock.
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\n",
    )
    .unwrap();

    pi_path
}

// ── Tests ──────────────────────────────────────────────────────────────

/// Multiple looms in the same rig process independently.
///
/// Two looms (review-loom, planning-loom) are registered. Both share
/// the same strand directory so both pick up the same strand. Each
/// loom processes independently and records its own events.
#[test]
fn multi_loom_independent_processing() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    // Loom 1: review
    let loom1 = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom1, "review");

    // Loom 2: planning
    let loom2 = create_loom_dir(&rig_dir, "planning");
    create_knot_file(&loom2, "plan");

    // Mock agent
    let pi_path = create_mock_pi_in_dir(&rig_dir, "output");

    // Start Knot with cli_path injection (no PATH manipulation)
    let config = knot::AppConfig::with_rig_dir(rig_dir.clone())
        .with_cli_path(pi_path);
    let handle = start_knot_with_config(config);

    // Wait for both looms to be discovered
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);
    wait_for_loom_in_state(&rig_dir, "planning-loom", 1);

    // Create a strand — both looms should pick it up (same strand dir)
    create_strand(&rig_dir, "feature.md", "feature content");

    // Both knots should complete
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_knot_status_in_state(&rig_dir, "planning-loom", "plan", "completed");

    // Verify both looms in state
    let state = read_state_file(&rig_dir).unwrap();
    let looms = state.get("looms").and_then(|v| v.as_array()).unwrap();
    assert_eq!(looms.len(), 2, "should have 2 looms in state");

    handle.abort();
}

/// Loom-log files are isolated per loom.
///
/// Each loom writes to its own \`{rig}/tie-offs/{loom-id}/.loom-log\`.
/// Review-loom's log should NOT contain planning-loom's knot events
/// and vice versa.
#[test]
fn multi_loom_log_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    // Loom 1: review
    let loom1 = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom1, "review");

    // Loom 2: planning
    let loom2 = create_loom_dir(&rig_dir, "planning");
    create_knot_file(&loom2, "plan");

    // Mock agent
    let pi_path = create_mock_pi_in_dir(&rig_dir, "output");

    // Start Knot with cli_path injection
    let config = knot::AppConfig::with_rig_dir(rig_dir.clone())
        .with_cli_path(pi_path);
    let handle = start_knot_with_config(config);

    wait_for_loom_in_state(&rig_dir, "review-loom", 1);
    wait_for_loom_in_state(&rig_dir, "planning-loom", 1);

    // Both looms share ./strands so both pick up the strand
    create_strand(&rig_dir, "feature.md", "content");

    // Wait for both to complete
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_knot_status_in_state(&rig_dir, "planning-loom", "plan", "completed");

    // Verify each loom has its own log file with the right events
    let log1 = read_loom_log(&rig_dir, "review-loom");
    let log2 = read_loom_log(&rig_dir, "planning-loom");

    // Each loom log should have KnotCompleted
    let log1_has_completed = log1.iter().any(|e| {
        loom_log_event_type(e) == Some("KnotCompleted")
    });
    let log2_has_completed = log2.iter().any(|e| {
        loom_log_event_type(e) == Some("KnotCompleted")
    });
    assert!(
        log1_has_completed,
        "review-loom log should have KnotCompleted"
    );
    assert!(
        log2_has_completed,
        "planning-loom log should have KnotCompleted"
    );

    // review-loom's log should NOT contain planning-loom's knot events
    let log1_has_plan = log1.iter().any(|e| {
        e.get("knot_id").and_then(|v| v.as_str()) == Some("plan")
    });
    let log2_has_review = log2.iter().any(|e| {
        e.get("knot_id").and_then(|v| v.as_str()) == Some("review")
    });
    assert!(
        !log1_has_plan,
        "review-loom log should not have plan knot events"
    );
    assert!(
        !log2_has_review,
        "planning-loom log should not have review knot events"
    );

    handle.abort();
}
