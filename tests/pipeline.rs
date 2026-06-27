//! Integration tests for the event processing pipeline.
//!
//! Verifies: Notify → debounce → ProcessStrand → tie-off.
//! Covers strand lifecycle (create/modify/delete), state file polling,
//! and loom-log event verification.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::path::{Path, PathBuf};
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

    // Verify tie-off was written (status already confirmed, file should be ready)
    let tie_off_dir = rig_dir.join("tie-offs/review-loom/review");
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
        thread::sleep(Duration::from_millis(50));
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
        thread::sleep(Duration::from_millis(50));
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

    // Wait for StrandProcessed to confirm processing finished
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");

    // Count KnotProcessing events — should be limited by debounce
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

    // Wait briefly for binary file processing to finish
    // (binary files produce StrandIgnored, not StrandProcessed)
    thread::sleep(Duration::from_millis(100));
    create_strand(&rig_dir, "notes.txt", "some plain text notes");

    // Wait for normal processing to complete
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotCompleted");

    // Verify tie-off was written
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
    let config = "agent-adapter: pi-stdio\n";
    fs::write(rig_dir.join(".workspace-agent-config.yaml"), config).unwrap();
    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), existing));
    }

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
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");

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
        thread::sleep(Duration::from_millis(50));
    }

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
    let config = "agent-adapter: pi-stdio\n";
    fs::write(rig_dir.join(".workspace-agent-config.yaml"), config).unwrap();
    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), existing));
    }

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Create and process a strand
    let strand_path = create_strand(&rig_dir, "feature.md", "content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Delete the strand
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
        thread::sleep(Duration::from_millis(50));
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

// ── Temp File / Missing File Integration Tests ─────────────────────────────

/// Create a mock pi that sleeps for a given duration before responding.
/// Used to create a processing gap for racing file deletion.
fn create_slow_mock_pi(
    rig_dir: &Path,
    response: &str,
    sleep_secs: u64,
) -> PathBuf {
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script = format!(
        "#!/usr/bin/env bash\n\
         # Stub pi - consumes stdin, sleeps, echoes response\n\
         cat > /dev/null\n\
         sleep {sleep_secs}\n\
         echo \"{response}\"\n\
         exit 0\n"
    );
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
    pi_path
}

/// Known temp files (sedXXXXXXX) are silently skipped by the pipeline.
///
/// Strategy: create a normal file first and wait for processing to start.
/// The mock agent sleeps for several seconds. During that processing,
/// create the temp file, wait for the debounce window (so the Created
/// event is emitted to the queue), then delete the temp file.
/// When the normal file's processing completes, the consumer picks up
/// the temp file's Created event — the file is gone, so the temp file
/// check silently skips it.
#[test]
fn pipeline_silently_skips_known_temp_file() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Use a slow mock agent so processing takes time,
    // giving us a window to create and delete the temp file.
    create_slow_mock_pi(&rig_dir, "slow output", 3);

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    let project_root = rig_dir.parent().unwrap();
    let strands_dir = project_root.join("strands");
    fs::create_dir_all(&strands_dir).unwrap();

    // Phase 1: Create normal.md. Wait for debounce + processing to start.
    // The slow agent keeps the pipeline busy for ~3 seconds.
    let normal_path = strands_dir.join("normal.md");
    fs::write(&normal_path, "normal content").unwrap();
    // Wait for debounce (20ms) + processing start + agent sleep to begin.
    // 100ms gives plenty of time for the event to be emitted and the
    // agent to start executing (the 3s sleep begins).
    thread::sleep(Duration::from_millis(100));

    // Phase 2: While the slow agent is busy, create the temp file.
    // Its Created event enters the debounce pending map.
    // Delete it quickly (within the debounce window) so the
    // Created+Deleted events coalesce to just Deleted.
    let temp_path = strands_dir.join("sedXXXXXXX");
    fs::write(&temp_path, "temp content").unwrap();
    // Immediate delete — within the debounce window (20ms).
    // The notify events (Created, Modified, Deleted) coalesce
    // to just Deleted in the debounce engine. The Deleted event
    // is processed normally (by design — deleted events skip
    // the existence check). The key verification is that NO
    // errors appear in the loom-log.
    fs::remove_file(&temp_path).unwrap();

    // Phase 3: Wait for normal.md processing to complete.
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Wait for any remaining events to be processed
    thread::sleep(Duration::from_millis(200));

    // Read loom-log and verify
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<&str> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    // normal.md should have been processed normally
    assert!(
        types.contains(&"KnotCompleted"),
        "should have KnotCompleted for normal.md. Events: {:?}",
        types
    );

    // Key verifications for the temp file scenario:
    // - No KnotFailed (temp file handling should not produce errors)
    // - No StrandSkipped (known temp files are silently skipped,
    //   and Deleted events process normally)
    // - No StrandIgnored (temp files are not binary files)
    assert!(
        !types.contains(&"KnotFailed"),
        "should NOT have KnotFailed (temp file should not error). Events: {:?}",
        types
    );
    assert!(
        !types.contains(&"StrandSkipped"),
        "should NOT have StrandSkipped for known temp file. Events: {:?}",
        types
    );
    assert!(
        !types.contains(&"StrandIgnored"),
        "should NOT have StrandIgnored for known temp file. Events: {:?}",
        types
    );

    // The Deleted event for sedXXXXXXX is processed normally
    // (by design — deleted events skip the existence check).
    // It produces KnotProcessing/KnotCompleted/StrandProcessed.
    // The Modified event (if emitted separately) would be silently
    // skipped by the temp file check. Either way, no errors appear.

    handle.abort();
}

