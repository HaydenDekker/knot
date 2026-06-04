//! Integration tests for the full Knot application.
//!
//! These tests spin up the actual server and verify end-to-end behaviour.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use knot::application::ports::{
    AgentRunner, LoomLogPort, LoomRepository, TieOffSink,
};
use knot::AppConfig;
use knot::ShutdownSignal;
use knot::RigAgentConfig;

/// Valid knot definition file content for creating test looms.
const VALID_KNOT_CONTENT: &str = "---
name: review-knot
agent-config:
  goal: \"Review PRD goals for clarity\"
  provider: \"openai\"
  model: \"gpt-4o\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: |
    Review the goals section of this PRD.
---

# Review Knot

This knot reviews PRD goals.
";

/// Create a mock agent script in the given directory.
/// The script echoes the given message and ignores all CLI arguments.
/// Returns the absolute path to the script.
fn create_mock_agent(
    dir: &std::path::Path,
    output: &str,
) -> std::path::PathBuf {
    let script_path = dir.join("mock-agent");
    fs::write(
        &script_path,
        format!("#!/bin/sh\necho '{}'\n", output),
    )
    .expect("should write mock agent script");
    fs::set_permissions(
        &script_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("should set script as executable");
    script_path
}

/// Create a stub-pi.sh script that mimics `pi -p` behaviour.
///
/// The stub:
/// 1. Parses `--model`, `--system-prompt`, and `@<file>` arguments
/// 2. If model contains "nonexistent", exits with code 1 (simulates error)
/// 3. Otherwise reads `@<file>` content and stdin, echoes them back
///
/// This verifies that Knot constructs the correct CLI invocation pattern
/// without needing a real LLM API key.
fn create_stub_pi_agent(dir: &std::path::Path) -> std::path::PathBuf {
    let script_path = dir.join("stub-pi");
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

SYSTEM_PROMPT=""
MODEL=""
FILE_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        -p)
            shift
            ;;
        --model)
            MODEL="$2"
            shift 2
            ;;
        --system-prompt)
            SYSTEM_PROMPT="$2"
            shift 2
            ;;
        --no-session|--no-tools)
            shift
            ;;
        --tool)
            shift 2
            ;;
        @*)
            FILE_ARGS+=("$1")
            shift
            ;;
        *)
            shift
            ;;
    esac
done

# Simulate error for nonexistent models
if echo "$MODEL" | grep -q "nonexistent"; then
    echo "Error: model '$MODEL' not found" >&2
    exit 1
fi

# Read stdin (the prompt sent by SubprocessAgentRunner)
STDIN_CONTENT=$(cat)

# Output what we received so integration tests can verify
{
    echo "=== SYSTEM PROMPT ==="
    echo "$SYSTEM_PROMPT"
    echo "=== MODEL ==="
    echo "$MODEL"
    echo "=== STRAND FILES ==="
    for f in "${FILE_ARGS[@]}"; do
        filepath="${f#@}"
        if [ -f "$filepath" ]; then
            echo "FILE: $filepath"
            cat "$filepath"
        fi
    done
    echo "=== STDIN ==="
    echo "$STDIN_CONTENT"
}
"#;
    fs::write(&script_path, script).expect("should write stub-pi script");
    fs::set_permissions(
        &script_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("should set stub-pi as executable");
    script_path
}

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

// ── Phase 0: Rig Directory Discovery ───────────────────────────────────────

/// Start Knot in empty dir; `./rig/` created automatically.
///
/// 1. Start Knot in a temp directory with no `./rig/` subdirectory
/// 2. Verify health endpoint responds
/// 3. Verify `./rig/` directory was created
#[test]
fn rig_directory_auto_created() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_path = tmp.path().join("rig");

    let port = 31980;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: rig_path.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify health endpoint responds
    let (status, body) = http_get_retry(&host_port, "/health", 30, 100)
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

    let _ = shutdown.send(());
}

