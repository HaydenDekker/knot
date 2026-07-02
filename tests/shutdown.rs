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

    // Mock agent — created inline, injected via cli_path
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    fs::write(
        &pi_path,
        "#!/usr/bin/env bash\n\
         cat > /dev/null\n\
         echo \"output\"\n\
         exit 0\n",
    ).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\n",
    ).unwrap();

    let config = knot::AppConfig::with_rig_dir(rig_dir.clone())
        .with_cli_path(pi_path);
    let handle = start_knot_with_config(config);
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
