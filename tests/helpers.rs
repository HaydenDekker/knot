//! Shared test infrastructure for integration tests.
//!
//! Provides helper functions for creating test fixtures, spawning servers,
//! making HTTP requests, and polling for expected states.

use std::fs;
use std::path::Path;

use knot::AppConfig;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration};

// ── Knot Fixtures ──────────────────────────────────────────────────────────

/// Generate valid knot definition file content with absolute paths.
/// Creates the strand and tie-off directories if they don't exist.
/// Returns (knot_content, strand_dir, tie_off_dir).
pub fn make_knot_content_with_dirs(
    project_root: &Path,
) -> (String, std::path::PathBuf, std::path::PathBuf) {
    let strand_dir = project_root.join("strands");
    let tie_off_dir = project_root.join("tie-offs");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();
    let content = format!(
        "---\nname: review-knot\nagent-config:\n  goal: \"Review PRD goals for clarity\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nstrand-dir: \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review the goals section of this PRD.\n---\n\n# Review Knot\n\nThis knot reviews PRD goals.\n",
        strand_dir.display(),
        tie_off_dir.display()
    );
    (content, strand_dir, tie_off_dir)
}

/// Generate valid knot definition file content (ignores returned paths).
pub fn make_knot_content(project_root: &Path) -> String {
    let (content, _, _) = make_knot_content_with_dirs(project_root);
    content
}

/// Create a mock agent script in the given directory.
/// The script echoes the given message and ignores all CLI arguments.
/// Returns the absolute path to the script.
pub fn create_mock_agent(
    dir: &Path,
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
pub fn create_stub_pi_agent(dir: &Path) -> std::path::PathBuf {
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
        --tools)
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

// ── HTTP Helpers ───────────────────────────────────────────────────────────

/// Read the full response from a TCP stream, returning (status_line, body).
async fn read_response(mut stream: TcpStream) -> Result<(String, String), String> {
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("read failed: {e}"))?;

    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();

    let status_line = lines
        .next()
        .ok_or("no status line")?
        .to_string();

    // Find body after first empty line
    let mut body_iter = lines.peekable();
    let mut found_blank = false;
    let mut body_lines = Vec::new();
    for line in &mut body_iter {
        if !found_blank {
            if line.trim().is_empty() {
                found_blank = true;
            }
        } else {
            body_lines.push(line.to_string());
        }
    }

    let body = body_lines.join("\n");
    Ok((status_line, body.trim().to_string()))
}

/// Simple async HTTP GET using raw TCP.
pub async fn http_get(host_port: &str, path: &str) -> Result<(String, String), String> {
    let mut stream = TcpStream::connect(host_port)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    read_response(stream).await
}

/// Retry HTTP GET with delays between attempts.
pub async fn http_get_retry(
    host_port: &str,
    path: &str,
    max_retries: usize,
    delay_ms: u64,
) -> Result<(String, String), String> {
    for attempt in 0..max_retries {
        match http_get(host_port, path).await {
            Ok(result) => return Ok(result),
            Err(e) if attempt == max_retries - 1 => return Err(e),
            Err(_) => sleep(Duration::from_millis(delay_ms)).await,
        }
    }
    Err(format!(
        "connection to {host_port}{path} failed after {max_retries} retries"
    ))
}

/// Simple async HTTP POST with JSON body using raw TCP.
pub async fn http_post_json(
    host_port: &str,
    path: &str,
    body: &serde_json::Value,
) -> Result<(String, String), String> {
    let body_str = body.to_string();
    let mut stream = TcpStream::connect(host_port)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: \
         application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{body_str}",
        body_str.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    read_response(stream).await
}

/// Retry HTTP POST with JSON body.
pub async fn http_post_json_retry(
    host_port: &str,
    path: &str,
    body: &serde_json::Value,
    max_retries: u32,
    delay_ms: u64,
) -> Result<(String, String), String> {
    for attempt in 0..max_retries {
        match http_post_json(host_port, path, body).await {
            Ok(result) => return Ok(result),
            Err(e) if attempt == max_retries - 1 => return Err(e),
            Err(_) => sleep(Duration::from_millis(delay_ms)).await,
        }
    }
    Err(format!(
        "connection to {host_port}{path} failed after {max_retries} retries"
    ))
}

/// Simple async HTTP PATCH with JSON body using raw TCP.
pub async fn http_patch_json(
    host_port: &str,
    path: &str,
    body: &serde_json::Value,
) -> Result<(String, String), String> {
    let body_str = body.to_string();
    let mut stream = TcpStream::connect(host_port)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let request = format!(
        "PATCH {path} HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: \
         application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{body_str}",
        body_str.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    read_response(stream).await
}

/// Simple async HTTP DELETE using raw TCP.
pub async fn http_delete(host_port: &str, path: &str) -> Result<(String, String), String> {
    let mut stream = TcpStream::connect(host_port)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let request = format!(
        "DELETE {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: \
         close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    read_response(stream).await
}

// ── Server Helpers ─────────────────────────────────────────────────────────

/// Wait for a TCP port to become available (async).
pub async fn wait_for_port(host_port: &str, timeout_ms: u64) -> Result<(), String> {
    let deadline = Duration::from_millis(timeout_ms);
    let start = tokio::time::Instant::now();

    loop {
        if TcpStream::connect(host_port).await.is_ok() {
            return Ok(());
        }
        if start.elapsed() >= deadline {
            return Err(format!(
                "connection to {host_port} timed out after {timeout_ms}ms"
            ));
        }
        sleep(Duration::from_millis(50)).await;
    }
}

/// Spawn a server in a background tokio task.
/// The task is dropped (and the server stopped) when the JoinHandle is dropped.
pub fn spawn_server(config: AppConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let _ = knot::start_server(config).await;
    })
}

/// Spawn a server with a controllable shutdown signal.
///
/// Returns a `oneshot::Sender<()>` — sending on it triggers the server's
/// graceful shutdown sequence (axum stops, pipeline drains, LoomStopped
/// written). The background `JoinHandle` completes when shutdown finishes.
///
/// Use this for tests that need to verify graceful shutdown behaviour
/// (pipeline drain, LoomStopped logging, in-flight work completion).
pub fn spawn_server_with_shutdown(
    config: knot::AppConfig,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let _ = knot::start_server_with_shutdown(
            config,
            knot::ShutdownSignal::Channel(rx),
        )
        .await;
    });
    (handle, tx)
}

/// Poll a knot status endpoint until it reaches a terminal state.
pub async fn poll_knot_status(
    host_port: &str,
    loom_id: &str,
    knot_id: &str,
    max_retries: usize,
    delay_ms: u64,
) -> Result<serde_json::Value, String> {
    for attempt in 0..max_retries {
        let path = format!("/looms/{loom_id}/knots/{knot_id}");
        match http_get(host_port, &path).await {
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
            sleep(Duration::from_millis(delay_ms)).await;
        }
    }
    Err("timeout waiting for knot status".to_string())
}
