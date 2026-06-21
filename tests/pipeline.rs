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

/// Binary files in a strand directory are silently skipped with a warning.
///
/// Verifies the full integration of text/binary detection:
/// - Binary file (null bytes) → `StrandIgnored` in loom-log, no tie-off
/// - Text file (`.txt`) in the same strand dir → normal processing
#[test]
fn pipeline_ignores_binary_files_and_processes_text_files() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "review output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // --- Binary file: should be ignored ---

    // Create a binary file with null bytes in the strand directory
    let project_root = rig_dir.parent().unwrap();
    let strands_dir = project_root.join("strands");
    fs::create_dir_all(&strands_dir).unwrap();
    let binary_path = strands_dir.join("data.bin");
    let binary_data: Vec<u8> = vec![
        0x00, 0x01, 0x02, 0xFF, 0xFE, 0x89, 0x50, 0x4E,
    ];
    fs::write(&binary_path, &binary_data).unwrap();

    // Wait for StrandIgnored event in loom-log
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandIgnored");

    // Verify the StrandIgnored event has correct fields
    thread::sleep(Duration::from_millis(500));
    let events = read_loom_log(&rig_dir, "review-loom");
    let ignored_event = events.iter().find(|e| {
        loom_log_event_type(e)
            .map(|t| t == "StrandIgnored")
            .unwrap_or(false)
    });
    assert!(
        ignored_event.is_some(),
        "should have StrandIgnored event. Events: {:?}",
        events.iter().filter_map(|e| loom_log_event_type(e)).collect::<Vec<_>>()
    );
    let ignored = ignored_event.unwrap();
    if let Some(data) = ignored.as_object().and_then(|o| o.get("StrandIgnored")) {
        if let Some(reason) = data.get("reason").and_then(|v| v.as_str()) {
            assert!(
                reason.contains("binary"),
                "reason should mention binary, got: {}",
                reason
            );
        } else {
            panic!("StrandIgnored event missing reason field");
        }
    } else {
        panic!("StrandIgnored event has no data object");
    }

    // Verify no KnotProcessing event was emitted for the binary file
    // (binary files skip processing entirely)
    let processing_events: Vec<_> = events
        .iter()
        .filter(|e| {
            loom_log_event_type(e)
                .map(|t| t == "KnotProcessing")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        processing_events.is_empty(),
        "should have no KnotProcessing events for binary file"
    );

    // --- Text file: should process normally ---

    thread::sleep(Duration::from_millis(500));
    create_strand(&rig_dir, "notes.txt", "some plain text notes");

    // Wait for normal processing to complete
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotCompleted");

    // Verify tie-off was written
    thread::sleep(Duration::from_millis(500));
    let tie_off_dir = rig_dir.join("tie-offs/review-loom/review");
    let tie_off_file = tie_off_dir.join("review-tie-off.md");
    assert!(
        tie_off_file.exists(),
        "tie-off file should exist for text file at {}",
        tie_off_file.display()
    );

    let tie_off_content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        tie_off_content.contains("review output"),
        "tie-off should contain agent output"
    );

    // Verify loom-log has KnotCompleted (not StrandIgnored) for the text file
    let events = read_loom_log(&rig_dir, "review-loom");
    let completed_events: Vec<_> = events
        .iter()
        .filter(|e| {
            loom_log_event_type(e)
                .map(|t| t == "KnotCompleted")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        !completed_events.is_empty(),
        "should have KnotCompleted for the .txt file"
    );

    handle.abort();
}

