//! Shared test infrastructure for integration tests.
//!
//! Provides helper functions for creating test fixtures, spawning servers,
//! making HTTP requests, and polling for expected states.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use knot::AppConfig;
use knot::ShutdownSignal;

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

/// Simple synchronous HTTP GET using raw TCP.
pub fn http_get(host_port: &str, path: &str) -> Result<(String, String), String> {
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
pub fn http_get_retry(
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

/// Simple synchronous HTTP POST with JSON body using raw TCP.
pub fn http_post_json(
    host_port: &str,
    path: &str,
    body: &serde_json::Value,
) -> Result<(String, String), String> {
    let body_str = body.to_string();
    let mut stream = TcpStream::connect(host_port)
        .map_err(|e| format!("connect failed: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: \
         application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n{body_str}",
        body_str.len()
    );

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

/// Simple synchronous HTTP DELETE using raw TCP.
pub fn http_delete(host_port: &str, path: &str) -> Result<(String, String), String> {
    let mut stream = TcpStream::connect(host_port)
        .map_err(|e| format!("connect failed: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

    let request = format!(
        "DELETE {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: \
         close\r\n\r\n"
    );

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

// ── Server Helpers ─────────────────────────────────────────────────────────

/// Wait for a TCP port to become available.
pub fn wait_for_port(host_port: &str, max_retries: usize, delay_ms: u64) -> Result<(), String> {
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
pub fn spawn_server(config: AppConfig) -> tokio::sync::oneshot::Sender<()> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("create runtime");
        let signal = ShutdownSignal::Channel(shutdown_rx);
        let _ = rt.block_on(knot::start_server_with_shutdown(config, signal));
    });

    shutdown_tx
}

/// Poll a knot status endpoint until it reaches a terminal state.
pub fn poll_knot_status(
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
