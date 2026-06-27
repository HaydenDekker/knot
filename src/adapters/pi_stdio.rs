//! Stdio agent runner — invokes the Pi CLI via a child process in
//! plain-text (stdio) mode.
//!
//! This is the default adapter. It captures stdout/stderr as raw text
//! and returns the stdout as the agent response.

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::application::ports::{
    AgentOutput, AgentRunner, ExecutionContext, PortError,
};
use crate::domain::entities::StrandPath;
use crate::domain::value_objects::AgentConfig;

/// Stdio-backed implementation of [`AgentRunner`].
///
/// Spawns the Pi CLI as a child process in plain-text mode, captures
/// stdout and stderr, and enforces a configurable timeout.
#[derive(Debug, Clone)]
pub struct PiStdioAgentRunner {
    /// Maximum duration the agent may run before being killed.
    /// Defaults to 120 seconds.
    timeout: Duration,
}

impl Default for PiStdioAgentRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
        }
    }
}

impl PiStdioAgentRunner {
    /// Create a new runner with the default 120-second timeout.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new runner with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Build the prompt with profile prompt, trigger line, and knot
    /// instructions.
    ///
    /// Ordering: profile prompt (persona) → knot instructions (task)
    /// → trigger line (event context). The strand file is referenced via
    /// `@{path}` in CLI args — not injected into the prompt body.
    fn build_prompt_with_context(
        ctx: &ExecutionContext,
        profile_prompt: &str,
    ) -> String {
        let mut full_prompt = String::new();

        // Profile prompt (agent persona/instructions)
        if !profile_prompt.is_empty() {
            full_prompt.push_str(profile_prompt);
            full_prompt.push_str("\n\n");
        }

        // Knot instructions (task-specific direction)
        full_prompt.push_str(&ctx.prompt);

        // Prepend a short trigger line for event awareness
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
}

impl AgentRunner for PiStdioAgentRunner {
    fn execute(&self, ctx: ExecutionContext) -> Result<AgentOutput, PortError> {
        // Spawn the child process.
        let child = std::process::Command::new(&ctx.cli_path)
            .args(&ctx.cli_args)
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
        let strand_desc_warn = strand_desc.clone(); // for timeout thread closure
        let effective_timeout = ctx.timeout.unwrap_or(self.timeout);

        // Shared flag: set to true when the child exits normally so the
        // timeout thread can suppress its warning (avoids spurious messages
        // when the agent finishes well before the deadline).
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_for_thread = Arc::clone(&cancelled);

        // Spawn a background thread that kills the child on timeout.
        let _timeout_thread = std::thread::Builder::new()
            .name("stdio-timeout".to_string())
            .spawn(move || {
                std::thread::sleep(effective_timeout);
                // If the child already exited, skip the kill + warning.
                if cancelled_for_thread.load(Ordering::Relaxed) {
                    return;
                }
                let _ = unsafe {
                    libc::kill(child_pid, libc::SIGKILL)
                };
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
        use std::io::Write;

        // Build full prompt: profile prompt → knot instructions → trigger line.
        let profile_prompt = ctx.profile_prompt.clone();
        let prompt_with_context = Self::build_prompt_with_context(&ctx, &profile_prompt);

        stdin
            .write_all(prompt_with_context.as_bytes())
            .map_err(|e| {
                PortError::AgentExecutionFailed {
                    message: format!("failed to write prompt to stdin: {e}"),
                    session_id: None,
                }
            })?;
        // Drop stdin to close the pipe (signals EOF to the child).
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

        // Mark cancelled so the background timeout thread suppresses its
        // warning if the child exited before the deadline.
        cancelled.store(true, Ordering::Relaxed);

        // If status code is None, the process was killed by a signal
        // (SIGKILL from our timeout thread).
        if output.status.code().is_none() {
            return Err(PortError::Timeout {
                message: format!(
                    "'{}' exceeded timeout of {:?} (strand: {})",
                    ctx.cli_path, effective_timeout, strand_desc
                ),
                session_id: None,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let exit_code = output.status.code().unwrap_or(-1);

        if exit_code != 0 {
            return Err(PortError::AgentExecutionFailed {
                message: format!(
                    "'{}' exited with code {}: {}",
                    ctx.cli_path,
                    exit_code,
                    if stderr.is_empty() { stdout } else { stderr }
                ),
                session_id: None,
            });
        }

        Ok(AgentOutput {
            stdout,
            stderr,
            exit_code,
            metadata: None,
        })
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
        "pi-stdio"
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
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        }
    }

    #[test]
    fn execute_successful_command() {
        let runner = PiStdioAgentRunner::new();
        let ctx = make_context("sh", &["-c", "echo hello"]);

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("hello"));
    }

    #[test]
    fn execute_captures_stdout() {
        let runner = PiStdioAgentRunner::new();
        let ctx = make_context("echo", &["test"]);

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(output.stdout, "test\n");
    }

    #[test]
    fn execute_captures_stderr() {
        let runner = PiStdioAgentRunner::new();
        // `cat >/dev/null` consumes stdin so the process stays alive for
        // the write, then we emit to stderr.
        let ctx = make_context("sh", &["-c", "cat >/dev/null; echo err >&2"]);

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(output.stderr.contains("err"));
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn execute_command_not_found() {
        let runner = PiStdioAgentRunner::new();
        let ctx = make_context("/nonexistent/path/does/not/exist", &[]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for missing binary");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::CommandNotFound(_)),
            "expected CommandNotFound, got {err:?}"
        );
    }

    #[test]
    fn execute_nonzero_exit_error() {
        let runner = PiStdioAgentRunner::new();
        // `cat >/dev/null` consumes stdin so the process stays alive for
        // the write, then exits with code 1.
        let ctx = make_context("sh", &["-c", "cat >/dev/null; exit 1"]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for non-zero exit");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::AgentExecutionFailed { .. }),
            "expected AgentExecutionFailed, got {err:?}"
        );
        assert!(err.to_string().contains("exited with code 1"));
    }

    #[test]
    fn execute_timeout() {
        let runner =
            PiStdioAgentRunner::with_timeout(Duration::from_millis(100));
        let ctx = make_context("sleep", &["30"]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for timeout");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
    }

    /// Verify that `PiStdioAgentRunner` writes the full prompt chain
    /// (profile_prompt + prompt + trigger line) to the child process's stdin.
    #[test]
    fn runner_passes_prompt_via_stdin() {
        let runner = PiStdioAgentRunner::new();
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), "cat".to_string()],
            prompt: "hello from knot\n".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(output.stdout, "hello from knot\n", "stdout should contain prompt");
    }

    /// Verify that the full prompt content round-trips through stdin
    /// using `cat /dev/stdin` (alternative way to read stdin).
    #[test]
    fn runner_passes_strand_via_at_syntax() {
        let runner = PiStdioAgentRunner::new();
        let ctx = ExecutionContext {
            cli_path: "cat".to_string(),
            cli_args: vec![],
            prompt: "strand content from file".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("strand.md")),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(
            output.stdout, "strand content from file",
            "cat should echo stdin content exactly"
        );
    }

    /// Verify that `PiStdioAgentRunner` includes profile prompt,
    /// knot instructions, and trigger line in the correct order.
    #[test]
    fn runner_passes_event_metadata() {
        let runner = PiStdioAgentRunner::new();
        let ctx = ExecutionContext {
            cli_path: "cat".to_string(),
            cli_args: vec![],
            prompt: "Review this file.".to_string(),
            profile_prompt: "You are a reviewer.".to_string(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("doc.md")),
            event_type: "Modified".to_string(),
            knot_name: Some("review".to_string()),
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        // Output should contain profile prompt at the start
        assert!(
            output.stdout.starts_with("You are a reviewer."),
            "output should start with profile prompt: {}",
            output.stdout
        );
        // Knot instructions should follow profile prompt
        assert!(
            output.stdout.contains("Review this file."),
            "output should contain knot instructions: {}",
            output.stdout
        );
        // Trigger line should be present
        assert!(
            output.stdout.contains("**review** triggered by **Modified** on **doc.md**"),
            "output should contain trigger line: {}",
            output.stdout
        );
    }

    /// Context timeout of 50ms overrides the runner's 120s default,
    /// killing a long-running process quickly.
    #[test]
    fn execute_context_timeout_override() {
        let runner =
            PiStdioAgentRunner::with_timeout(Duration::from_secs(120));
        let ctx = ExecutionContext {
            cli_path: "sleep".to_string(),
            cli_args: vec!["30".to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
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
        // Should complete well under 120s (the runner default)
        assert!(
            elapsed < Duration::from_secs(5),
            "should use context timeout, not runner default"
        );
    }

    /// When context has no timeout override, the runner's default is used.
    #[test]
    fn execute_context_timeout_fallback_to_runner_default() {
        let runner =
            PiStdioAgentRunner::with_timeout(Duration::from_millis(50));
        let ctx = ExecutionContext {
            cli_path: "sleep".to_string(),
            cli_args: vec!["30".to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
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
        // Should timeout at runner's 50ms default, not hang
        assert!(
            elapsed < Duration::from_secs(5),
            "should use runner default timeout"
        );
    }

    /// Context timeout can be larger than the runner's default,
    /// allowing longer-running agents.
    #[test]
    fn execute_context_timeout_larger_than_default() {
        let runner =
            PiStdioAgentRunner::with_timeout(Duration::from_millis(50));
        // Context allows 3s — enough for `sh -c 'cat >/dev/null; echo ok'`
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), "echo ok".to_string()],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            knot_name: None,
            timeout: Some(Duration::from_secs(3)),
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(output.stdout.contains("ok"));
    }

    /// Existing timeout test with no context override still passes
    /// (regression guard — runner default is used).
    #[test]
    fn execute_timeout_regression_no_context_override() {
        let runner =
            PiStdioAgentRunner::with_timeout(Duration::from_millis(100));
        let ctx = make_context("sleep", &["30"]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for timeout");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
    }

    /// Verify that `--name` and its value are passed through in CLI args.
    /// Uses `sh -c` to echo the args so we can inspect them in stdout.
    #[test]
    fn runner_passes_name_flag_through_cli_args() {
        let runner = PiStdioAgentRunner::new();
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec![
                "-c".to_string(),
                "cat >/dev/null; echo \"$@\"".to_string(),
                // These become $0, $1, $2, $3 etc. in the shell
                "--".to_string(),
                "--name".to_string(),
                "my-knot triggered by Modified on doc.md".to_string(),
            ],
            prompt: "test prompt".to_string(),
            profile_prompt: String::new(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(
            output.stdout.contains("--name"),
            "stdout should contain --name flag: {}",
            output.stdout
        );
        assert!(
            output.stdout.contains("my-knot triggered by Modified on doc.md"),
            "stdout should contain session title: {}",
            output.stdout
        );
    }
}