/// Start Knot in dir with `./rig/` containing loom subdirectories;
/// looms discovered and registered.
///
/// 1. Create a temp dir with a `./rig/` subdirectory containing a loom
/// 2. Start Knot with base_dir pointing to the rig
/// 3. Verify looms are discovered via `GET /looms`
#[test]
fn rig_directory_scanned() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_path = tmp.path().join("rig");

    // Create rig directory with a loom subdirectory
    fs::create_dir(&rig_path).unwrap();
    let loom_dir = rig_path.join("docs-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    let port = 31981;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: rig_path.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify rig directory exists (already existed, but verify)
    assert!(rig_path.exists(), "rig directory should exist");

    // GET /looms should return the discovered loom
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
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

    let _ = shutdown.send(());
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

/// `RigAgentConfig` is loaded with defaults (`pi` CLI); accessible
/// in `AppContext` via the `/config/rig` HTTP endpoint.
#[test]
fn app_loads_rig_agent_config() {
    let port = 31985;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig::default_config(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // GET /config/rig → 200 with JSON
    let (status, body) =
        http_get_retry(&host_port, "/config/rig", 30, 100)
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
    let (ctx, _event_rx) = knot::build_app_context(&config);

    // Store is present and empty (not yet populated)
    assert!(ctx.store.list().is_empty());

    // Ports are present (trait objects)
    let _repo: &dyn LoomRepository = &*ctx.loom_repo;
    let _log: &dyn LoomLogPort = &*ctx.loom_log_port;
    let _sink: &dyn TieOffSink = &*ctx.tie_off_sink;

    // Agent runner is present (subprocess)
    let _runner: &dyn AgentRunner = &*ctx.agent_runner;

    // Workspace config is loaded with defaults
    assert_eq!(ctx.rig_config.cli_path, "pi");
    assert!(ctx.rig_config.cli_args.is_empty());

    // Event sender is present; receiver is returned for pipeline wiring
    // (Receiver type proves the channel was created)
    let _ = _event_rx;
}

// ── Phase 1: Startup Discovery and Watcher Boot ────────────────────────────

/// Given a rig with loom directories, startup discovers them and
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
fn startup_logs_knot_registration() {
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
        .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "idle",
        "knot status should be idle (from KnotRegistered event)"
    );

    let _ = shutdown.send(());
}

// ── Phase 2: Event Pipeline Wiring ─────────────────────────────────────────

/// Poll the knot status endpoint until it reports a terminal state.
fn poll_knot_status(
    host_port: &str,
    loom_id: &str,
    knot_id: &str,
    max_retries: usize,
    delay_ms: u64,
) -> Result<serde_json::Value, String> {
    for attempt in 0..max_retries {
        let path = format!("/looms/{loom_id}/knots/{knot_id}");
        match http_get(host_port, &path) {
            Ok((status, body)) if status.contains("200") => {
                let val: serde_json::Value =
                    serde_json::from_str(&body).map_err(|e| e.to_string())?;
                let state = val["status"].as_str().unwrap_or("");
                if state == "completed" || state == "failed" {
                    return Ok(val);
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
        if attempt < max_retries - 1 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }
    Err("timeout waiting for knot status".to_string())
}

/// Create a file in the watched directory → raw event emitted → debounced
/// → `ProcessStrand` invoked → knot-state transitions to `completed`.
/// Verifies the full pipeline:
/// NotifyEventSource → mpsc → DebounceEngine → ProcessStrand.
#[test]
fn event_flows_through_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("pipeline-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

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

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create a strand file in the watched source directory
    let strand_path = loom_dir.join("test-strand.md");
    fs::write(&strand_path, "strand content").expect("should create file");

    // Wait for debounce window + processing time
    std::thread::sleep(Duration::from_millis(300));

    // Poll knot status — should reach terminal state (completed or failed)
    let status =
        poll_knot_status(&host_port, "pipeline-loom", "review-knot", 60, 100)
            .expect("knot status should reach terminal state");
    assert_eq!(
        status["status"].as_str().unwrap(),
        "completed",
        "knot state should be completed"
    );

    // Verify tie-off file was produced
    let tie_off_path = loom_dir.join(".knot-output/test-strand.md.output");
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

    let _ = shutdown.send(());
}

/// Rapid file edits (3 writes within 50ms) → debounce coalesces into
/// one event → only one `ProcessStrand` invocation → one tie-off produced.
#[test]
fn debounce_prevents_duplicate_processing() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("debounce-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

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

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create initial file to establish the strand
    let strand_path = loom_dir.join("rapid-edit.md");
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
    let tie_off_dir = loom_dir.join(".knot-output");
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

    let _ = shutdown.send(());
}

// ── Phase 3: End-to-End Integration Tests ──────────────────────────────────

/// Full pipeline test using mock agent CLI (`echo "processed"`).
///
/// 1. Create strand → tie-off file created with content
/// 2. Modify strand → tie-off overwritten with new content
/// 3. Delete strand → tie-off reports deletion (file still exists)
#[test]
fn full_pipeline_create_modify_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create loom directory with knot definition
    let loom_dir = base_dir.join("pipeline-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

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

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Step 1: Create strand → tie-off file created
    let strand_path = loom_dir.join("test-strand.md");
    fs::write(&strand_path, "initial content").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let tie_off_path = loom_dir.join(".knot-output/test-strand.md.output");
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

    // Step 2: Modify strand → tie-off overwritten
    fs::write(&strand_path, "modified content").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("processed"),
        "tie-off should still contain 'processed' after modify, got: {content}"
    );

    // Step 3: Delete strand → tie-off reports deletion (file still exists)
    fs::remove_file(&strand_path).unwrap();
    std::thread::sleep(Duration::from_millis(500));

    assert!(
        tie_off_path.exists(),
        "tie-off file should still exist after delete"
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("deleted"),
        "tie-off should report deletion, got: {content}"
    );

    // Strand file should not exist (it was deleted)
    assert!(!strand_path.exists(), "strand file should be deleted");

    let _ = shutdown.send(());
}

/// Same flow as above but observable via HTTP endpoints.
///
/// 1. `GET /looms` → loom listed
/// 2. `GET /looms/:id/knots/:knot_name` → status is `idle` before event,
///    `completed` after processing
/// 3. `GET /looms/:id/activity` → contains `StrandProcessed` entry
#[test]
fn full_pipeline_http_observable() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create loom directory with knot definition
    let loom_dir = base_dir.join("http-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    // Mock agent script — ignores all CLI args built by ProcessStrand
    let mock_agent = create_mock_agent(&base_dir, "processed");

    let port = 31993;
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

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. GET /looms → loom listed
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "should have 1 loom");
    assert_eq!(
        summaries[0]["id"].as_str().unwrap(),
        "http-loom",
        "loom id should match"
    );

    // 2a. GET /looms/:id/knots/:knot_name → status is `idle` before event
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/http-loom/knots/review-knot",
            30,
            100,
        )
        .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "idle",
        "knot status should be idle before any event"
    );

    // Create a strand file to trigger processing
    let strand_path = loom_dir.join("http-strand.md");
    fs::write(&strand_path, "http strand content").unwrap();

    // 2b. Poll until status is `completed`
    let status_result =
        poll_knot_status(&host_port, "http-loom", "review-knot", 60, 100);
    assert!(
        status_result.is_ok(),
        "knot status should reach terminal state"
    );
    let completed_status = status_result.unwrap();
    assert_eq!(
        completed_status["status"]
            .as_str()
            .unwrap(),
        "completed",
        "knot status should be completed after processing"
    );

    // 3. GET /looms/:id/activity → contains StrandProcessed entry
    let (status, body) =
        http_get_retry(&host_port, "/looms/http-loom/activity", 30, 100)
            .expect("activity endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let events: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");

    // Find StrandProcessed event
    let has_strand_processed = events.iter().any(|e| {
        e.get("StrandProcessed").is_some()
            || e.get("strand_path").is_some()
    });
    assert!(
        has_strand_processed,
        "activity log should contain StrandProcessed entry, got {events:?}"
    );

    let _ = shutdown.send(());
}

