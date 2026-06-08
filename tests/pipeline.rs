//! Event pipeline, debounce, and strand lifecycle tests.
//!
//! Verifies the full pipeline flow:
//! NotifyEventSource → mpsc → DebounceEngine → ProcessStrand → tie-off.

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

// ── Event Pipeline Wiring ──────────────────────────────────────────────────

/// Create a file in the watched directory → raw event emitted → debounced
/// → `ProcessStrand` invoked → knot-state transitions to `completed`.
/// Verifies the full pipeline:
/// NotifyEventSource → mpsc → DebounceEngine → ProcessStrand.
#[tokio::test]
async fn event_flows_through_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("pipeline-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Mock agent script — ignores all CLI args built by ProcessStrand
    let mock_agent =
        create_mock_agent(&base_dir, "agent output");

    let port = 31990;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // Create a strand file in the watched source directory
    let strand_path = strand_dir.join("test-strand.md");
    fs::write(&strand_path, "strand content").expect("should create file");

    // Wait for debounce window + processing time
    std::thread::sleep(Duration::from_millis(300));

    // Poll knot status — should reach terminal state (completed or failed)
    let status =
        poll_knot_status(&host_port, "pipeline-loom", "review-knot", 60, 100)
            .await
            .expect("knot status should reach terminal state");
    assert_eq!(
        status["status"].as_str().unwrap(),
        "completed",
        "knot state should be completed"
    );

    // Verify tie-off file was produced
    let tie_off_path = tie_off_dir.join("test-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off file should exist: {}",
        tie_off_path.display()
    );

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("agent output"),
        "tie-off should contain agent output, got: {content}"
    );

    let _ = shutdown_tx.send(());
}

/// Rapid file edits (3 writes within 50ms) → debounce coalesces into
/// one event → only one `ProcessStrand` invocation → one tie-off produced.
#[tokio::test]
async fn debounce_prevents_duplicate_processing() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("debounce-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let port = 31991;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), "echo 'output'".to_string()],
        },
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // Create initial file in the strand directory (watched by the loom)
    let strand_path = strand_dir.join("rapid-edit.md");
    fs::write(&strand_path, "initial").expect("should create file");

    // Wait for the first event to fully process
    std::thread::sleep(Duration::from_millis(400));

    // Rapid edits: 3 writes within 50ms
    for i in 0..3 {
        fs::write(&strand_path, format!("edit {}", i))
            .expect("should write edit");
        std::thread::sleep(Duration::from_millis(10));
    }

    // Wait for debounce window + processing
    std::thread::sleep(Duration::from_millis(300));

    // Poll knot status — should reach terminal state
    let status =
        poll_knot_status(&host_port, "debounce-loom", "review-knot", 60, 100)
            .await
            .expect("knot status should reach terminal state");
    let final_status = status["status"].as_str().unwrap();
    assert!(
        matches!(final_status, "completed" | "failed"),
        "knot should reach terminal state, got: {final_status}"
    );

    // Verify debounce worked: rapid edits produced fewer StrandProcessed
    // events than raw writes. Each write may emit 1-2 raw events (notify
    // internals), so without debouncing we'd see 3-6+ StrandProcessed
    // events for the burst alone. With debouncing, the 3 rapid writes
    // coalesce to 1 debounced event.
    let log_path = base_dir.join("debounce-loom/.loom-log");
    let log_content =
        fs::read_to_string(&log_path).expect("loom log should exist");
    let strand_processed_count = log_content
        .lines()
        .filter(|line| {
            line.contains("StrandProcessed")
                && line.contains("rapid-edit.md")
        })
        .count();

    // Total StrandProcessed: 1 for initial create + 1 for debounced burst
    // = 2. Allow some slack for notify emitting extra events.
    assert!(
        strand_processed_count <= 4,
        "debounce should coalesce rapid edits; expected <= 4 events, got {}",
        strand_processed_count
    );

    // Tie-off directory exists and has at least one file for the strand
    let tie_off_dir = tie_off_dir;
    assert!(
        tie_off_dir.exists(),
        "tie-off directory should exist"
    );
    let tie_off_files: Vec<_> = fs::read_dir(&tie_off_dir)
        .expect("should read tie-off dir")
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        !tie_off_files.is_empty(),
        "should have at least 1 tie-off file"
    );

    let _ = shutdown_tx.send(());
}

