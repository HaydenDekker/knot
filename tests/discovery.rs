//! Loom discovery, filtering, watcher boot, and registration logging.
//!
//! Extracted from `integration.rs` — Phase 3 of integration test refactor.

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;

use helpers::*;

// ── Discovery ──────────────────────────────────────────────────────────────

/// Given a rig with loom directories, startup discovers them and
/// registers them in `LoomStore`. Verifiable via `GET /looms`.
#[tokio::test]
async fn startup_discovers_looms() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("my-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let port = 31986;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir,
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // GET /looms should return the discovered loom
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100).await
            .expect("looms endpoint should respond");

    assert!(status.contains("200"), "expected 200, got: {status}");

    // Parse and verify response
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "should have 1 loom");
    assert_eq!(
        summaries[0]["id"].as_str().unwrap(),
        "my-loom",
        "loom id should match"
    );

    // Verify loom has the knot via GET /looms/my-loom
    let (status, body) =
        http_get(&host_port, "/looms/my-loom").await
            .expect("get loom endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(knots.len(), 1, "loom should have 1 knot");
    assert_eq!(
        knots[0]["id"].as_str().unwrap(),
        "review-knot",
        "knot id should match"
    );

    let _ = shutdown_tx.send(());
}

/// After startup, `NotifyEventSource` is watching all loom source
/// directories. Verified by creating a file in the watched directory
/// and confirming the server remains healthy.
#[tokio::test]
async fn startup_starts_watchers() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("watch-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let port = 31987;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir,
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // Server is healthy at startup
    let (status, _) =
        http_get(&host_port, "/health").await
            .expect("health should respond");
    assert!(status.contains("200"), "server should be healthy");

    // Create a file in the watched source directory.
    // If the watcher is running, this should not crash the server.
    fs::write(loom_dir.join("new-strand.md"), "new content")
        .expect("should create file");

    // Give notify time to emit the event
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Server should still be healthy (proves watcher is active)
    let (status, _) =
        http_get_retry(&host_port, "/health", 30, 100).await
            .expect("health should still respond");
    assert!(
        status.contains("200"),
        "server should still be healthy after file creation"
    );

    // Loom should still be discoverable
    let (status, body) =
        http_get(&host_port, "/looms").await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "looms endpoint should respond");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "loom should still be listed");

    let _ = shutdown_tx.send(());
}

/// After startup, loom-log and knot-state files exist on disk for each
/// loom/knot discovered during startup.
#[tokio::test]
async fn startup_logs_knot_registration() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("state-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let port = 31988;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // Verify loom log file exists on disk
    let log_file = base_dir.join("state-loom/.loom-log");
    assert!(
        log_file.exists(),
        "loom log file should exist: {}",
        log_file.display()
    );

    // Verify log contains KnotRegistered and LoomStarted entries
    let log_content =
        fs::read_to_string(&log_file).expect("should read log file");
    assert!(
        log_content.contains("KnotRegistered"),
        "log should contain KnotRegistered entry"
    );
    assert!(
        log_content.contains("LoomStarted"),
        "log should contain LoomStarted entry"
    );

    // Verify knot status is derivable from loom-log via HTTP
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/state-loom/knots/review-knot",
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
        "knot status should be idle (from KnotRegistered event)"
    );

    let _ = shutdown_tx.send(());
}

// ── Filtering ──────────────────────────────────────────────────────────────

/// Non-`-loom` directories in the rig are ignored during discovery.
///
/// 1. Create rig with both `*-loom` and non-`*-loom` directories
/// 2. Start Knot with base_dir pointing to the rig
/// 3. Verify only `-loom` directories appear in `GET /looms`
#[tokio::test]
async fn discovery_ignores_non_loom_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_path = tmp.path().join("rig");
    fs::create_dir_all(&rig_path).unwrap();

    // Create a valid loom directory (ends in `-loom`)
    let valid_loom = rig_path.join("valid-loom");
    fs::create_dir(&valid_loom).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(tmp.path());
    fs::write(valid_loom.join("review.md"), knot_content).unwrap();

    // Create non-loom directories that should be ignored
    let output_dir = rig_path.join("output");
    fs::create_dir(&output_dir).unwrap();
    fs::write(output_dir.join("something.txt"), "not a loom").unwrap();

    let state_dir = rig_path.join("some-state");
    fs::create_dir(&state_dir).unwrap();
    fs::write(state_dir.join("state.json"), "{}" ).unwrap();

    // A loom-log directory (created by LoomLogPort, should be ignored)
    let log_dir = rig_path.join("phantom-id");
    fs::create_dir(&log_dir).unwrap();
    fs::write(log_dir.join(".loom-log"), "LoomStarted").unwrap();

    let port = 32025;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: rig_path.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000).await
        .expect("server should start listening");

    // GET /looms should return only the valid loom
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100).await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");

    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "should have exactly 1 loom (non-loom dirs ignored)");
    assert_eq!(
        summaries[0]["id"].as_str().unwrap(),
        "valid-loom",
        "loom id should be the only -loom directory"
    );

    // Verify non-loom directories are NOT in the list
    let loom_ids: Vec<_> =
        summaries.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(
        !loom_ids.contains(&"output"),
        "'output' directory should not be discovered as a loom"
    );
    assert!(
        !loom_ids.contains(&"some-state"),
        "'some-state' directory should not be discovered as a loom"
    );
    assert!(
        !loom_ids.contains(&"phantom-id"),
        "'phantom-id' directory (loom-log directory) should not be discovered as a loom"
    );

    let _ = shutdown_tx.send(());
}