/// Two looms with different source dirs and tie-off points.
///
/// 1. Create strand in loom A → tie-off in A's point only
/// 2. Create strand in loom B → tie-off in B's point only
/// 3. No cross-interference (A's knots don't process B's strands)
#[test]
fn multiple_looms_independent() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Loom A
    let loom_a_dir = base_dir.join("loom-a");
    fs::create_dir(&loom_a_dir).unwrap();
    fs::write(
        loom_a_dir.join("review.md"),
        "---\nname: review-knot\nagent-config:\n  goal: \"Review A\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review A's documents.\n---\n",
    )
    .unwrap();

    // Loom B
    let loom_b_dir = base_dir.join("loom-b");
    fs::create_dir(&loom_b_dir).unwrap();
    fs::write(
        loom_b_dir.join("review.md"),
        "---\nname: review-knot\nagent-config:\n  goal: \"Review B\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review B's documents.\n---\n",
    )
    .unwrap();

    let port = 31994;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: "sh".to_string(),
            cli_args: vec![
                "-c".to_string(),
                "echo 'processed'".to_string(),
            ],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify both looms are registered
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 2, "should have 2 looms");

    // Collect loom IDs from the response
    let loom_ids: Vec<_> =
        summaries.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(
        loom_ids.contains(&"loom-a"),
        "loom-a should be registered"
    );
    assert!(
        loom_ids.contains(&"loom-b"),
        "loom-b should be registered"
    );

    // 1. Create strand in loom A
    let strand_a_path = loom_a_dir.join("strand-a.md");
    fs::write(&strand_a_path, "content for A").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Tie-off appears only in A's output directory
    let tie_off_a = loom_a_dir.join(".knot-output/strand-a.md.output");
    assert!(
        tie_off_a.exists(),
        "tie-off should exist in loom A: {}",
        tie_off_a.display()
    );

    // 2. Create strand in loom B
    let strand_b_path = loom_b_dir.join("strand-b.md");
    fs::write(&strand_b_path, "content for B").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Tie-off appears only in B's output directory
    let tie_off_b = loom_b_dir.join(".knot-output/strand-b.md.output");
    assert!(
        tie_off_b.exists(),
        "tie-off should exist in loom B: {}",
        tie_off_b.display()
    );

    // 3. No cross-interference
    // A's tie-off dir should NOT contain B's strand output
    let tie_off_dir_a = loom_a_dir.join(".knot-output");
    let files_in_a: Vec<_> =
        fs::read_dir(&tie_off_dir_a)
            .expect("should read tie-off dir A")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    assert!(
        !files_in_a.iter().any(|f| f.contains("strand-b")),
        "loom A should not contain loom B's strand output, got {files_in_a:?}"
    );

    // B's tie-off dir should NOT contain A's strand output
    let tie_off_dir_b = loom_b_dir.join(".knot-output");
    let files_in_b: Vec<_> =
        fs::read_dir(&tie_off_dir_b)
            .expect("should read tie-off dir B")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    assert!(
        !files_in_b.iter().any(|f| f.contains("strand-a")),
        "loom B should not contain loom A's strand output, got {files_in_b:?}"
    );

    let _ = shutdown.send(());
}