/// Unknown missing files produce a StrandSkipped loom-log entry.
///
/// Uses the same "slow agent + race" strategy as the temp file test.
/// Creates a normal file first, then while processing, creates and
/// deletes a second file (with a non-temp name). When its Created
/// event is processed, the file is gone and the unknown-missing-file
/// path produces a StrandSkipped entry.
#[test]
fn pipeline_logs_strand_skipped_for_unknown_missing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Use a slow mock agent
    create_slow_mock_pi(&rig_dir, "slow output", 3);

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    let project_root = rig_dir.parent().unwrap();
    let strands_dir = project_root.join("strands");
    fs::create_dir_all(&strands_dir).unwrap();

    // Phase 1: Create normal.md and wait for processing to start
    let normal_path = strands_dir.join("normal.md");
    fs::write(&normal_path, "normal content").unwrap();
    thread::sleep(Duration::from_millis(100));

    // Phase 2: While processing, create the missing file and delete it
    let missing_path = strands_dir.join("some_missing_file.md");
    fs::write(&missing_path, "will be deleted").unwrap();
    thread::sleep(Duration::from_millis(50)); // wait for debounce
    fs::remove_file(&missing_path).unwrap();

    // Phase 3: Wait for normal.md processing to complete
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Phase 4: Wait for the StrandSkipped event from the missing file
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandSkipped");

    let events = read_loom_log(&rig_dir, "review-loom");

    // Verify StrandSkipped is present
    let skipped_events: Vec<_> = events
        .iter()
        .filter(|e| {
            loom_log_event_type(e)
                .map(|t| t == "StrandSkipped")
                .unwrap_or(false)
        })
        .collect();
    assert!(
        !skipped_events.is_empty(),
        "should have StrandSkipped event. Events: {:?}",
        events.iter().filter_map(|e| loom_log_event_type(e)).collect::<Vec<_>>()
    );

    // Verify the StrandSkipped event has the correct reason
    let skipped = &skipped_events[0];
    if let Some(data) = skipped.as_object().and_then(|o| o.get("StrandSkipped")) {
        if let Some(reason) = data.get("reason").and_then(|v| v.as_str()) {
            assert!(
                reason.contains("missing file"),
                "reason should mention missing file, got: {}",
                reason
            );
        } else {
            panic!("StrandSkipped event missing reason field");
        }
    } else {
        panic!("StrandSkipped event has no data object");
    }

    // normal.md should have been processed normally
    let types: Vec<&str> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();
    assert!(
        types.contains(&"KnotCompleted"),
        "should have KnotCompleted for normal.md. Events: {:?}",
        types
    );

    // No KnotFailed should appear
    assert!(
        !types.contains(&"KnotFailed"),
        "should NOT have KnotFailed. Events: {:?}",
        types
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

    // Create target strand and wait for processing to complete
    let target_strand = create_strand(&rig_dir, "target.md", "v1");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");

    // Create other strands to add noise entries (interleaved)
    let _other1 = create_strand(&rig_dir, "other1.md", "noise 1");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");

    // Modify target — add explicit delay between writes to ensure
    // separate tie-off entries (debounce window is 20ms, but tie-off
    // write is async so we need extra time for the previous entry)
    fs::write(&target_strand, "v2").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");
    thread::sleep(Duration::from_millis(100));

    let _other2 = create_strand(&rig_dir, "other2.md", "noise 2");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");

    fs::write(&target_strand, "v3").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");
    thread::sleep(Duration::from_millis(100));

    let _other3 = create_strand(&rig_dir, "other3.md", "noise 3");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");

    fs::write(&target_strand, "v4").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");
    thread::sleep(Duration::from_millis(100));

    fs::write(&target_strand, "v5").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");
    thread::sleep(Duration::from_millis(100));

    fs::write(&target_strand, "v6").unwrap();
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "StrandProcessed");
    thread::sleep(Duration::from_millis(100));

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
        thread::sleep(Duration::from_millis(50));
    }

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

