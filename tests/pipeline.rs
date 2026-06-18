//! Integration tests for the event processing pipeline.
//!
//! Verifies: Notify → debounce → ProcessStrand → tie-off.
//! Covers strand lifecycle (create/modify/delete), state file polling,
//! and loom-log event verification.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::thread;
use std::time::Duration;

use helpers::*;

/// A strand file triggers the full pipeline: KnotProcessing → agent run → tie-off → KnotCompleted.
#[test]
fn pipeline_processes_strand_create() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Create a mock pi binary that returns deterministic output
    let _pi_path = create_mock_pi(&rig_dir, "review output");

    // Start Knot in background
    let handle = start_knot(rig_dir.clone());

    // Wait for the loom to be discovered
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Write a strand file to trigger processing
    let strand_path = create_strand(&rig_dir, "feature.md", "new feature request");

    // Wait for knot to transition to completed
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Verify loom-log has KnotCompleted event
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotCompleted");

    // Verify tie-off was written
    let tie_off_dir = rig_dir.join("tie-offs/review-loom/review");
    thread::sleep(Duration::from_millis(500));
    let tie_off_file = tie_off_dir.join("review-tie-off.md");
    assert!(
        tie_off_file.exists(),
        "tie-off file should exist at {}",
        tie_off_file.display()
    );

    let tie_off_content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        tie_off_content.contains("review output"),
        "tie-off should contain agent output"
    );

    drop(strand_path);
    handle.abort();
}

/// Modifying a strand file triggers reprocessing.
#[test]
fn pipeline_reprocesses_on_strand_modify() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "review output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Create initial strand
    let strand_path = create_strand(&rig_dir, "feature.md", "v1");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Modify the strand — triggers reprocessing
    thread::sleep(Duration::from_millis(500));
    fs::write(&strand_path, "v2 modified").unwrap();

    // Wait for KnotCompleted again (reprocessing)
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut completed_count = 0;
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let events = read_loom_log(&rig_dir, "review-loom");
        for event in &events {
            if let Some(ty) = loom_log_event_type(&event) {
                if ty == "KnotCompleted" {
                    completed_count += 1;
                }
            }
        }
        if completed_count >= 2 {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    assert!(
        completed_count >= 2,
        "should have at least 2 KnotCompleted events (original + reprocess), got {}",
        completed_count
    );

    handle.abort();
}

/// Deleting a strand file triggers the pipeline (Deleted event).
#[test]
fn pipeline_handles_strand_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "review output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Create a strand
    let strand_path = create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Delete the strand
    thread::sleep(Duration::from_millis(500));
    fs::remove_file(&strand_path).unwrap();

    // Wait for StrandProcessed event (the delete triggers processing too)
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut found_processed = false;
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let events = read_loom_log(&rig_dir, "review-loom");
        for event in &events {
            if let Some(ty) = loom_log_event_type(&event) {
                if ty == "StrandProcessed" {
                    found_processed = true;
                }
            }
        }
        if found_processed {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    assert!(found_processed, "should have StrandProcessed event after delete");

    handle.abort();
}

/// The debounce engine prevents rapid-fire events from causing duplicate processing.
#[test]
fn pipeline_debounces_rapid_strand_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "review output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Write strand — triggers processing
    let strand_path = create_strand(&rig_dir, "feature.md", "v1");

    // Rapidly modify the strand multiple times
    for i in 0..5 {
        fs::write(&strand_path, &format!("v{}", i + 2)).unwrap();
        thread::sleep(Duration::from_millis(10));
    }

    // Wait for processing to complete
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Count KnotProcessing events — should be limited by debounce
    thread::sleep(Duration::from_millis(1000));
    let events = read_loom_log(&rig_dir, "review-loom");
    let processing_count = events
        .iter()
        .filter(|e| {
            loom_log_event_type(e)
                .map(|t| t == "KnotProcessing")
                .unwrap_or(false)
        })
        .count();

    // With debounce, should have far fewer processing events than modifications
    // (exact count depends on debounce window, but should be < 6)
    assert!(
        processing_count < 6,
        "debounce should limit processing events, got {}",
        processing_count
    );

    handle.abort();
}

/// State file reflects processing status changes during the pipeline.
#[test]
fn state_file_reflects_pipeline_progress() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "review output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Before any strand, knot should be idle
    let state = read_state_file(&rig_dir).unwrap();
    let knots = state
        .get("looms")
        .and_then(|v| v.as_array())
        .unwrap()[0]
        .get("knots")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(
        knots[0].get("status").and_then(|v| v.as_str()),
        Some("idle")
    );

    // Write a strand — triggers processing
    create_strand(&rig_dir, "feature.md", "new feature");

    // Eventually knot should be completed
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Read state file and verify
    let state = read_state_file(&rig_dir).unwrap();
    let knots = state
        .get("looms")
        .and_then(|v| v.as_array())
        .unwrap()[0]
        .get("knots")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(
        knots[0].get("status").and_then(|v| v.as_str()),
        Some("completed")
    );
    // Should have a tie-off path
    assert!(
        knots[0].get("last_tie_off_path").is_some(),
        "should have tie-off path"
    );

    handle.abort();
}

/// Loom-log contains the full event sequence for a strand processing.
#[test]
fn loom_log_contains_full_event_sequence() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "review output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Read loom-log and verify event sequence
    thread::sleep(Duration::from_millis(500));
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<&str> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    // Should have: KnotRegistered, LoomStarted, KnotProcessing,
    // KnotCompleted, StrandProcessed
    assert!(
        types.contains(&"KnotRegistered"),
        "should have KnotRegistered. Events: {:?}",
        types
    );
    assert!(
        types.contains(&"LoomStarted"),
        "should have LoomStarted. Events: {:?}",
        types
    );
    assert!(
        types.contains(&"KnotProcessing"),
        "should have KnotProcessing. Events: {:?}",
        types
    );
    assert!(
        types.contains(&"KnotCompleted"),
        "should have KnotCompleted. Events: {:?}",
        types
    );
    assert!(
        types.contains(&"StrandProcessed"),
        "should have StrandProcessed. Events: {:?}",
        types
    );

    handle.abort();
}

/// The pipeline handles agent execution errors gracefully.
#[test]
fn pipeline_handles_agent_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Create a mock pi that always fails (exit 1)
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
    let config = format!(
        "cli_path: \"{}\"\n\
         cli_args: []\n",
        pi_path.display()
    );
    fs::write(rig_dir.join(".workspace-agent-config.yaml"), config).unwrap();

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Write a strand — will fail because mock pi exits with code 1
    create_strand(&rig_dir, "feature.md", "content");

    // Wait for failure
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "failed");

    // Verify loom-log has KnotFailed event
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotFailed");

    handle.abort();
}
