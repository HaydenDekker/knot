//! Integration tests for tie-off file output.
//!
//! Verifies tie-off path structure, append-mode history, and
//! markdown section formatting.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::thread;
use std::time::Duration;

use helpers::*;

/// Tie-off is written to the correct path under tie-offs/.
#[test]
fn tie_off_written_to_correct_path() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Verify tie-off at expected path: rig/tie-offs/{loom-id}/{knot-name}/{knot-id}-tie-off.md
    let tie_off = rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    assert!(
        tie_off.exists(),
        "tie-off should exist at {}",
        tie_off.display()
    );

    handle.abort();
}

/// Multiple runs append to the same tie-off file.
#[test]
fn tie_off_append_mode_history() {
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

    // Second strand — wait for it to be picked up and processed.
    // Knot status is already "completed" from first run, so we can't
    // use wait_for_knot_status_in_state to detect the second run.
    // Instead wait for the tie-off file to grow (second append).
    create_strand(&rig_dir, "feature2.md", "feature 2");
    let tie_off_path = rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    let first_size = fs::metadata(&tie_off_path)
        .map(|m| m.len())
        .unwrap_or(0);
    for _ in 0..60 {
        thread::sleep(Duration::from_millis(100));
        if let Ok(m) = fs::metadata(&tie_off_path) {
            if m.len() > first_size {
                break;
            }
        }
    }

    let tie_off = rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    let content = fs::read_to_string(&tie_off).unwrap();

    // Should contain output from multiple runs
    let count = content.matches("review v1").count();
    assert!(
        count >= 2,
        "tie-off should contain at least 2 entries (append mode), got {}",
        count
    );

    handle.abort();
}