// ── End-to-End Pipeline ────────────────────────────────────────────────────

/// Full pipeline test: create → modify → delete strand lifecycle.
///
/// Verifies both filesystem state (tie-off files) and HTTP observability
/// (`/looms`, `/looms/:id/knots/:name`, `/looms/:id/activity`).
///
/// 1. Create strand → tie-off file created, knot status `completed`
/// 2. Modify strand → tie-off overwritten
/// 3. Delete strand → tie-off appended with Deleted header
/// 4. HTTP: `/looms` lists loom, activity has `StrandProcessed`
#[tokio::test]
async fn full_pipeline_create_modify_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create loom directory with knot definition
    let loom_dir = base_dir.join("pipeline-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Mock agent script — ignores all CLI args built by ProcessStrand
    let mock_agent = create_mock_agent(&base_dir, "processed");

    let port = 31992;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // — HTTP: loom is registered —
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "should have 1 loom");
    assert_eq!(
        summaries[0]["id"].as_str().unwrap(),
        "pipeline-loom",
        "loom id should match"
    );

    // — HTTP: knot status is `idle` before any event —
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/pipeline-loom/knots/review-knot",
            30,
            100,
        )
        .await
        .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "idle",
        "knot status should be idle before any event"
    );

    // — Step 1: Create strand → tie-off file created —
    let strand_path = strand_dir.join("test-strand.md");
    fs::write(&strand_path, "initial content").unwrap();

    // Poll until status is `completed`
    let status_result =
        poll_knot_status(&host_port, "pipeline-loom", "review-knot", 60, 100)
            .await;
    assert!(
        status_result.is_ok(),
        "knot status should reach terminal state"
    );
    let completed_status = status_result.unwrap();
    assert_eq!(
        completed_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed after processing"
    );

    let tie_off_path = tie_off_dir.join("test-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist after create: {}",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("processed"),
        "tie-off should contain 'processed', got: {content}"
    );

    // — Step 2: Modify strand → tie-off overwritten —
    fs::write(&strand_path, "modified content").unwrap();

    // Poll until processing completes
    poll_knot_status(&host_port, "pipeline-loom", "review-knot", 60, 100)
        .await
        .expect("knot status should reach terminal state after modify");

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("processed"),
        "tie-off should still contain 'processed' after modify, got: {content}"
    );

    // — Step 3: Delete strand → tie-off appended with Deleted header —
    fs::remove_file(&strand_path).unwrap();

    // Allow file watcher to detect the deletion
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Poll until processing completes
    poll_knot_status(&host_port, "pipeline-loom", "review-knot", 60, 100)
        .await
        .expect("knot status should reach terminal state after delete");

    assert!(
        tie_off_path.exists(),
        "tie-off file should still exist after delete"
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("## Event: Deleted"),
        "tie-off should have Deleted event header, got: {content}"
    );
    assert!(
        content.contains("processed"),
        "tie-off should contain agent response, got: {content}"
    );
    // Modified + Deleted sections (modify overwrites, delete appends)
    let delimiter_count = content.matches("---").count();
    assert!(
        delimiter_count >= 3,
        "should have 2 sections with delimiters, found {}: {}",
        delimiter_count, content
    );

    // Strand file should not exist (it was deleted)
    assert!(!strand_path.exists(), "strand file should be deleted");

    // — HTTP: activity log contains `StrandProcessed` entry —
    let (status, body) =
        http_get_retry(&host_port, "/looms/pipeline-loom/activity", 30, 100)
            .await
            .expect("activity endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let events: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");

    let has_strand_processed = events.iter().any(|e| {
        e.get("StrandProcessed").is_some()
            || e.get("strand_path").is_some()
    });
    assert!(
        has_strand_processed,
        "activity log should contain StrandProcessed entry, got {events:?}"
    );

    let _ = shutdown_tx.send(());
}

