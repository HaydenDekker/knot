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

/// Loom activity is isolated per loom.
///
/// Plan 083: per-loom activity lives in the in-memory run store (keyed
/// by loom), so file-level isolation is now observable through each
/// loom's own tie-off file: review-loom's tie-off must not contain
/// planning-loom's knot sections and vice versa.
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

    // Each loom produced its own tie-off with its own knot's section
    let runtime_root = knot::domain::knot_file::derive_runtime_root(&rig_dir);
    let review_tie_off = runtime_root.join("review-loom").join("tie-off-review.md");
    let plan_tie_off = runtime_root.join("planning-loom").join("tie-off-plan.md");
    let review_content = fs::read_to_string(&review_tie_off)
        .expect("review-loom tie-off should exist");
    let plan_content = fs::read_to_string(&plan_tie_off)
        .expect("planning-loom tie-off should exist");
    assert!(
        review_content.contains("## review triggered by"),
        "review-loom tie-off should carry the review knot's section"
    );
    assert!(
        plan_content.contains("## plan triggered by"),
        "planning-loom tie-off should carry the plan knot's section"
    );

    // review-loom's record must NOT contain planning-loom's knot work
    assert!(
        !review_content.contains("## plan "),
        "review-loom record should not have plan knot events"
    );
    assert!(
        !plan_content.contains("## review "),
        "planning-loom record should not have review knot events"
    );

    handle.abort();
}
