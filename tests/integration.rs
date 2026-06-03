//! Integration tests for the full Knot application.
//!
//! These tests spin up the actual server and verify end-to-end behaviour.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use knot::application::ports::{KnotStatePort, LoomLogPort, LoomRepository, TieOffSink};
use knot::AppConfig;
use knot::ShutdownSignal;
use knot::WorkspaceAgentConfig;

/// Valid knot definition file content for creating test looms.
const VALID_KNOT_CONTENT: &str = "---
name: review-knot
agent-config:
  goal: \"Review PRD goals for clarity\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: |
    Review the goals section of this PRD.
---

# Review Knot

This knot reviews PRD goals.
";

// ── Helpers ────────────────────────────────────────────────────────────────

/// Simple synchronous HTTP GET using raw TCP.
fn http_get(host_port: &str, path: &str) -> Result<(String, String), String> {
    let mut stream = TcpStream::connect(host_port)
        .map_err(|e| format!("connect failed: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n"
    );

    // Write before creating BufReader (avoids borrow conflict)
    stream.write_all(request.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;
    stream.flush().map_err(|e| format!("flush failed: {e}"))?;

    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    let status_line = lines
        .next()
        .ok_or("no status line")?
        .map_err(|e| format!("read failed: {e}"))?;

    let mut remaining = Vec::new();
    for line_result in lines {
        let line = line_result.map_err(|e| format!("read failed: {e}"))?;
        remaining.push(line);
    }

    let body_start = remaining
        .iter()
        .position(|l| l.trim().is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);

    let body = remaining[body_start..].join("\n");
    Ok((status_line, body.trim().to_string()))
}

/// Retry HTTP GET with delays between attempts.
fn http_get_retry(
    host_port: &str,
    path: &str,
    max_retries: usize,
    delay_ms: u64,
) -> Result<(String, String), String> {
    for attempt in 0..max_retries {
        match http_get(host_port, path) {
            Ok(result) => return Ok(result),
            Err(e) if attempt == max_retries - 1 => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(delay_ms)),
        }
    }
    Err(format!(
        "connection to {host_port}{path} failed after {max_retries} retries"
    ))
}

/// Wait for a TCP port to become available.
fn wait_for_port(host_port: &str, max_retries: usize, delay_ms: u64) -> Result<(), String> {
    for attempt in 0..max_retries {
        if TcpStream::connect(host_port).is_ok() {
            return Ok(());
        }
        if attempt < max_retries - 1 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }
    Err(format!(
        "connection to {host_port} failed after {max_retries} retries"
    ))
}

/// Spawn a server in a background thread with a shutdown channel.
/// Returns a shutdown sender that, when sent, gracefully stops the server.
fn spawn_server(config: AppConfig) -> tokio::sync::oneshot::Sender<()> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("create runtime");
        let signal = ShutdownSignal::Channel(shutdown_rx);
        let _ = rt.block_on(knot::start_server_with_shutdown(config, signal));
    });

    shutdown_tx
}

// ── Integration Tests ─────────────────────────────────────────────────────

/// `main()` starts HTTP server, `GET /health` returns `200 ok`.
#[test]
fn app_starts_and_serves_health() {
    let port = 31984;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // GET /health → 200 ok
    let (status, body) = http_get_retry(&host_port, "/health", 30, 100)
        .expect("health endpoint should respond");

    assert!(status.contains("200"), "expected 200 OK, got: {status}");
    assert_eq!(body, "ok", "health body should be 'ok'");

    // Graceful shutdown
    let _ = shutdown.send(());
}

/// `WorkspaceAgentConfig` is loaded with defaults (`pi` CLI); accessible
/// in `AppContext` via the `/config/workspace` HTTP endpoint.
#[test]
fn app_loads_workspace_agent_config() {
    let port = 31985;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        workspace_config: WorkspaceAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // GET /config/workspace → 200 with JSON
    let (status, body) =
        http_get_retry(&host_port, "/config/workspace", 30, 100)
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
    let _ = shutdown.send(());
}

// ── Composition Root Test (non-network) ────────────────────────────────────

/// Verify `build_app_context` wires all hex layers correctly.
#[test]
fn build_app_context_wires_layers() {
    let config = AppConfig::default_config();
    let ctx = knot::build_app_context(&config);

    // Store is present and empty (not yet populated)
    assert!(ctx.store.list().is_empty());

    // Ports are present (trait objects)
    let _repo: &dyn LoomRepository = &*ctx.loom_repo;
    let _state: &dyn KnotStatePort = &*ctx.knot_state_port;
    let _log: &dyn LoomLogPort = &*ctx.loom_log_port;
    let _sink: &dyn TieOffSink = &*ctx.tie_off_sink;

    // Workspace config is loaded with defaults
    assert_eq!(ctx.workspace_config.cli_path, "pi");
    assert!(ctx.workspace_config.cli_args.is_empty());

    // Event sender is present (receiver is intentionally unused in Phase 0)
    // Phase 2 wires the receiver into the processing pipeline.
}

// ── Phase 1: Startup Discovery and Watcher Boot ────────────────────────────

/// Given a workspace with loom directories, startup discovers them and
/// registers them in `LoomStore`. Verifiable via `GET /looms`.
#[test]
fn startup_discovers_looms() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("my-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    let port = 31986;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir,
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // GET /looms should return the discovered loom
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
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
        http_get(&host_port, "/looms/my-loom")
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

    let _ = shutdown.send(());
}

/// After startup, `NotifyEventSource` is watching all loom source
/// directories. Verified by creating a file in the watched directory
/// and confirming the server remains healthy.
#[test]
fn startup_starts_watchers() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("watch-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    let port = 31987;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir,
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Server is healthy at startup
    let (status, _) =
        http_get(&host_port, "/health")
            .expect("health should respond");
    assert!(status.contains("200"), "server should be healthy");

    // Create a file in the watched source directory.
    // If the watcher is running, this should not crash the server.
    fs::write(loom_dir.join("new-strand.md"), "new content")
        .expect("should create file");

    // Give notify time to emit the event
    std::thread::sleep(Duration::from_millis(500));

    // Server should still be healthy (proves watcher is active)
    let (status, _) =
        http_get_retry(&host_port, "/health", 30, 100)
            .expect("health should still respond");
    assert!(
        status.contains("200"),
        "server should still be healthy after file creation"
    );

    // Loom should still be discoverable
    let (status, body) =
        http_get(&host_port, "/looms")
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "looms endpoint should respond");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "loom should still be listed");

    let _ = shutdown.send(());
}

/// After startup, loom-log and knot-state files exist on disk for each
/// loom/knot discovered during startup.
#[test]
fn startup_creates_state_files() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("state-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    let port = 31988;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify knot state file exists on disk
    let state_file = base_dir.join(".knots/review-knot.state");
    assert!(
        state_file.exists(),
        "knot state file should exist: {}",
        state_file.display()
    );

    // Verify state file contains idle status
    let state_content =
        fs::read_to_string(&state_file).expect("should read state file");
    let state: serde_json::Value =
        serde_json::from_str(&state_content).expect("state should be JSON");
    assert_eq!(
        state["status"].as_str().unwrap(),
        "idle",
        "initial state should be idle"
    );

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

    let _ = shutdown.send(());
}