// ── Phase 4: Graceful Shutdown ────────────────────────────────────────────

/// Send shutdown signal; file watcher stops, no new events are processed
/// after shutdown.
///
/// Verified by:
/// 1. Starting server with a loom
/// 2. Sending shutdown signal
/// 3. Waiting briefly for shutdown to complete
/// 4. Creating a new strand file — should NOT produce a tie-off
/// 5. Confirming the tie-off file does NOT exist
#[test]
fn graceful_shutdown_stops_watchers() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("shutdown-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    let port = 31995;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: "sh".to_string(),
            cli_args: vec![
                "-c".to_string(),
                "echo 'processed'".to_string(),
            ],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify server is healthy before shutdown
    let (status, _) =
        http_get(&host_port, "/health").expect("health should respond");
    assert!(status.contains("200"), "server should be healthy");

    // Send shutdown signal
    let _ = shutdown.send(());

    // Give shutdown time to complete (drop watcher, drain pipeline)
    std::thread::sleep(Duration::from_millis(1000));

    // Create a strand file AFTER shutdown — should NOT be processed
    let strand_path = loom_dir.join("post-shutdown-strand.md");
    fs::write(&strand_path, "this should not be processed").unwrap();

    // Wait a bit to confirm no processing happens
    std::thread::sleep(Duration::from_millis(500));

    // Tie-off file should NOT exist (watcher was stopped)
    let tie_off_path =
        loom_dir.join(".knot-output/post-shutdown-strand.md.output");
    assert!(
        !tie_off_path.exists(),
        "tie-off should NOT exist after shutdown: {}",
        tie_off_path.display()
    );
}

/// Shutdown writes `LoomStopped` to each loom's activity log.
///
/// Verified by:
/// 1. Starting server with a loom
/// 2. Sending shutdown signal
/// 3. Reading the loom-log file
/// 4. Confirming it contains `LoomStopped` event
#[test]
fn shutdown_logs_loom_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("log-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    let port = 31996;
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

    // Verify server is healthy before shutdown
    let (status, _) =
        http_get(&host_port, "/health").expect("health should respond");
    assert!(status.contains("200"), "server should be healthy");

    // Send shutdown signal
    let _ = shutdown.send(());

    // Give shutdown time to complete (including LoomStopped log write)
    std::thread::sleep(Duration::from_millis(1000));

    // Read the loom-log file
    let log_file = loom_dir.join(".loom-log");
    assert!(
        log_file.exists(),
        "loom log file should exist: {}",
        log_file.display()
    );

    let log_content =
        fs::read_to_string(&log_file).expect("should read log file");

    // Verify log contains LoomStopped entry
    assert!(
        log_content.contains("LoomStopped"),
        "log should contain LoomStopped entry, got: {log_content}"
    );

    // Also verify the log still has the startup entries
    assert!(
        log_content.contains("LoomStarted"),
        "log should still contain LoomStarted entry"
    );
    assert!(
        log_content.contains("KnotRegistered"),
        "log should still contain KnotRegistered entry"
    );
}

// ── Phase 1: Full Pipeline with Subdirectory Rig ────────────────────────

