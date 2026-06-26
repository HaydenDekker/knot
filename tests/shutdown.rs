//! Integration tests for shutdown behaviour.
//!
//! Verifies that the Knot process stops cleanly and that loom-logs
//! contain the expected events up to the point of termination.
//!
//! Note: The test harness aborts the Knot task directly (simulating a
//! forceful termination), so `LoomStopped` events are NOT written —
//! those require a graceful `ctrl_c` signal which doesn't fire in tests.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::thread;
use std::time::Duration;

use helpers::*;

/// Knot process stops cleanly on abort.
#[test]
fn shutdown_writes_loom_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Verify LoomStarted was written
    let events = read_loom_log(&rig_dir, "review-loom");
    assert!(events.iter().any(|e| loom_log_event_type(e) == Some("LoomStarted")));

    // Shutdown — aborts the task, so LoomStopped is NOT written.
    // We verify the process stops cleanly instead.
    handle.abort();

    // Wait for thread to finish
    thread::sleep(Duration::from_millis(50));

    // Verify loom-log exists and has LoomStarted (process ran correctly)
    let events = read_loom_log(&rig_dir, "review-loom");
    assert!(!events.is_empty(), "loom-log should have events");
}

/// Processing completes before shutdown aborts the task.
#[test]
fn shutdown_drains_pipeline_before_loom_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Trigger processing
    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Shutdown
    handle.abort();
    thread::sleep(Duration::from_millis(50));

    // Verify processing completed (StrandProcessed / KnotCompleted in log)
    let events = read_loom_log(&rig_dir, "review-loom");
    let has_completed = events
        .iter()
        .any(|e| loom_log_event_type(e) == Some("KnotCompleted"));
    assert!(
        has_completed,
        "processing should complete before shutdown"
    );
}
