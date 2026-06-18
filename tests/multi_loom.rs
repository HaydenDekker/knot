//! Integration tests for multi-loom scenarios.
//!
//! Verifies isolation between looms and independent processing.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::thread;
use std::time::Duration;

use helpers::*;

/// Multiple looms in the same rig process independently.
#[test]
fn multi_loom_independent_processing() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let loom1 = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom1, "review");

    let loom2 = create_loom_dir(&rig_dir, "planning");
    create_knot_file(&loom2, "plan");

    create_mock_pi(&rig_dir, "output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);
    wait_for_loom_in_state(&rig_dir, "planning-loom", 1);

    // Create strands for both looms
    create_strand(&rig_dir, "feature.md", "feature content");

    // Both should process
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Verify both looms in state
    let state = read_state_file(&rig_dir).unwrap();
    let looms = state.get("looms").and_then(|v| v.as_array()).unwrap();
    assert_eq!(looms.len(), 2);

    handle.abort();
}

/// Loom-log files are isolated per loom (each loom writes to its own .loom-log).
#[test]
fn multi_loom_log_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let loom1 = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom1, "review");

    let loom2 = create_loom_dir(&rig_dir, "planning");
    create_knot_file(&loom2, "plan");

    create_mock_pi(&rig_dir, "output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);
    wait_for_loom_in_state(&rig_dir, "planning-loom", 1);

    // Both looms share ./strands so both pick up the strand
    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_knot_status_in_state(&rig_dir, "planning-loom", "plan", "completed");

    // Verify each loom has its own log file with the right events
    let log1 = read_loom_log(&rig_dir, "review-loom");
    let log2 = read_loom_log(&rig_dir, "planning-loom");

    // Each loom log should have events for its own loom
    let log1_review = log1
        .iter()
        .any(|e| loom_log_event_type(e) == Some("KnotCompleted"));
    let log2_planning = log2
        .iter()
        .any(|e| loom_log_event_type(e) == Some("KnotCompleted"));
    assert!(log1_review, "review-loom log should have KnotCompleted");
    assert!(log2_planning, "planning-loom log should have KnotCompleted");

    // review-loom's log should not contain planning-loom's knot events and vice versa
    let log1_has_plan = log1
        .iter()
        .any(|e| e.get("knot_id").and_then(|v| v.as_str()) == Some("plan"));
    let log2_has_review = log2
        .iter()
        .any(|e| e.get("knot_id").and_then(|v| v.as_str()) == Some("review"));
    assert!(!log1_has_plan, "review-loom log should not have plan knot events");
    assert!(!log2_has_review, "planning-loom log should not have review knot events");

    handle.abort();
}