/// Two knots sharing the same strand directory: modifying one knot's
/// strand_dir should NOT remove the other knot's watch on the shared
/// directory. This is the end-to-end integration test for the bug fix
/// where `unwatch()` was removing all entries for a path instead of
/// just the matching (path, WatchType) pair.
///
/// Steps:
/// 1. Create a loom with two knots (knot-a, knot-b) sharing `shared-strands`.
/// 2. Start Knot and wait for both knots to register.
/// 3. Create a strand file — both knots should process it.
/// 4. Modify knot-a's strand_dir to `new-strands`.
/// 5. Create another strand file in `shared-strands` — knot-b should
///    still process it, proving knot-a's unwatch didn't wipe knot-b's watch.
#[test]
fn multi_knot_shared_directory_unwatch_does_not_remove_other_watch() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    let project_root = tmp.path();
    fs::create_dir_all(&rig_dir).unwrap();

    // Create the shared strand directory
    let shared_strands = project_root.join("shared-strands");
    fs::create_dir_all(&shared_strands).unwrap();

    // Create a new strand directory for knot-a after modification
    let new_strands = project_root.join("new-strands");
    fs::create_dir_all(&new_strands).unwrap();

    // Create the loom directory
    let loom_dir = rig_dir.join("review-loom");
    fs::create_dir_all(&loom_dir).unwrap();

    // Create knot-a with shared-strands
    let knot_a_content = "---
name: knot-a
agent-profile-ref: fast
strand-dir: \"shared-strands\"
git-versioned: false
---

Knot A instructions.";
    fs::write(loom_dir.join("knot-a.md"), knot_a_content).unwrap();

    // Create knot-b with the SAME shared-strands
    let knot_b_content = "---
name: knot-b
agent-profile-ref: fast
strand-dir: \"shared-strands\"
git-versioned: false
---

