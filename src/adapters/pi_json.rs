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
}

impl Default for PiJsonAgentRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
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
        Self { timeout }
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
        let cli_args = Self::build_json_cli_args(&ctx.cli_args);

        // Spawn the child process.
        let child = std::process::Command::new(&ctx.cli_path)
            .args(&cli_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(PortError::CommandNotFound(format!(
                    "'{}': {}",
                    ctx.cli_path, e
                )));
            }
            Err(e) => {
                return Err(PortError::AgentExecutionFailed {
                    message: format!(
                        "failed to spawn '{}': {}",
                        ctx.cli_path, e
                    ),
                    session_id: None,
                });
            }
        };

        let child_pid = child.id() as i32;
        let cli_path = ctx.cli_path.clone();
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
                let _ = unsafe { libc::kill(child_pid, libc::SIGKILL) };
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
        let output = child.wait_with_output().map_err(|e| {
            PortError::AgentExecutionFailed {
                message: format!(
                    "failed to wait for '{}': {}",
                    ctx.cli_path, e
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
                    ctx.cli_path, effective_timeout, strand_desc
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
                    ctx.cli_path,
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
        let mut cli_args = agent_config.build_cli_args();
        // Append --name for pi session title
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
        cli_args.push("--name".to_string());
        cli_args.push(session_title);
        // Append strand content reference using pi's @file syntax.
        // Only for Created/Modified events (file exists on disk).
        if let Some(ref file_path) = strand_file_ref {
            cli_args.push(format!("@{}", file_path.0.display()));
        }

        let ctx = ExecutionContext {
            cli_path: "pi".to_string(),
            cli_args,
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

    fn make_context(cli: &str, args: &[&str]) -> ExecutionContext {
        ExecutionContext {
            cli_path: cli.to_string(),
            cli_args: args.iter().map(|s| s.to_string()).collect(),
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

    /// Build an ExecutionContext that simulates JSON-L output using `sh -c`.
    fn jsonl_context(session_id: &str, response: &str) -> ExecutionContext {
        let script = format!(
            r#"echo '{{"type":"session","id":"{}"}}'; echo '{{"type":"agent_end","messages":[{{"role":"assistant","content":[{{"type":"text","text":"{}"}}]}}]}}'"#,
            session_id, response
        );
        ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), script],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        }
    }

    #[test]
    fn test_json_runner_parses_session_id() {
        let runner = PiJsonAgentRunner::new();
        let ctx = jsonl_context("abc-123", "hello");

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(
            output.metadata.is_some(),
            "metadata should be present"
        );
        let metadata = output.metadata.unwrap();
        assert_eq!(
            metadata.session_id.as_deref(),
            Some("abc-123"),
            "session_id should match"
        );
    }

    #[test]
    fn test_json_runner_parses_token_usage() {
        let runner = PiJsonAgentRunner::new();
        let script = r#"echo '{"type":"session","id":"sess-1"}'; echo '{"type":"agent_end","usage":{"input":100,"output":50,"cache_read":10,"cache_write":5,"total":165},"messages":[{"role":"assistant","content":[{"type":"text","text":"ok"}]}]}'"#;
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), script.to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        let metadata = output.metadata.unwrap();
        let usage = metadata.token_usage.unwrap();
        assert_eq!(usage.input, 100);
        assert_eq!(usage.output, 50);
        assert_eq!(usage.cache_read, 10);
        assert_eq!(usage.cache_write, 5);
        assert_eq!(usage.total, 165);
    }

    #[test]
    fn test_json_runner_parses_response_text() {
        let runner = PiJsonAgentRunner::new();
        let ctx = jsonl_context("sess-x", "the response text");

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(
            output.stdout.contains("the response text"),
            "stdout should contain response text: {}",
            output.stdout
        );
    }

    #[test]
    fn test_json_runner_timeout_captures_session_id() {
        let runner =
            PiJsonAgentRunner::with_timeout(Duration::from_millis(100));
        // Emit session line then sleep (will be killed)
        let script =
            r#"echo '{"type":"session","id":"timeout-sess"}'; sleep 30"#;
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), script.to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for timeout");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        assert_eq!(
            err.session_id().map(|s| s.as_str()),
            Some("timeout-sess"),
            "session_id should be captured despite timeout"
        );
    }

    #[test]
    fn test_json_runner_nonzero_exit_captures_session_id() {
        let runner = PiJsonAgentRunner::new();
        // Emit session line then exit with code 1
        let script =
            r#"echo '{"type":"session","id":"fail-sess"}'; exit 1"#;
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), script.to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for non-zero exit");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::AgentExecutionFailed { .. }),
            "expected AgentExecutionFailed, got {err:?}"
        );
        assert_eq!(
            err.session_id().map(|s| s.as_str()),
            Some("fail-sess"),
            "session_id should be captured on failure"
        );
    }

    #[test]
    fn test_json_runner_command_not_found() {
        let runner = PiJsonAgentRunner::new();
        let ctx = make_context("/nonexistent/json-runner", &[]);

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

    #[test]
    fn test_json_runner_malformed_json_fallback() {
        let runner = PiJsonAgentRunner::new();
        let script = r#"echo 'not json at all'; echo 'garbled output'"#;
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), script.to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed (fallback)");

        let output = result.unwrap();
        assert!(
            output.metadata.is_none(),
            "metadata should be None for malformed output"
        );
        assert!(
            output.stdout.contains("not json at all"),
            "stdout should contain raw output: {}",
            output.stdout
        );
    }

    #[test]
    fn test_json_runner_empty_output() {
        let runner = PiJsonAgentRunner::new();
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), "cat >/dev/null".to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(
            output.stdout.is_empty(),
            "stdout should be empty: {}",
            output.stdout
        );
        assert!(
            output.metadata.is_none(),
            "metadata should be None for empty output"
        );
    }

    #[test]
    fn test_json_runner_adds_mode_json_flag() {
        let runner = PiJsonAgentRunner::new();
        // `cat >/dev/null` consumes stdin; positional args $0 and $1
        // are the `--mode json` flags appended by the adapter.
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec![
                "-c".to_string(),
                "cat >/dev/null; echo \"$0\" \"|\" \"$1\"".to_string(),
            ],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(
            output.stdout.contains("--mode"),
            "stdout should contain --mode flag: {}",
            output.stdout
        );
        assert!(
            output.stdout.contains("json"),
            "stdout should contain json flag: {}",
            output.stdout
        );
    }

    #[test]
    fn test_json_runner_parses_message_end_response() {
        // Verify response text is extracted from message_end events
        let runner = PiJsonAgentRunner::new();
        let script = r#"echo '{"type":"session","id":"msg-sess"}'; echo '{"type":"message_end","role":"assistant","content":"response from message_end"}'"#;
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), script.to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(
            output.stdout.contains("response from message_end"),
            "stdout should contain message_end response: {}",
            output.stdout
        );
    }

    #[test]
    fn test_json_runner_prompt_passthrough() {
        // Verify the prompt is sent via stdin (like SubprocessAgentRunner)
        let runner = PiJsonAgentRunner::new();
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), "cat".to_string()],
            prompt: "my knot instructions".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(
            output.stdout.contains("my knot instructions"),
            "stdout should contain prompt: {}",
            output.stdout
        );
    }

    #[test]
    fn test_json_runner_context_timeout_override() {
        let runner =
            PiJsonAgentRunner::with_timeout(Duration::from_secs(120));
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), "exec sleep 30".to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(
                PathBuf::from("test.md"),
            ),
            event_type: String::new(),
            knot_name: None,
            timeout: Some(Duration::from_millis(50)),
        };

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