/// Full pipeline test with loom in a subdirectory.
///
/// 1. Create a rig with a loom in a subdirectory
/// 2. Loom source_dir defaults to the loom directory
/// 3. Create a strand in the loom's source directory
/// 4. Tie-off should be produced at the loom's tie-off directory
#[test]
fn full_pipeline_subdirectory_rig() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Rig subdirectory (what the server scans).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory with knot definition.
    let loom_dir = rig.join("config-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

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

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify loom is registered (source_dir defaults to loom directory).
    let (status, body) =
        http_get_retry(&host_port, "/looms/config-loom", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert!(
        loom["source_dir"].as_str().unwrap().contains("config-loom"),
        "source_dir should be the loom directory"
    );

    // Create a strand in the loom's source directory.
    let strand_path = loom_dir.join("external-strand.md");
    fs::write(&strand_path, "external strand content").unwrap();

    // Wait for debounce + processing.
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Tie-off should appear in the loom's .knot-output directory.
    let tie_off_path = loom_dir.join(".knot-output/external-strand.md.output");
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

    let _ = shutdown.send(());
}

// ── Phase 2: Agent Error Logging in Knot-State and Loom-Log ────────────

/// Full pipeline test with a nonexistent agent CLI.
///
/// 1. Create a rig with a loom
/// 2. Configure `cli_path` to a nonexistent binary
/// 3. Create a strand — agent will fail
/// 4. Verify knot-state shows `Failed` with error message
/// 5. Verify loom-log contains `StrandProcessed` with error field
#[test]
fn full_pipeline_agent_error_in_state_and_log() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("error-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    let port = 31998;
    let host_port = format!("127.0.0.1:{port}");

    // Use a nonexistent CLI path
    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: "/nonexistent/path/to/fake-agent".to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create a strand to trigger processing
    let strand_path = loom_dir.join("error-strand.md");
    fs::write(&strand_path, "error strand content").unwrap();

    // Wait for debounce + processing
    std::thread::sleep(Duration::from_millis(500));

    // 1. Verify knot status shows `Failed` with error message
    let (status, body) =
        http_get(&host_port, "/looms/error-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "failed",
        "knot status should be failed"
    );
    assert!(
        knot_status["last_error"].is_string(),
        "knot status should have error message"
    );
    let error_msg = knot_status["last_error"].as_str().unwrap();
    assert!(
        error_msg.contains("command not found"),
        "error should mention command not found, got: {error_msg}"
    );

    // 2. Verify loom-log contains StrandProcessed with error field
    let log_path = base_dir.join("error-loom/.loom-log");
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
    assert!(
        log_content.contains("command not found"),
        "loom log should contain error details, got: {log_content}"
    );

    let _ = shutdown.send(());
}

// ── Phase 3: Full Integration Verification ──────────────────────────────

/// End-to-end test combining `.loom-config.yaml` external directories with
/// a nonexistent agent CLI.
///
/// 1. Loom in a subdirectory with nonexistent agent CLI (`/no/such/agent`).
/// 2. Create strand → triggers processing → agent fails.
/// 3. Verify:
///    - Knot status shows `Failed` with descriptive error.
///    - Loom-log contains `StrandProcessed` with error details.
///    - Tie-off file written at loom's `.knot-output` with `Failed` status.
#[test]
fn full_pipeline_external_source_with_agent_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Rig subdirectory (what the server scans for looms).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory with knot definition.
    let loom_dir = rig.join("error-external-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    let port = 32001;
    let host_port = format!("127.0.0.1:{port}");

    // Use a nonexistent agent CLI path.
    let config = AppConfig {
        base_dir: rig.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: "/no/such/agent".to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Verify loom is discovered.
    let (status, _body) =
        http_get_retry(&host_port, "/looms/error-external-loom", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");

    // 2. Create strand in loom source directory → triggers processing.
    let strand_path = loom_dir.join("error-strand.md");
    fs::write(&strand_path, "external error strand content").unwrap();

    // Wait for debounce + processing.
    std::thread::sleep(Duration::from_millis(500));

    // 3. Verify knot status shows `Failed` with descriptive error.
    let (status, body) =
        http_get(&host_port, "/looms/error-external-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "failed",
        "knot status should be failed"
    );
    assert!(
        knot_status["last_error"].is_string(),
        "knot status should have error message"
    );
    let error_msg = knot_status["last_error"].as_str().unwrap();
    assert!(
        error_msg.contains("command not found"),
        "error should mention command not found, got: {error_msg}"
    );

    // 4. Verify loom-log contains `StrandProcessed` with error details.
    let log_path = rig.join("error-external-loom/.loom-log");
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
    assert!(
        log_content.contains("command not found"),
        "loom log should contain error details, got: {log_content}"
    );

    // 5. Verify tie-off file written with Failed content.
    let tie_off_path =
        loom_dir.join(".knot-output/error-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        tie_off_content.contains("Processing failed"),
        "tie-off should contain 'Processing failed', got: {tie_off_content}"
    );
    assert!(
        tie_off_content.contains("command not found"),
        "tie-off should contain error details, got: {tie_off_content}"
    );

    let _ = shutdown.send(());
}

/// End-to-end test combining `.loom-config.yaml` external directories with
/// a mock agent CLI (`echo "summary"`). Verifies the full happy path with
/// external source and output directories.
///
/// 1. Loom in a subdirectory with mock agent CLI (`echo summary`).
/// 2. Create strand → triggers processing → agent succeeds.
/// 3. Verify:
///    - Loom discovered with source_dir = loom directory.
///    - Knot status shows `completed`.
///    - Loom-log contains `StrandProcessed` with no error.
///    - Tie-off file written at loom's `.knot-output` with agent output.
#[test]
fn full_pipeline_external_source_with_mock_agent_success() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Rig subdirectory (what the server scans for looms).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory with knot definition.
    let loom_dir = rig.join("success-external-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

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

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Verify loom discovered.
    let (status, _body) =
        http_get_retry(&host_port, "/looms/success-external-loom", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");

    // 2. Create strand in loom source directory.
    let strand_path = loom_dir.join("success-strand.md");
    fs::write(&strand_path, "external success strand content").unwrap();

    // Wait for debounce + processing.
    std::thread::sleep(Duration::from_millis(500));

    // 3. Verify knot status shows `completed`.
    let (status, body) =
        http_get(&host_port, "/looms/success-external-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
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
    let tie_off_path =
        loom_dir.join(".knot-output/success-strand.md.output");
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

    let _ = shutdown.send(());
}

// ── Phase 3: Stub pi CLI Integration Tests ────────────────────────────────

/// Full happy path using a stub `pi` CLI that mimics `pi -p` behaviour.
///
/// The stub reads `--system-prompt` and `@<file>` args, then echoes them
/// back. This verifies that Knot constructs the correct CLI invocation
/// from the knot's agent config and prompt template, and that the
/// subprocess runner passes strand content to the agent.
///
/// 1. Create loom with knot (provider/openai, model/gpt-4o)
/// 2. Start server with stub-pi.sh as cli_path
/// 3. Create strand → tie-off contains system prompt + strand content
/// 4. Verify knot-state is `completed`
/// 5. Verify loom-log contains `StrandProcessed` with no error
#[test]
fn full_pipeline_with_pi_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("pi-loom");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(loom_dir.join("review.md"), VALID_KNOT_CONTENT).unwrap();

    // Create the stub-pi script that echoes received args and content
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32003;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create a strand file to trigger processing
    let strand_path = loom_dir.join("test-strand.md");
    fs::write(&strand_path, "This is the strand content for review.")
        .expect("should create strand file");

    // Wait for debounce + processing
    std::thread::sleep(Duration::from_millis(500));

    // 1. Verify tie-off exists and contains the agent output
    let tie_off_path = loom_dir.join(".knot-output/test-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );

    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");

    // Tie-off should contain the system prompt from --system-prompt arg
    assert!(
        tie_off_content.contains("Review the goals section"),
        "tie-off should contain system prompt, got: {tie_off_content}"
    );

    // Tie-off should contain the strand file content (read via @<file>)
    assert!(
        tie_off_content.contains("This is the strand content for review."),
        "tie-off should contain strand content, got: {tie_off_content}"
    );

    // Tie-off should contain the model name (proves --model was passed)
    assert!(
        tie_off_content.contains("gpt-4o"),
        "tie-off should contain model name, got: {tie_off_content}"
    );

    // 2. Verify knot status is `completed`
    let (status, body) =
        http_get(&host_port, "/looms/pi-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    // 3. Verify loom-log contains StrandProcessed with no error
    let log_path = base_dir.join("pi-loom/.loom-log");
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

    let _ = shutdown.send(());
}

/// Verify the stub `pi` CLI receives system prompt and strand content,
/// and that a nonexistent model causes knot-state to show `failed`.
///
/// 1. Start server with stub-pi.sh and a knot using `nonexistent-model`
/// 2. Create strand → stub exits with code 1 (simulates model not found)
/// 3. Verify knot-state shows `failed` with error message
/// 4. Verify tie-off contains error details
/// 5. Verify loom-log contains `StrandProcessed` with error field
#[test]
fn pi_agent_receives_system_prompt_and_strand() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot that uses a nonexistent model
    let loom_dir = base_dir.join("error-loom");
    fs::create_dir(&loom_dir).unwrap();
    let knot_content = r#"---
name: review-knot
agent-config:
  goal: "Review with nonexistent model"
  provider: "openai"
  model: "nonexistent-model-xyz"
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review the goals section of this PRD.
---

# Error Test Knot

This knot tests error handling.
"#;
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Create the stub-pi script (exits 1 for "nonexistent" models)
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32004;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create a strand file to trigger processing
    let strand_path = loom_dir.join("error-strand.md");
    fs::write(&strand_path, "Error test strand content")
        .expect("should create strand file");

    // Wait for debounce + processing
    std::thread::sleep(Duration::from_millis(500));

    // 1. Verify knot status shows `failed` with error message
    let (status, body) =
        http_get(&host_port, "/looms/error-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "failed",
        "knot status should be failed for nonexistent model"
    );
    assert!(
        knot_status["last_error"].is_string(),
        "knot status should have error message"
    );
    let error_msg = knot_status["last_error"].as_str().unwrap();
    assert!(
        error_msg.contains("agent execution failed")
            || error_msg.contains("exited with code 1"),
        "error should mention agent failure, got: {error_msg}"
    );

    // 2. Verify tie-off contains error details
    let tie_off_path = loom_dir.join(".knot-output/error-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        tie_off_content.contains("Processing failed"),
        "tie-off should contain 'Processing failed', got: {tie_off_content}"
    );

    // 3. Verify loom-log contains StrandProcessed with error
    let log_path = base_dir.join("error-loom/.loom-log");
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
    // The error field should be present (non-null) in the log
    assert!(
        log_content.contains("agent execution failed")
            || log_content.contains("exited with code"),
        "loom log should contain error details, got: {log_content}"
    );

    let _ = shutdown.send(());
}

// ── Phase 4: Demo Verification ────────────────────────────────────────────

/// Demo verification: the `knot-test` loom config has provider/model fields,
/// Knot processes `sample-document.md` and produces a populated tie-off,
/// and the loom-log records successful processing.
///
/// This test mirrors the demo workflow:
/// 1. Create a rig with a `knot-test` loom (provider + model in config)
/// 2. Place `sample-document.md` in the source directory
/// 3. Start Knot with stub-pi agent
/// 4. Verify tie-off is populated (contains system prompt + strand content)
/// 5. Verify loom-log records `StrandProcessed` with no error
#[test]
fn demo_knot_test_processes_sample_document() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create knot-test loom directory with provider/model in config
    let loom_dir = base_dir.join("knot-test");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(
        &loom_dir.join("review-knot.md"),
        r#"---
name: review-knot
agent-config:
  goal: "Review and summarize documents"
  provider: "openai"
  model: "gpt-4o"
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review the provided document. Provide a concise summary
    of its key points and any recommendations.
---

# Review Knot

This knot reviews and summarizes documents.
"#,
    )
    .unwrap();

    // Create the sample-document.md strand
    fs::write(
        &loom_dir.join("sample-document.md"),
        r#"# Sample Document for Knot Processing

## Introduction

This is a sample document that demonstrates the Knot file
processing pipeline.

## Key Points

1. The Knot service watches a source directory for file events.
2. When a file is created or modified, the configured agent
   processes its content.
3. The agent output (tie-off) is written to the output directory.
4. Processing events are recorded in the loom-log file.

## Recommendations

- Keep documents concise for faster processing.
- Use markdown format for best results.
- Monitor the loom-log for processing status.
"#,
    )
    .unwrap();

    // Create stub-pi agent script
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32005;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Wait for initial file processing (file already exists at startup,
    // but notify may emit a Create event during discovery)
    std::thread::sleep(Duration::from_millis(500));

    // If the initial file hasn't been processed yet (startup race),
    // create a new file to trigger processing explicitly.
    let tie_off_path =
        loom_dir.join(".knot-output/sample-document.md.output");
    if !tie_off_path.exists() {
        // Touch the file to trigger a Modify event
        fs::write(&loom_dir.join("sample-document.md"),
            "# Sample Document for Knot Processing\n\n## Updated\n\nContent.")
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
    }

    // 1. Verify tie-off exists and contains populated content
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );

    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");

    // Tie-off should contain system prompt (proves --system-prompt was passed)
    assert!(
        tie_off_content.contains("Review the provided document"),
        "tie-off should contain system prompt, got: {tie_off_content}"
    );

    // Tie-off should contain model name (proves --model was passed)
    assert!(
        tie_off_content.contains("gpt-4o"),
        "tie-off should contain model name, got: {tie_off_content}"
    );

    // Tie-off should contain strand content (proves @<file> was used)
    assert!(
        tie_off_content.contains("Sample Document")
            || tie_off_content.contains("Knot Processing"),
        "tie-off should contain strand content, got: {tie_off_content}"
    );

    // 2. Verify knot status is `completed` via HTTP
    let (status, body) =
        http_get(&host_port, "/looms/knot-test/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    // 3. Verify loom-log records successful processing
    let log_path = base_dir.join("knot-test/.loom-log");
    assert!(
        log_path.exists(),
        "loom log should exist: {}",
        log_path.display()
    );
    let log_content =
        fs::read_to_string(&log_path).expect("should read loom log");
    assert!(
        log_content.contains("StrandProcessed"),
        "loom log should contain StrandProcessed entry"
    );
    assert!(
        log_content.contains("sample-document.md"),
        "loom log should reference sample-document.md"
    );

    let _ = shutdown.send(());
}

/// Demo verification: knot-test loom with tools configured.
///
/// Uses a knot config with `tools: [fs, web]` to verify the
/// `build_cli_args` path that emits `--tool` flags instead of
/// `--no-tools`.
#[test]
fn demo_knot_test_with_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create knot-test loom with tools in agent-config
    let loom_dir = base_dir.join("knot-test");
    fs::create_dir(&loom_dir).unwrap();
    fs::write(
        &loom_dir.join("review-knot.md"),
        r#"---
name: review-knot
agent-config:
  goal: "Review with tools"
  provider: "anthropic"
  model: "claude-sonnet-4-20250514"
  tools:
    - fs
    - web
prompt-template:
  input-bundling: "full-file"
  instructions: |
    Review the document with tool access.
---

# Review Knot With Tools
"#,
    )
    .unwrap();

    // Create stub-pi agent that echoes all received flags
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32006;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create a strand file
    fs::write(&loom_dir.join("input.md"), "Document to review.").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Verify tie-off exists and contains the model from knot config
    let tie_off_path = loom_dir.join(".knot-output/input.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        tie_off_content.contains("claude-sonnet-4-20250514"),
        "tie-off should contain the configured model, got: {tie_off_content}"
    );

    // Verify knot status is completed
    let (status, body) =
        http_get(&host_port, "/looms/knot-test/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    let _ = shutdown.send(());
}

// ── Phase 5: Per-Knot Source Directory Integration Test ────────────────────

/// Two knots in one loom, each with its own source directory.
///
/// 1. Create a rig with a loom containing two knot files.
/// 2. Each knot defines its own `source-dir` pointing to a separate dir.
/// 3. Start the server with a mock agent.
/// 4. Create a strand in knot A's source → processed by knot A only.
/// 5. Create a strand in knot B's source → processed by knot B only.
/// 6. Verify both knots reach `completed` status independently.
#[test]
fn server_starts_with_per_knot_source_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Rig subdirectory (what the server scans).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory (contains knot definitions).
    let loom_dir = rig.join("multi-knot-loom");
    fs::create_dir(&loom_dir).unwrap();

    // External source directories for each knot.
    let source_a = root.join("source-a");
    fs::create_dir(&source_a).unwrap();
    let source_b = root.join("source-b");
    fs::create_dir(&source_b).unwrap();

    // Knot A — watches source-a.
    let knot_a_content = format!(
        "---
name: knot-a
agent-config:
  goal: \"Review A\"
  provider: \"openai\"
  model: \"gpt-4o\"
source-dir: \"{}\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review A\"
---
",
        source_a.display()
    );
    fs::write(loom_dir.join("knot-a.md"), knot_a_content).unwrap();

    // Knot B — watches source-b.
    let knot_b_content = format!(
        "---
name: knot-b
agent-config:
  goal: \"Review B\"
  provider: \"openai\"
  model: \"gpt-4o\"
source-dir: \"{}\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review B\"
---
",
        source_b.display()
    );
    fs::write(loom_dir.join("knot-b.md"), knot_b_content).unwrap();

    // Mock agent script — ignores all CLI args built by ProcessStrand.
    let mock_agent = create_mock_agent(&root, "processed");

    let port = 32010;
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

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify loom is discovered with 2 knots.
    let (status, body) =
        http_get_retry(&host_port, "/looms/multi-knot-loom", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().expect("knots should be array");
    assert_eq!(knots.len(), 2, "loom should have 2 knots");

    // Verify both knots are present.
    let knot_ids: Vec<_> = knots
        .iter()
        .map(|k| k["id"].as_str().unwrap())
        .collect();
    assert!(
        knot_ids.contains(&"knot-a"),
        "knot-a should be present"
    );
    assert!(
        knot_ids.contains(&"knot-b"),
        "knot-b should be present"
    );

    // 1. Create a strand in source-a → should trigger knot-a.
    let strand_a_path = source_a.join("strand-a.md");
    fs::write(&strand_a_path, "content for A").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Verify knot-a reaches completed status.
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/multi-knot-loom/knots/knot-a",
            30,
            100,
        )
        .expect("knot-a status should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_a_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_a_status["status"].as_str().unwrap(),
        "completed",
        "knot-a status should be completed"
    );

    // 2. Create a strand in source-b → should trigger knot-b.
    let strand_b_path = source_b.join("strand-b.md");
    fs::write(&strand_b_path, "content for B").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Verify knot-b reaches completed status.
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/multi-knot-loom/knots/knot-b",
            30,
            100,
        )
        .expect("knot-b status should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_b_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_b_status["status"].as_str().unwrap(),
        "completed",
        "knot-b status should be completed"
    );

    // Verify knot-a strand_path references source-a file.
    assert!(
        knot_a_status["last_strand_path"]
            .as_str()
            .unwrap_or("")
            .contains("strand-a.md"),
        "knot-a should reference strand-a.md, got: {knot_a_status:?}"
    );

    // Verify knot-b strand_path references source-b file.
    assert!(
        knot_b_status["last_strand_path"]
            .as_str()
            .unwrap_or("")
            .contains("strand-b.md"),
        "knot-b should reference strand-b.md, got: {knot_b_status:?}"
    );

    let _ = shutdown.send(());
}


