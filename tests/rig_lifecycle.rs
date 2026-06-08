//! Rig directory lifecycle and server bootstrap tests.
//!
//! Verifies that Knot correctly creates the rig directory on startup,
//! scans existing rig directories for looms, serves health and config
//! endpoints, and persists loom registration across server restarts.

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

// ── Rig Directory Lifecycle ────────────────────────────────────────────────

/// Start Knot in empty dir; `./rig/` created automatically.
///
/// 1. Start Knot in a temp directory with no `./rig/` subdirectory
/// 2. Verify health endpoint responds
/// 3. Verify `./rig/` directory was created
#[tokio::test]
async fn rig_directory_auto_created() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_path = tmp.path().join("rig");

    let port = 31980;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: rig_path.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Verify health endpoint responds
    let (status, body) = http_get_retry(&host_port, "/health", 30, 100)
        .await
        .expect("health endpoint should respond");
    assert!(status.contains("200"), "expected 200 OK, got: {status}");
    assert_eq!(body, "ok");

    // Verify ./rig/ directory was created
    assert!(
        rig_path.exists(),
        "rig directory should have been auto-created at {}",
        rig_path.display()
    );
    assert!(
        rig_path.is_dir(),
        "rig path should be a directory"
    );

    let _ = shutdown_tx.send(());
}

/// Start Knot in dir with `./rig/` containing loom subdirectories;
/// looms discovered and registered.
///
/// 1. Create a temp dir with a `./rig/` subdirectory containing a loom
/// 2. Start Knot with base_dir pointing to the rig
/// 3. Verify looms are discovered via `GET /looms`
#[tokio::test]
async fn rig_directory_scanned() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_path = tmp.path().join("rig");

    // Create rig directory with a loom subdirectory
    fs::create_dir(&rig_path).unwrap();
    let loom_dir = rig_path.join("docs-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(tmp.path());
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let port = 31981;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: rig_path.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Verify rig directory exists (already existed, but verify)
    assert!(rig_path.exists(), "rig directory should exist");

    // GET /looms should return the discovered loom
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
        "docs-loom",
        "loom id should match"
    );

    // Verify rig config endpoint returns rig path
    let (status, body) =
        http_get_retry(&host_port, "/config/rig", 30, 100)
            .await
            .expect("config endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let config_json: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert!(
        config_json["rig_path"].is_string(),
        "config should have rig_path field"
    );
    assert!(
        config_json["rig_path"].as_str().unwrap().contains("rig"),
        "rig_path should contain 'rig'"
    );

    let _ = shutdown_tx.send(());
}

// ── Server Bootstrap ──────────────────────────────────────────────────────

/// `main()` starts HTTP server, `GET /health` returns `200 ok`.
#[tokio::test]
async fn app_starts_and_serves_health() {
    let port = 31984;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // GET /health → 200 ok
    let (status, body) = http_get_retry(&host_port, "/health", 30, 100)
        .await
        .expect("health endpoint should respond");

    assert!(status.contains("200"), "expected 200 OK, got: {status}");
    assert_eq!(body, "ok", "health body should be 'ok'");

    // Graceful shutdown
    let _ = shutdown_tx.send(());
}

/// `RigAgentConfig` is loaded with defaults (`pi` CLI); accessible
/// in `AppContext` via the `/config/rig` HTTP endpoint.
#[tokio::test]
async fn app_loads_rig_agent_config() {
    let port = 31985;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // GET /config/rig → 200 with JSON
    let (status, body) =
        http_get_retry(&host_port, "/config/rig", 30, 100)
            .await
            .expect("config endpoint should respond");

    assert!(status.contains("200"), "expected 200 OK, got: {status}");

    // Parse JSON and verify defaults
    let config: serde_json::Value =
        serde_json::from_str(&body).expect("response should be valid JSON");

    assert_eq!(config["cli_path"], "pi", "default cli_path should be 'pi'");
    assert!(config["cli_args"].is_array(), "cli_args should be an array");
    assert_eq!(
        config["cli_args"].as_array().map(|a| a.len()),
        Some(0),
        "default cli_args should be empty"
    );

    // Graceful shutdown
    let _ = shutdown_tx.send(());
}

/// Register a loom via API, stop server, restart — loom re-discovered
/// with same configuration (knot files survive restart).
///
/// 1. Start server with empty rig
/// 2. POST /looms with a loom that has a knot
/// 3. Verify loom directory and knot file created on disk
/// 4. Shutdown server
/// 5. Restart server with same rig directory
/// 6. Verify loom is re-discovered via GET /looms with matching config
#[tokio::test]
async fn api_register_then_discover_after_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_path = tmp.path().join("rig");
    let strand_dir = tmp.path().join("strands");
    let tie_off_dir = tmp.path().join("tie-offs");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();

    let port = 32011;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: rig_path.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    // --- First server instance ---
    let (_handle1, shutdown1) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server 1 should start listening");

    // POST /looms to register a new loom
    let post_body = serde_json::json!({
        "id": "persist-loom",
        "knots": [{
            "name": "persist-knot",
            "agent_config": {
                "goal": "Persist test",
                "provider": "openai",
                "model": "gpt-4o"
            },
            "prompt_template": {
                "input_bundling": "full-file",
                "instructions": "Review this content."
            },
            "strand_dir": strand_dir.to_string_lossy().to_string(),
            "tie_off_dir": tie_off_dir.to_string_lossy().to_string()
        }]
    });

    let (status, _body) =
        http_post_json(&host_port, "/looms", &post_body)
            .await
            .expect("POST /looms should respond");
    assert!(
        status.contains("201"),
        "expected 201 Created, got: {status}"
    );

    // Verify loom directory and knot file created on disk
    let loom_dir = rig_path.join("persist-loom");
    assert!(
        loom_dir.exists(),
        "loom directory should be created on disk: {}",
        loom_dir.display()
    );
    let knot_file = loom_dir.join("persist-knot.md");
    assert!(
        knot_file.exists(),
        "knot file should be created on disk: {}",
        knot_file.display()
    );

    // Shutdown first server and wait for port release
    let _ = shutdown1.send(());
    tokio::time::timeout(Duration::from_secs(10), _handle1)
        .await
        .expect("server 1 should complete shutdown within timeout");

    // --- Second server instance (restart) ---
    let config2 = AppConfig {
        base_dir: rig_path.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let (_handle2, shutdown2) = spawn_server_with_shutdown(config2);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server 2 should start listening");

    // GET /looms should re-discover the loom
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");

    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "should re-discover exactly 1 loom");
    assert_eq!(
        summaries[0]["id"].as_str().unwrap(),
        "persist-loom",
        "loom id should match after restart"
    );

    // Verify knot configuration matches
    let (status, body) =
        http_get_retry(&host_port, "/looms/persist-loom", 30, 100)
            .await
            .expect("get loom endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");

    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(knots.len(), 1, "loom should have 1 knot after restart");
    assert_eq!(
        knots[0]["id"].as_str().unwrap(),
        "persist-knot",
        "knot id should match after restart"
    );

    let _ = shutdown2.send(());
}
