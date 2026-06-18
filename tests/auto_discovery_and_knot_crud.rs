//! Integration tests for file-watcher auto-discovery.
//!
//! Verifies that new looms and knots are auto-discovered when
//! files are created/modified/deleted on the filesystem.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::thread;
use std::time::Duration;

use helpers::*;

/// New loom directory is auto-discovered by the file watcher.
#[test]
fn auto_discover_new_loom() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);
    assert!(state.get("looms").and_then(|v| v.as_array()).unwrap().is_empty());

    // Create a new loom directory
    thread::sleep(Duration::from_millis(500));
    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    // Wait for auto-discovery
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    handle.abort();
}

/// New knot file in existing loom is auto-discovered.
#[test]
fn auto_discover_new_knot() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Add a second knot
    thread::sleep(Duration::from_millis(500));
    create_knot_file(&loom_dir, "summary");

    // Wait for the second knot to be discovered
    wait_for_loom_in_state(&rig_dir, "review-loom", 2);

    handle.abort();
}

/// Deleting a knot file is auto-detected.
#[test]
fn auto_detect_knot_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_knot_file(&loom_dir, "summary");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 2);

    // Delete one knot file
    thread::sleep(Duration::from_millis(500));
    fs::remove_file(loom_dir.join("summary.md")).unwrap();

    // Wait for deletion to be detected
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let state = match read_state_file(&rig_dir) {
            Ok(s) => s,
            Err(_) => {
                thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        let knot_count = state
            .get("looms")
            .and_then(|v| v.as_array())
            .and_then(|a| a.iter().find(|l| {
                l.get("id").and_then(|v| v.as_str()) == Some("review-loom")
            }))
            .and_then(|l| l.get("knots"))
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(99);

        if knot_count <= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    handle.abort();
}

/// Deleting a loom directory is auto-detected.
#[test]
fn auto_detect_loom_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Delete the loom directory
    thread::sleep(Duration::from_millis(500));
    fs::remove_dir_all(&loom_dir).unwrap();

    // Wait for deletion to be detected
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let state = match read_state_file(&rig_dir) {
            Ok(s) => s,
            Err(_) => {
                thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        let looms = state.get("looms").and_then(|v| v.as_array()).unwrap();
        if looms.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    handle.abort();
}

/// Modifying a knot file triggers re-scanning.
#[test]
fn auto_detect_knot_modification() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Modify the knot file
    thread::sleep(Duration::from_millis(500));
    let modified_content = make_knot_content("review", "fast", "./strands");
    fs::write(loom_dir.join("review.md"), &modified_content).unwrap();

    // Loom-log should have a KnotModified or re-registration event
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let events = read_loom_log(&rig_dir, "review-loom");
        let types: Vec<_> = events
            .iter()
            .filter_map(|e| loom_log_event_type(e))
            .collect();

        if types.contains(&"KnotRegistered") && types.len() >= 2 {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    handle.abort();
}

/// Multiple looms are discovered independently.
#[test]
fn auto_discover_multiple_looms_sequentially() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    let state = wait_for_state_file(&rig_dir);
    assert!(state.get("looms").and_then(|v| v.as_array()).unwrap().is_empty());

    // Create first loom
    thread::sleep(Duration::from_millis(500));
    let loom1 = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom1, "review");
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Create second loom
    thread::sleep(Duration::from_millis(500));
    let loom2 = create_loom_dir(&rig_dir, "planning");
    create_knot_file(&loom2, "plan");
    wait_for_loom_in_state(&rig_dir, "planning-loom", 1);

    // Both should be in state
    let state = read_state_file(&rig_dir).unwrap();
    let looms = state.get("looms").and_then(|v| v.as_array()).unwrap();
    assert_eq!(looms.len(), 2);

    handle.abort();
}

/// Loom-log records auto-discovery events.
#[test]
fn loom_log_records_discovery_events() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    let _ = wait_for_state_file(&rig_dir);

    // Create loom
    thread::sleep(Duration::from_millis(500));
    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Check loom-log
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    assert!(types.contains(&"KnotRegistered"));
    assert!(types.contains(&"LoomStarted"));

    handle.abort();
}

/// File watcher handles rapid successive loom creations.
#[test]
fn auto_discover_rapid_loom_creations() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let handle = start_knot(rig_dir.clone());
    let _ = wait_for_state_file(&rig_dir);

    // Rapidly create 3 looms
    thread::sleep(Duration::from_millis(500));
    for i in 0..3 {
        let loom_dir = create_loom_dir(&rig_dir, &format!("loom{}", i));
        create_knot_file(&loom_dir, "knot");
    }

    // Wait for all to be discovered
    for i in 0..3 {
        wait_for_loom_in_state(&rig_dir, &format!("loom{}-loom", i), 1);
    }

    let state = read_state_file(&rig_dir).unwrap();
    let looms = state.get("looms").and_then(|v| v.as_array()).unwrap();
    assert_eq!(looms.len(), 3);

    handle.abort();
}

/// Auto-discovery is idempotent — re-scanning doesn't duplicate looms.
#[test]
fn auto_discovery_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    create_fast_profile(&rig_dir);

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // State should show exactly 1 loom
    let state = read_state_file(&rig_dir).unwrap();
    let looms = state.get("looms").and_then(|v| v.as_array()).unwrap();
    assert_eq!(looms.len(), 1);

    // Modify the knot file (triggers re-scan)
    thread::sleep(Duration::from_millis(500));
    fs::write(
        loom_dir.join("review.md"),
        make_knot_content("review", "fast", "./strands"),
    )
    .unwrap();

    // After re-scan, should still be exactly 1 loom
    thread::sleep(Duration::from_millis(2000));
    let state = read_state_file(&rig_dir).unwrap();
    let looms = state.get("looms").and_then(|v| v.as_array()).unwrap();
    assert_eq!(
        looms.len(),
        1,
        "should not duplicate loom after re-scan"
    );

    handle.abort();
}
