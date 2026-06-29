//! JSON agent runner — invokes the Pi CLI with `--mode json` and parses
//! the JSON-L output stream.
//!
//! Reads stdout line-by-line as newline-delimited JSON, extracting:
//! - Session ID from the first `session` event
//! - Token usage from `agent_end` usage data
//! - Response text from `message_end` or `agent_end` message content
//!
//! Falls back to raw stdout if JSON-L parsing fails.

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::application::ports::{
    AgentInvocationMetadata, AgentOutput, AgentRunner, ExecutionContext,
    PortError, TokenUsage,
};
use crate::domain::entities::StrandPath;
use crate::domain::value_objects::AgentConfig;

/// JSON-L implementation of [`AgentRunner`].
///
/// Appends `--mode json` to CLI arguments, spawns the child process,
/// and parses stdout as newline-delimited JSON events.
#[derive(Debug, Clone)]
pub struct PiJsonAgentRunner {
    /// Maximum duration the agent may run before being killed.
    /// Defaults to 120 seconds.
    timeout: Duration,
    /// Path to the agent CLI binary. Resolved once at construction time
    /// to avoid PATH lookup races at execution time.
    cli_path: String,
}

impl Default for PiJsonAgentRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            cli_path: Self::resolve_cli_path(),
        }
    }
}