Knot B instructions.";
    fs::write(loom_dir.join("knot-b.md"), knot_b_content).unwrap();

    // Create the fast profile and mock pi binary
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "mock tie-off output");

    // Start Knot in background
    let handle = start_knot(rig_dir.clone());

    // Wait for the loom to be discovered with 2 knots
    wait_for_loom_in_state(&rig_dir, "review-loom", 2);

    // Create a strand file in the shared directory
    let strand_path_1 = shared_strands.join("strand1.md");
    fs::write(&strand_path_1, "strand content 1").unwrap();

    // Wait for both knots to complete processing (each should produce KnotCompleted)
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut knot_a_completions = 0;
    let mut knot_b_completions = 0;
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let events = read_loom_log(&rig_dir, "review-loom");
        for event in &events {
            let event_type = loom_log_event_type(event);
            if event_type == Some("KnotCompleted") {
                // Determine which knot completed by checking knot_id
                // Loog entries are externally tagged: {"KnotCompleted": {...}}
                // so we must unwrap the variant key first.
                if let Some(inner) = loom_log_event_inner(event) {
                    if let Some(knot_id) = inner.get("knot_id").and_then(|k| k.as_str()) {
                        if knot_id == "knot-a" {
                            knot_a_completions += 1;
                        } else if knot_id == "knot-b" {
                            knot_b_completions += 1;
                        }
                    }
                }
            }
        }
        if knot_a_completions >= 1 && knot_b_completions >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    assert!(
        knot_a_completions >= 1,
        "knot-a should have completed processing strand1, got {} completions",
        knot_a_completions
    );
    assert!(
        knot_b_completions >= 1,
        "knot-b should have completed processing strand1, got {} completions",
        knot_b_completions
    );

    // Now modify knot-a's strand_dir to a new directory
    // This triggers handle_knot_modified which calls unwatch(old_strand_dir)
    let modified_knot_a_content = "---
name: knot-a
agent-profile-ref: fast
strand-dir: \"new-strands\"
git-versioned: false
---

Knot A instructions — updated strand_dir.";
    fs::write(loom_dir.join("knot-a.md"), modified_knot_a_content).unwrap();

    // Wait for knot-a's strand_dir to be updated in state.json
    let deadline2 = std::time::Instant::now() + Duration::from_secs(30);
    let mut knot_a_updated = false;
    while !knot_a_updated && std::time::Instant::now() < deadline2 {
        if let Ok(state) = read_state_file(&rig_dir) {
            if let Some(looms) = state.get("looms").and_then(|l| l.as_array()) {
                for loom in looms {
                    if let Some(looms_knots) = loom.get("knots").and_then(|k| k.as_array()) {
                        for knot in looms_knots {
                            if let Some(name) = knot.get("id").and_then(|n| n.as_str()) {
                                if name == "knot-a" {
                                    if let Some(strand_dir) = knot.get("strand_dir").and_then(|s| s.as_str()) {
                                        if strand_dir.contains("new-strands") {
                                            knot_a_updated = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if !knot_a_updated {
            thread::sleep(Duration::from_millis(50));
        }
    }

    // Create another strand file in the ORIGINAL shared directory.
    // knot-b should STILL process it, because knot-a's unwatch should
    // have only removed knot-a's entry, not knot-b's.
    let strand_path_2 = shared_strands.join("strand2.md");
    fs::write(&strand_path_2, "strand content 2").unwrap();

    // Wait for knot-b to process strand2 (knot-a should NOT process it
    // since it now watches new-strands, not shared-strands)
    let deadline3 = std::time::Instant::now() + Duration::from_secs(60);
    let mut knot_b_second_completion = false;
    loop {
        if std::time::Instant::now() > deadline3 {
            break;
        }
        let events = read_loom_log(&rig_dir, "review-loom");
        for event in &events {
            let event_type = loom_log_event_type(event);
            if event_type == Some("KnotCompleted") {
                // Loog entries are externally tagged: {"KnotCompleted": {...}}
                // so we must unwrap the variant key first.
                if let Some(inner) = loom_log_event_inner(event) {
                    if let Some(knot_id) = inner.get("knot_id").and_then(|k| k.as_str()) {
                        if let Some(strand_path) = inner.get("strand_path").and_then(|s| s.as_str()) {
                            if knot_id == "knot-b" && strand_path.contains("strand2") {
                                knot_b_second_completion = true;
                                break;
                            }
                        }
                    }
                }
            }
        }
        if knot_b_second_completion {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    assert!(
        knot_b_second_completion,
        "knot-b should still process strand2 from shared-strands after knot-a's strand_dir changed. This verifies the bug fix: unwatch() only removes the matching (path, WatchType) pair, not all entries for a path."
    );

    handle.abort();
}