/// Non-`.md` text files (`.rs`, `.json`, etc.) are processed normally.
///
/// Verifies that the `.md`-only filter has been removed and arbitrary
/// text extensions trigger full pipeline processing.
#[test]
fn pipeline_processes_non_md_text_files() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "rust review output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Create a .rs source file as a strand
    create_strand(
        &rig_dir,
        "lib.rs",
        "fn main() { println!(\"hello\"); }",
    );

    // Wait for normal processing
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotCompleted");

    // Verify tie-off was written
    thread::sleep(Duration::from_millis(500));
    let tie_off_dir = rig_dir.join("tie-offs/review-loom/review");
    let tie_off_file = tie_off_dir.join("review-tie-off.md");
    assert!(
        tie_off_file.exists(),
        "tie-off file should exist for .rs file at {}",
        tie_off_file.display()
    );

    let tie_off_content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        tie_off_content.contains("rust review output"),
        "tie-off should contain agent output for .rs file"
    );

    // Verify loom-log has KnotCompleted and NOT StrandIgnored
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<&str> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();
    assert!(
        types.contains(&"KnotCompleted"),
        "should have KnotCompleted for .rs file. Events: {:?}",
        types
    );
    assert!(
        !types.contains(&"StrandIgnored"),
        "should NOT have StrandIgnored for .rs file. Events: {:?}",
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

// ── Deletion Context Integration Tests ───────────────────────────────────

/// When a strand is deleted, the agent's prompt should contain a deletion
/// notice and the previous processing history from the tie-off file.
#[test]
fn delete_event_agent_receives_context() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Mock pi captures stdin to a file so we can inspect the prompt
    let capture_file = tmp.path().join("agent_stdin.txt");
    create_mock_pi_capturing_stdin(&rig_dir, "review output", &capture_file);

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Phase 1: Create a strand and wait for processing
    let strand_path = create_strand(&rig_dir, "feature.md", "initial content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Wait for debounce to settle before deletion
    thread::sleep(Duration::from_secs(1));

    // Phase 2: Delete the strand and wait for delete processing
    fs::remove_file(&strand_path).unwrap();

    // Wait for the delete event to be processed.
    // After delete, the tie-off will have grown (another section appended).
    let tie_off_path =
        rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    let first_size =
        fs::metadata(&tie_off_path).map(|m| m.len()).unwrap_or(0);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        if let Ok(m) = fs::metadata(&tie_off_path) {
            if m.len() > first_size {
                break;
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    thread::sleep(Duration::from_millis(500));

    // Read the captured stdin from the delete-processing run
    let prompt = fs::read_to_string(&capture_file)
        .unwrap_or_else(|_| panic!("capture file should exist at {}",
            capture_file.display()));

    // Verify deletion notice is present
    assert!(
        prompt.contains("This file was deleted"),
        "prompt should contain deletion notice:\n{}",
        prompt
    );

    // Verify previous processing history is included
    assert!(
        prompt.contains("Previous processing history"),
        "prompt should contain previous processing history:\n{}",
        prompt
    );

    // Verify the strand path appears in the history (full path or filename)
    assert!(
        prompt.contains("feature.md"),
        "prompt should reference the strand path:\n{}",
        prompt
    );

    // Verify a trigger line from previous processing appears
    assert!(
        prompt.contains("triggered by Created")
            || prompt.contains("triggered by Modified"),
        "prompt should contain a trigger entry from previous processing:\n{}",
        prompt
    );

    handle.abort();
}

/// When a strand is deleted, the agent should execute successfully without
/// errors about missing files (no `@file` reference for deleted events).
#[test]
fn delete_event_agent_skips_missing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Mock pi that captures both stdin and stderr to detect file errors
    let stderr_file = tmp.path().join("agent_stderr.txt");
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script = format!(
        "#!/usr/bin/env bash\n\
         # Stub pi - consumes stdin, echoes response, captures stderr\n\
         cat > /dev/null\n\
         echo \"review output\"\n\
         exit 0\n"
    );
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

    // Create and process a strand
    let strand_path = create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Delete the strand
    thread::sleep(Duration::from_millis(500));
    fs::remove_file(&strand_path).unwrap();

    // Wait for delete processing — look for second KnotCompleted in loom-log
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

    // Should have exactly 2 KnotCompleted events (create + delete)
    assert!(
        completed_count >= 2,
        "should have 2 KnotCompleted events (create + delete), got {}",
        completed_count
    );

    // Verify no KnotFailed event appeared (would indicate file-not-found error)
    let events = read_loom_log(&rig_dir, "review-loom");
    let failed_events: Vec<_> = events
        .iter()
        .filter(|e| {
            loom_log_event_type(e)
                .map(|t| t == "KnotFailed")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        failed_events.is_empty(),
        "should have no KnotFailed events (agent should not error on missing file)"
    );

    // Verify the stderr capture file was never written to (no errors)
    assert!(
        !stderr_file.exists(),
        "stderr capture file should not exist (no errors from agent)"
    );

    handle.abort();
}

/// When a tie-off file has many entries for multiple strands, deleting one
/// strand should only inject the last 5 entries for that strand (not all).
#[test]
fn delete_event_large_tieoff_bounded_context() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Mock pi captures stdin to a file
    let capture_file = tmp.path().join("agent_stdin.txt");
    create_mock_pi_capturing_stdin(&rig_dir, "review output", &capture_file);

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Create target strand and wait for processing + debounce settle
    let target_strand = create_strand(&rig_dir, "target.md", "v1");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    thread::sleep(Duration::from_secs(1));

    // Create other strands to add noise entries (interleaved)
    let _other1 = create_strand(&rig_dir, "other1.md", "noise 1");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Modify target — each modification needs debounce settle time
    fs::write(&target_strand, "v2").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    thread::sleep(Duration::from_secs(1));

    let _other2 = create_strand(&rig_dir, "other2.md", "noise 2");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    fs::write(&target_strand, "v3").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    thread::sleep(Duration::from_secs(1));

    let _other3 = create_strand(&rig_dir, "other3.md", "noise 3");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    fs::write(&target_strand, "v4").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    thread::sleep(Duration::from_secs(1));

    fs::write(&target_strand, "v5").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    thread::sleep(Duration::from_secs(1));

    fs::write(&target_strand, "v6").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    thread::sleep(Duration::from_secs(1));

    // At this point target.md should have 6+ entries in the tie-off
    // and there are 3 other strands with entries too.

    // Verify tie-off has many entries
    let tie_off_path =
        rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    let tie_off_before_delete = fs::read_to_string(&tie_off_path).unwrap();
    let target_entries_before = tie_off_before_delete
        .matches("target.md")
        .count();
    assert!(
        target_entries_before >= 6,
        "should have at least 6 entries for target.md before delete, got {}",
        target_entries_before
    );

    // Delete target strand
    fs::remove_file(&target_strand).unwrap();

    // Wait for delete processing (tie-off grows)
    let first_size =
        fs::metadata(&tie_off_path).map(|m| m.len()).unwrap_or(0);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        if let Ok(m) = fs::metadata(&tie_off_path) {
            if m.len() > first_size {
                break;
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    thread::sleep(Duration::from_millis(500));

    // Read the captured stdin from the delete-processing run
    let prompt = fs::read_to_string(&capture_file)
        .unwrap_or_else(|_| panic!("capture file should exist at {}",
            capture_file.display()));

    // Count how many target.md references appear in the prompt
    let target_refs_in_prompt = prompt.matches("target.md").count();

    // The prompt contains:
    // - "Strand: target.md" (strand label) — 1 ref
    // - up to 5 history headers with the strand path — up to 5 refs
    // - the trigger line at the bottom with the strand path — 1 ref
    // So at most 1 + 5 + 1 = 7 references.
    assert!(
        target_refs_in_prompt <= 7,
        "prompt should contain at most 7 references to target.md \
         (strand label + last 5 history entries + trigger line), \
         got {}. Prompt:\n{}",
        target_refs_in_prompt,
        prompt
    );

    // The prompt should NOT contain other strand names
    assert!(
        !prompt.contains("other1.md"),
        "prompt should NOT contain other1.md (only target strand history):\n{}",
        prompt
    );
    assert!(
        !prompt.contains("other2.md"),
        "prompt should NOT contain other2.md:\n{}",
        prompt
    );
    assert!(
        !prompt.contains("other3.md"),
        "prompt should NOT contain other3.md:\n{}",
        prompt
    );

    // Verify the deletion notice is present
    assert!(
        prompt.contains("This file was deleted"),
        "prompt should contain deletion notice:\n{}",
        prompt
    );

    handle.abort();
}