impl PiJsonAgentRunner {
    /// Create a new runner with the default 120-second timeout.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new runner with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            cli_path: Self::resolve_cli_path(),
        }
    }

    /// Create a new runner with a specific CLI path.
    /// Used by integration tests to inject a mock binary.
    #[cfg(test)]
    pub fn with_cli_path(cli_path: String) -> Self {
        Self {
            timeout: Duration::from_secs(120),
            cli_path,
        }
    }

    /// Resolve the CLI path for the agent binary.
    ///
    /// Checks the `KNOT_TEST_CLI_PATH` environment variable first (set by
    /// integration test helpers), then falls back to PATH lookup of `"pi"`.
    fn resolve_cli_path() -> String {
        std::env::var("KNOT_TEST_CLI_PATH").unwrap_or_else(|_| "pi".to_string())
    }

    /// Append `--mode json` flags to CLI args.
    fn build_json_cli_args(cli_args: &[String]) -> Vec<String> {
        let mut args = cli_args.to_vec();
        args.push("--mode".to_string());
        args.push("json".to_string());
        args
    }

    /// Build the prompt with profile prompt, trigger line, and knot
    /// instructions.
    fn build_prompt_with_context(
        ctx: &ExecutionContext,
        profile_prompt: &str,
    ) -> String {
        let mut full_prompt = String::new();

        if !profile_prompt.is_empty() {
            full_prompt.push_str(profile_prompt);
            full_prompt.push_str("\n\n");
        }

        full_prompt.push_str(&ctx.prompt);

        if !ctx.event_type.is_empty() {
            full_prompt.push_str("\n\n");
            full_prompt.push_str(&format!(
                "**{}** triggered by **{}** on **{}**",
                ctx.knot_name.as_deref().unwrap_or("unknown"),
                ctx.event_type,
                ctx.strand_path.0.display()
            ));
        }

        full_prompt
    }

    /// Parse a single JSON-L line and update tracked state.
    ///
    /// Returns `true` if the line was valid JSON, `false` otherwise.
    /// When `false` is returned the caller should treat output as raw.
    fn parse_json_line(
        line: &str,
        session_id: &mut Option<String>,
        response_text: &mut String,
        token_usage: &mut Option<TokenUsage>,
    ) -> bool {
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return false,
        };

        let event_type = value.get("type").and_then(|t| t.as_str());

        match event_type {
            Some("session") => {
                if let Some(id) = value.get("id").and_then(|id| id.as_str()) {
                    *session_id = Some(id.to_string());
                }
            }
            Some("message_end") => {
                // Response text from message_end with role: "assistant"
                if let Some(role) = value.get("role").and_then(|r| r.as_str()) {
                    if role == "assistant" {
                        if let Some(content) = value.get("content") {
                            if let Some(text) = content.as_str() {
                                response_text.push_str(text);
                            }
                        }
                    }
                }
            }
            Some("agent_end") => {
                // Extract token usage from usage object
                if let Some(usage_obj) = value.get("usage") {
                    *token_usage = Some(TokenUsage {
                        input: usage_obj
                            .get("input")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        output: usage_obj
                            .get("output")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        cache_read: usage_obj
                            .get("cache_read")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        cache_write: usage_obj
                            .get("cache_write")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        total: usage_obj
                            .get("total")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                    });
                }

                // Extract response text from messages array
                // Format: messages[].content[].text
                if let Some(messages) = value.get("messages") {
                    if let Some(arr) = messages.as_array() {
                        for msg in arr {
                            if let Some(role) =
                                msg.get("role").and_then(|r| r.as_str())
                            {
                                if role == "assistant" {
                                    if let Some(content) = msg.get("content") {
                                        if let Some(carr) = content.as_array() {
                                            for item in carr {
                                                if let Some(text) =
                                                    item.get("text").and_then(|t| t.as_str())
                                                {
                                                    response_text.push_str(text);
                                                }
                                            }
                                        } else if let Some(text) =
                                            content.as_str()
                                        {
                                            response_text.push_str(text);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        true
    }

    /// Parse JSON-L from raw stdout, extracting session_id, response,
    /// and token usage. Returns `true` if all lines parsed as valid JSON.
    fn parse_stdout(
        raw_stdout: &str,
    ) -> (bool, Option<String>, String, Option<TokenUsage>) {
        let mut session_id: Option<String> = None;
        let mut response_text = String::new();
        let mut token_usage: Option<TokenUsage> = None;
        let mut had_parse_error = false;

        for line in raw_stdout.lines() {
            if line.is_empty() {
                continue;
            }
            if !Self::parse_json_line(
                line,
                &mut session_id,
                &mut response_text,
                &mut token_usage,
            ) {
                had_parse_error = true;
            }
        }

        (
            had_parse_error,
            session_id,
            response_text,
            token_usage,
        )
    }
}

impl AgentRunner for PiJsonAgentRunner {
    fn execute(&self, ctx: ExecutionContext) -> Result<AgentOutput, PortError> {
        // Build CLI args from agent_config, then append --mode json.
        let base_args = ctx.agent_config.build_cli_args();
        let cli_args = Self::build_json_cli_args(&base_args);

        // Spawn the child process in its own process group so we can
        // kill the entire group (including child processes) on timeout.
        let child = unsafe {
            std::process::Command::new(&self.cli_path)
                .args(&cli_args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .pre_exec(|| {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                })
                .spawn()
        };

        let mut child = match child {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(PortError::CommandNotFound(format!(
                    "'{}': {}",
                    self.cli_path, e
                )));
            }
            Err(e) => {
                return Err(PortError::AgentExecutionFailed {
                    message: format!(
                        "failed to spawn '{}': {}",
                        self.cli_path, e
                    ),
                    session_id: None,
                });
            }
        };

        let child_pid = child.id() as i32;
        let cli_path = self.cli_path.clone();
        let strand_desc = ctx.strand_path.0.display().to_string();
        let strand_desc_warn = strand_desc.clone();
        let effective_timeout = ctx.timeout.unwrap_or(self.timeout);

        // Shared flag: set to true when the child exits normally.
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_for_thread = Arc::clone(&cancelled);

        // Spawn a background thread that kills the child on timeout.
        let _timeout_thread = std::thread::Builder::new()
            .name("json-timeout".to_string())
            .spawn(move || {
                std::thread::sleep(effective_timeout);
                if cancelled_for_thread.load(Ordering::Relaxed) {
                    return;
                }
                // Kill the entire process group (child + subprocesses)
                let _ = unsafe { libc::kill(-child_pid, libc::SIGKILL) };
                eprintln!(
                    "WARNING: killed '{}' after timeout of {:?} (strand: {})",
                    cli_path, effective_timeout, strand_desc_warn
                );
            })
            .map_err(|e| {
                PortError::AgentExecutionFailed {
                    message: format!("failed to spawn timeout thread: {e}"),
                    session_id: None,
                }
            })?;

        // Write the prompt to the child's stdin.
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let profile_prompt = ctx.profile_prompt.clone();
        let prompt_with_context =
            Self::build_prompt_with_context(&ctx, &profile_prompt);

        stdin
            .write_all(prompt_with_context.as_bytes())
            .map_err(|e| {
                PortError::AgentExecutionFailed {
                    message: format!("failed to write prompt to stdin: {e}"),
                    session_id: None,
                }
            })?;
        drop(stdin);

        // Wait for the child and capture output.
        // Use a thread + timeout so we don't block forever if
        // `wait_with_output()` hangs (e.g. orphaned child processes
        // preventing pipe close).
        let wait_handle = std::thread::Builder::new()
            .name("json-wait".to_string())
            .spawn(move || child.wait_with_output())
            .map_err(|e| {
                PortError::AgentExecutionFailed {
                    message: format!("failed to spawn wait thread: {e}"),
                    session_id: None,
                }
            })?;

        // Wait up to 2x the effective timeout for the child to exit.
        // The timeout thread kills the child after effective_timeout,
        // so 2x gives it time to clean up.
        let wait_deadline = effective_timeout.saturating_mul(2)
            .max(Duration::from_secs(5));
        let start_wait = std::time::Instant::now();
        let mut output = None;
        loop {
            if wait_handle.is_finished() {
                output = Some(wait_handle.join().expect("wait thread panicked"));
                break;
            }
            if start_wait.elapsed() > wait_deadline {
                // Child didn't exit in time — force kill again and wait.
                // Kill the entire process group.
                let _ = unsafe { libc::kill(-child_pid, libc::SIGKILL) };
                // Give it a moment, then try joining anyway.
                std::thread::sleep(Duration::from_millis(500));
                output = Some(
                    wait_handle
                        .join()
                        .expect("wait thread panicked"),
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let output = output.unwrap().map_err(|e| {
            PortError::AgentExecutionFailed {
                message: format!(
                    "failed to wait for '{}': {}",
                    self.cli_path, e
                ),
                session_id: None,
            }
        })?;

        // Mark cancelled so the timeout thread suppresses its warning.
        cancelled.store(true, Ordering::Relaxed);

        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let raw_stdout =
            String::from_utf8_lossy(&output.stdout).into_owned();

        // If status code is None, the process was killed by a signal
        // (SIGKILL from our timeout thread).
        if output.status.code().is_none() {
            let (
                _had_error,
                session_id,
                _response,
                _token_usage,
            ) = Self::parse_stdout(&raw_stdout);
            return Err(PortError::Timeout {
                message: format!(
                    "'{}' exceeded timeout of {:?} (strand: {})",
                    self.cli_path, effective_timeout, strand_desc
                ),
                session_id,
            });
        }

        let exit_code = output.status.code().unwrap_or(-1);

        if exit_code != 0 {
            let (
                _had_error,
                session_id,
                _response,
                _token_usage,
            ) = Self::parse_stdout(&raw_stdout);
            return Err(PortError::AgentExecutionFailed {
                message: format!(
                    "'{}' exited with code {}: {}",
                    self.cli_path,
                    exit_code,
                    if stderr.is_empty() {
                        raw_stdout.clone()
                    } else {
                        stderr.clone()
                    }
                ),
                session_id,
            });
        }

        // Success — parse JSON-L from stdout.
        if raw_stdout.trim().is_empty() {
            return Ok(AgentOutput {
                stdout: String::new(),
                stderr,
                exit_code,
                metadata: None,
            });
        }

        let (had_parse_error, session_id, response_text, token_usage) =
            Self::parse_stdout(&raw_stdout);

        if had_parse_error {
            // Graceful degradation — treat as plain text.
            Ok(AgentOutput {
                stdout: raw_stdout,
                stderr,
                exit_code,
                metadata: None,
            })
        } else {
            Ok(AgentOutput {
                stdout: response_text,
                stderr,
                exit_code,
                metadata: Some(AgentInvocationMetadata {
                    session_id,
                    token_usage,
                }),
            })
        }
    }

    fn execute_with_config(
        &self,
        agent_config: &AgentConfig,
        strand_path: StrandPath,
        strand_file_ref: Option<StrandPath>,
        prompt: String,
        profile_prompt: String,
        event_type: String,
        knot_name: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<AgentOutput, PortError> {
        // Clone config and append adapter-specific args via extra_args.
        // The retry loop populates extra_args with --session-id.
        let mut config = agent_config.clone();

        // Append --name for pi session title.
        let strand_filename = strand_path.0
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let session_title = format!(
            "{} triggered by {} on {}",
            knot_name.as_deref().unwrap_or("unknown"),
            event_type,
            strand_filename,
        );
        config.extra_args.push("--name".to_string());
        config.extra_args.push(session_title);
        // Append strand content reference using pi's @file syntax.
        // Only for Created/Modified events (file exists on disk).
        if let Some(ref file_path) = strand_file_ref {
            config.extra_args.push(format!("@{}", file_path.0.display()));
        }

        let ctx = ExecutionContext {
            agent_config: config,
            prompt,
            profile_prompt,
            strand_path,
            event_type,
            knot_name,
            timeout,
        };
        self.execute(ctx)
    }

    fn runner_type(&self) -> &str {
        "pi-json"
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Mock script: passes stdin through to stdout (for JSON-L parsing tests).
    fn make_json_mock_script() -> String {
        r#"#!/usr/bin/env bash
cat
echo ""
"#
            .to_string()
    }

    fn make_json_mock_path() -> PathBuf {
        std::env::temp_dir().join("knot-test-mock-json")
    }

    /// Create a PiJsonAgentRunner configured with the passthrough mock.
    fn make_mock_json_runner() -> PiJsonAgentRunner {
        let script = make_json_mock_script();
        let path = make_json_mock_path();
        std::fs::write(&path, &script).ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &path,
                std::fs::Permissions::from_mode(0o755),
            )
            .ok();
        }
        PiJsonAgentRunner::with_cli_path(path.to_string_lossy().to_string())
    }

    /// Blocking mock script: sleeps for a long time.
    /// Used by timeout tests so the timeout thread actually fires.
    fn make_json_blocking_mock_script() -> String {
        r#"#!/usr/bin/env bash
sleep 300
"#
            .to_string()
    }

    fn make_json_blocking_mock_path() -> PathBuf {
        std::env::temp_dir().join("knot-test-mock-json-blocking")
    }

    /// Create a PiJsonAgentRunner configured with the blocking mock.
    /// The process stays alive long enough for timeout tests to fire.
    fn make_blocking_json_runner() -> PiJsonAgentRunner {
        let script = make_json_blocking_mock_script();
        let path = make_json_blocking_mock_path();
        std::fs::write(&path, &script).ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &path,
                std::fs::Permissions::from_mode(0o755),
            )
            .ok();
        }
        PiJsonAgentRunner::with_cli_path(path.to_string_lossy().to_string())
    }

    fn make_context(args: &[&str]) -> ExecutionContext {
        ExecutionContext {
            agent_config: AgentConfig {
                goal: "test".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: args.iter().map(|s| s.to_string()).collect(),
            },
            prompt: "test prompt".to_string(),
            profile_prompt: "You are a test agent.".to_string(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        }
    }


    // ── JSON-L Parser Unit Tests (direct, no subprocess) ─────────────

    /// Helper to call parse_stdout from tests.
    fn run_parse_stdout(raw: &str) -> (bool, Option<String>, String, Option<TokenUsage>) {
        PiJsonAgentRunner::parse_stdout(raw)
    }

    /// Unit test: `parse_stdout` extracts session ID from JSON-L.
    #[test]
    fn test_json_runner_parses_session_id() {
        let raw = r#"{"type":"session","id":"abc-123"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"hello"}]}]}"#;
        let (had_error, session_id, response_text, _usage) =
            run_parse_stdout(raw);
        assert!(!had_error, "should parse cleanly");
        assert_eq!(session_id.as_deref(), Some("abc-123"));
        assert!(response_text.contains("hello"));
    }

    /// Unit test: `parse_stdout` extracts token usage.
    #[test]
    fn test_json_runner_parses_token_usage() {
        let raw = r#"{"type":"session","id":"sess-1"}
{"type":"agent_end","usage":{"input":100,"output":50,"cache_read":10,"cache_write":5,"total":165},"messages":[{"role":"assistant","content":[{"type":"text","text":"ok"}]}]}"#;
        let (_had_error, _session_id, _response, usage) =
            run_parse_stdout(raw);
        let usage = usage.unwrap();
        assert_eq!(usage.input, 100);
        assert_eq!(usage.output, 50);
        assert_eq!(usage.cache_read, 10);
        assert_eq!(usage.cache_write, 5);
        assert_eq!(usage.total, 165);
    }

    /// Unit test: `parse_stdout` extracts response text.
    #[test]
    fn test_json_runner_parses_response_text() {
        let raw = r#"{"type":"session","id":"sess-x"}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"the response text"}]}]}"#;
        let (_had_error, _session_id, response_text, _usage) =
            run_parse_stdout(raw);
        assert!(response_text.contains("the response text"));
    }

    /// Unit test: `parse_stdout` extracts session_id from timeout error.
    #[test]
    fn test_json_runner_timeout_captures_session_id() {
        let raw = r#"{"type":"session","id":"timeout-sess"}"#;
        let (_had_error, session_id, _response, _usage) =
            run_parse_stdout(raw);
        assert_eq!(session_id.as_deref(), Some("timeout-sess"));
    }

    /// Unit test: `parse_stdout` extracts session_id from failure output.
    #[test]
    fn test_json_runner_nonzero_exit_captures_session_id() {
        let raw = r#"{"type":"session","id":"fail-sess"}"#;
        let (_had_error, session_id, _response, _usage) =
            run_parse_stdout(raw);
        assert_eq!(session_id.as_deref(), Some("fail-sess"));
    }

    #[test]
    fn test_json_runner_command_not_found() {
        let runner = PiJsonAgentRunner::with_cli_path(
            "/nonexistent/json-runner".to_string(),
        );
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for missing binary");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::CommandNotFound(_)),
            "expected CommandNotFound, got {err:?}"
        );
        assert!(
            err.session_id().is_none(),
            "no session_id for CommandNotFound"
        );
    }

    /// Unit test: malformed JSON sets had_error flag.
    /// The raw fallback (returning full stdout) happens at the
    /// `execute` level when had_parse_error is true.
    #[test]
    fn test_json_runner_malformed_json_fallback() {
        let raw = "not json at all\ngarbled output\n";
        let (had_error, _session_id, response, _usage) =
            run_parse_stdout(raw);
        assert!(had_error, "should have parse errors");
        // parse_stdout doesn't accumulate raw lines — response_text
        // is empty for non-JSON input. The caller (execute) uses the
        // had_error flag to fall back to raw stdout instead.
        assert!(response.is_empty());
    }

    /// Unit test: empty input produces empty output.
    #[test]
    fn test_json_runner_empty_output() {
        let raw = "";
        let (had_error, _session_id, response, _usage) =
            run_parse_stdout(raw);
        assert!(!had_error);
        assert!(response.is_empty());
    }

    /// Unit test: `--mode json` is appended by `build_json_cli_args`.
    #[test]
    fn test_json_runner_adds_mode_json_flag() {
        let base_args = vec!["-p".to_string(), "--model".to_string(), "gpt-4o".to_string()];
        let json_args = PiJsonAgentRunner::build_json_cli_args(&base_args);
        assert!(json_args.contains(&"--mode".to_string()));
        assert!(json_args.contains(&"json".to_string()));
        assert!(json_args.contains(&"gpt-4o".to_string()));
    }

    /// Unit test: `message_end` events extract response text.
    #[test]
    fn test_json_runner_parses_message_end_response() {
        let raw = r#"{"type":"session","id":"msg-sess"}
{"type":"message_end","role":"assistant","content":"response from message_end"}"#;
        let (_had_error, _session_id, response, _usage) =
            run_parse_stdout(raw);
        assert!(response.contains("response from message_end"));
    }

    /// Integration test: mock echoes stdin which contains the prompt.
    #[test]
    fn test_json_runner_prompt_passthrough() {
        let runner = make_mock_json_runner();
        let mut ctx = make_context(&[]);
        ctx.prompt = "my knot instructions".to_string();

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        // The mock echoes stdin, which contains the prompt chain.
        assert!(
            output.stdout.contains("my knot instructions"),
            "stdout should contain prompt: {}",
            output.stdout
        );
    }

    /// Integration test: mock echoes stdin → timeout kills it.
    #[test]
    fn test_json_runner_context_timeout_override() {
        let runner = make_blocking_json_runner();
        let mut ctx = make_context(&[]);
        ctx.timeout = Some(Duration::from_millis(50));

        let start = std::time::Instant::now();
        let result = runner.execute(ctx);
        let elapsed = start.elapsed();

        assert!(result.is_err(), "should error for timeout");
        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "should use context timeout, not runner default"
        );
    }
}