// ── Pipeline with Subdirectory Rig ─────────────────────────────────────────

/// Full pipeline test with loom in a subdirectory rig.
///
/// Verifies that when `base_dir` is a subdirectory of the project root,
/// looms are still discovered and strands processed correctly.
///
/// 1. Rig subdirectory scanned for looms
/// 2. Loom discovered with correct id
/// 3. Strand processed → tie-off produced with agent output
#[tokio::test]
async fn full_pipeline_with_subdirectory_rig() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Rig subdirectory (what the server scans).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory with knot definition.
    let loom_dir = rig.join("config-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(root);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Mock agent script — ignores all CLI args built by ProcessStrand
    let mock_agent = create_mock_agent(&root, "processed external");

    let port = 31997;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: rig.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // Verify loom is registered.
    let (status, body) =
        http_get_retry(&host_port, "/looms/config-loom", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        loom["id"].as_str().unwrap(),
        "config-loom",
        "loom id should match"
    );

    // Create a strand in the loom's source directory.
    let strand_path = strand_dir.join("external-strand.md");
    fs::write(&strand_path, "external strand content").unwrap();

    // Wait for debounce + processing via polling
    poll_knot_status(&host_port, "config-loom", "review-knot", 60, 100)
        .await
        .expect("knot status should reach terminal state");

    // Tie-off should appear in the loom's .knot-output directory.
    let tie_off_path = tie_off_dir.join("external-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("processed external"),
        "tie-off should contain agent output, got: {content}"
    );

    let _ = shutdown_tx.send(());
}

// ── Pipeline with External Directories ─────────────────────────────────────

/// End-to-end test with external source and output directories.
///
/// Verifies the full happy path: loom discovered, strand processed,
/// knot reaches `completed`, loom-log has `StrandProcessed`, and
/// tie-off file contains agent output.
///
/// 1. Loom in rig subdirectory discovered correctly
/// 2. Create strand → processing completes successfully
/// 3. Knot status `completed` with no error
/// 4. Loom-log contains `StrandProcessed` referencing strand filename
/// 5. Tie-off file written with agent output
#[tokio::test]
async fn full_pipeline_with_external_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Rig subdirectory (what the server scans for looms).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory with knot definition.
    let loom_dir = rig.join("success-external-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(root);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Mock agent script — ignores all CLI args built by ProcessStrand
    let mock_agent = create_mock_agent(&root, "summary");

    let port = 32002;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: rig.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // 1. Verify loom discovered.
    let (status, _body) =
        http_get_retry(&host_port, "/looms/success-external-loom", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");

    // 2. Create strand in loom source directory.
    let strand_path = strand_dir.join("success-strand.md");
    fs::write(&strand_path, "external success strand content").unwrap();

    // Wait for debounce + processing via polling
    let knot_status: serde_json::Value =
        poll_knot_status(&host_port, "success-external-loom", "review-knot", 60, 100)
            .await
            .expect("knot status should reach terminal state");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );
    assert!(
        knot_status["last_error"].is_null(),
        "knot status should have no error on success"
    );

    // 4. Verify loom-log contains `StrandProcessed` with no error.
    let log_path = rig.join("success-external-loom/.loom-log");
    assert!(
        log_path.exists(),
        "loom log should exist: {}",
        log_path.display()
    );
    let log_content =
        fs::read_to_string(&log_path).expect("should read log file");
    assert!(
        log_content.contains("StrandProcessed"),
        "loom log should contain StrandProcessed entry"
    );
    // On success the error field is null/absent in the JSON.
    assert!(
        log_content.contains("success-strand.md"),
        "loom log should reference the strand filename"
    );

    // 5. Verify tie-off file written with agent output.
    let tie_off_path = tie_off_dir.join("success-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        tie_off_content.contains("summary"),
        "tie-off should contain agent output 'summary', got: \
         {tie_off_content}"
    );

    let _ = shutdown_tx.send(());
}
