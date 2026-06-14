//! Shared test infrastructure for integration tests.

#![allow(dead_code)]
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

/// Generate valid knot definition file content with absolute strand path.
/// Creates the strand directory if it doesn't exist.
/// Returns (knot_content, strand_dir). Tie-off paths are statically derived.
pub fn make_knot_content_with_dirs(
    project_root: &Path,
) -> (String, std::path::PathBuf) {
    let strand_dir = project_root.join("strands");
    fs::create_dir_all(&strand_dir).unwrap();
    let content = format!(
        "---\nname: review-knot\nagent-profile-ref: fast\nstrand-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review the goals section of this PRD.\n---\n\n# Review Knot\n\nThis knot reviews PRD goals.\n",
        strand_dir.display()
    );
    (content, strand_dir)
}

/// Generate valid knot definition file content (ignores returned paths).
pub fn make_knot_content(project_root: &Path) -> String {
    let (content, _) = make_knot_content_with_dirs(project_root);
    content
}

/// Create the "fast" agent profile in the given directory.
///
/// Writes `profiles/fast.md` with a valid AgentProfile frontmatter so that
/// knots with `agent-profile-ref: fast` can resolve at runtime.
pub fn create_fast_profile(dir: &Path) {
    let profiles_dir = dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    fs::write(
        profiles_dir.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\nsystem-prompt: |\n  You are a reviewer.\n---\n\nFast Profile\n",
    )
    .unwrap();
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
        format!("#!/bin/sh\ncat >/dev/null\necho '{}'\n", output),
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

/// Helper: wait for auto-discovery to register a loom (poll GET /looms).
pub async fn wait_for_loom_discovery(
    host_port: &str,
    expected_count: usize,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let result = tokio::time::timeout(
            Duration::from_millis(2000),
            http_get(host_port, "/looms"),
        )
        .await;
        let (st, body) = match result {
            Ok(Ok(r)) => r,
            _ => continue,
        };
        if st.contains("200") {
            let summaries: Vec<serde_json::Value> =
                serde_json::from_str(&body).unwrap_or_default();
            if summaries.len() == expected_count {
                return true;
            }
        }
    }
    false
}

/// Helper: wait for a knot to appear in the loom's knot list.
pub async fn wait_for_knot_count(
    host_port: &str,
    loom_id: &str,
    expected: usize,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let result = tokio::time::timeout(
            Duration::from_millis(2000),
            http_get(host_port, &format!("/looms/{loom_id}/knots")),
        )
        .await;
        let (st, body) = match result {
            Ok(Ok(r)) => r,
            _ => continue,
        };
        if st.contains("200") {
            let knots: Vec<String> =
                serde_json::from_str(&body).unwrap_or_default();
            if knots.len() == expected {
                return true;
            }
        }
    }
    false
}

// ── Git Helpers ────────────────────────────────────────────────────────────

/// Initialise a git repository in the given directory.
///
/// Runs `git init -b main` and configures `user.email` and `user.name`
/// so that commits can be made without interactive prompts.
///
/// Returns `true` if git was successfully initialised.
pub fn init_git_repo(dir: &Path) -> bool {
    let init = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir)
        .output();

    if init.as_ref().map(|o| o.status.success()).unwrap_or(false) {
        // Configure user so commits don't fail
        let _ = std::process::Command::new("git")
            .args(["config", "user.email", "test@knot.local"])
            .current_dir(dir)
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "user.name", "Knot Test"])
            .current_dir(dir)
            .output();

        // Create an initial empty commit so the repo has a baseline.
        // This ensures `git add -A` / `git commit` work even when the
        // first change is just the tie-off file.
        // We create a small sentinel file first so the initial commit
        // has content (avoids empty tree issues on some git versions).
        let sentinel = dir.join(".knot-git-sentinel");
        let _ = std::fs::write(&sentinel, "");
        let _ = std::process::Command::new("git")
            .args(["add", ".knot-git-sentinel"])
            .current_dir(dir)
            .output();
        let _ = std::process::Command::new("git")
            .args(["commit", "-m", "initial commit"])
            .current_dir(dir)
            .output();

        true
    } else {
        false
    }
}

/// Read the latest commit message from a git repository.
///
/// Returns `(subject, body)` where `subject` is the first line and
/// `body` is everything after the blank line separator.
///
/// Returns `None` if the directory is not a git repository or has no
/// commits.
pub fn get_latest_commit(dir: &Path) -> Option<(String, String)> {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%B"])
        .current_dir(dir)
        .output()
        .ok()?;

    let msg = String::from_utf8_lossy(&output.stdout).to_string();
    let lines: Vec<&str> = msg.lines().collect();
    let subject = lines.first().map(|s| s.to_string())?;
    let body = msg
        .find("\n\n")
        .map(|pos| msg[pos + 2..].trim().to_string())
        .unwrap_or_default();

    Some((subject, body))
}

/// Count the number of commits in a git repository.
pub fn count_commits(dir: &Path) -> Option<usize> {
    let output = std::process::Command::new("git")
        .args(["log", "--format=%H"])
        .current_dir(dir)
        .output()
        .ok()?;

    let log = String::from_utf8_lossy(&output.stdout);
    Some(log.lines().filter(|l| !l.is_empty()).count())
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
