//! Subprocess agent runner — invokes an agent CLI via a child process.

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::application::ports::{
    AgentOutput, AgentRunner, ExecutionContext, PortError,
};

/// Subprocess-backed implementation of [`AgentRunner`].
///
/// Spawns the agent CLI as a child process, captures stdout and stderr,
/// and enforces a configurable timeout.
#[derive(Debug, Clone)]
pub struct SubprocessAgentRunner {
    /// Maximum duration the agent may run before being killed.
    /// Defaults to 120 seconds.
    timeout: Duration,
}

impl Default for SubprocessAgentRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
        }
    }
}

impl SubprocessAgentRunner {
    /// Create a new runner with the default 120-second timeout.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new runner with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Build the prompt with event context block prepended.
    ///
    /// If event context fields are present, a `## Event Context` block
    /// is prepended to the prompt before writing to stdin. This gives
    /// the agent metadata about the strand event being processed.
    fn build_prompt_with_context(ctx: &ExecutionContext) -> String {
        let mut full_prompt = String::new();

        // Prepend event context block if any fields are set
        if !ctx.event_type.is_empty() || !ctx.previous_tie_off.is_empty() {
            full_prompt.push_str("## Event Context\n");
            if !ctx.event_type.is_empty() {
                full_prompt.push_str(&format!("Event: {}\n", ctx.event_type));
            }
            full_prompt.push_str(&format!(
                "Strand: {}\n",
                ctx.strand_path.0.display()
            ));
            if !ctx.previous_tie_off.is_empty() {
                full_prompt.push_str("Previous tie-off:\n");
                full_prompt.push_str(&ctx.previous_tie_off);
                full_prompt.push('\n');
            }
            full_prompt.push('\n');
        }

        full_prompt.push_str(&ctx.prompt);
        full_prompt
    }
}

impl AgentRunner for SubprocessAgentRunner {
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
                return Err(PortError::AgentExecutionFailed(format!(
                    "failed to spawn '{}': {}",
                    ctx.cli_path, e
                )));
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
            .name("subprocess-timeout".to_string())
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
                PortError::AgentExecutionFailed(format!(
                    "failed to spawn timeout thread: {e}"
                ))
            })?;

        // Write the prompt to the child's stdin.
        let mut stdin = child.stdin.take().expect("stdin was piped");
        use std::io::Write;

        // Build event context block to prepend to prompt.
        let prompt_with_context = Self::build_prompt_with_context(&ctx);

        stdin
            .write_all(prompt_with_context.as_bytes())
            .map_err(|e| {
                PortError::AgentExecutionFailed(format!(
                    "failed to write prompt to stdin: {e}"
                ))
            })?;
        // Drop stdin to close the pipe (signals EOF to the child).
        drop(stdin);

        // Wait for the child and capture output.
        let output = child.wait_with_output().map_err(|e| {
            PortError::AgentExecutionFailed(format!(
                "failed to wait for '{}': {}",
                ctx.cli_path, e
            ))
        })?;

        // Mark cancelled so the background timeout thread suppresses its
        // warning if the child exited before the deadline.
        cancelled.store(true, Ordering::Relaxed);

        // If status code is None, the process was killed by a signal
        // (SIGKILL from our timeout thread).
        if output.status.code().is_none() {
            return Err(PortError::Timeout(format!(
                "'{}' exceeded timeout of {:?} (strand: {})",
                ctx.cli_path, effective_timeout, strand_desc
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let exit_code = output.status.code().unwrap_or(-1);

        if exit_code != 0 {
            return Err(PortError::AgentExecutionFailed(format!(
                "'{}' exited with code {}: {}",
                ctx.cli_path,
                exit_code,
                if stderr.is_empty() { stdout } else { stderr }
            )));
        }

        Ok(AgentOutput {
            stdout,
            stderr,
            exit_code,
        })
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
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            previous_tie_off: String::new(),
            timeout: None,
        }
    }

    #[test]
    fn execute_successful_command() {
        let runner = SubprocessAgentRunner::new();
        let ctx = make_context("sh", &["-c", "echo hello"]);

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("hello"));
    }

    #[test]
    fn execute_captures_stdout() {
        let runner = SubprocessAgentRunner::new();
        let ctx = make_context("echo", &["test"]);

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(output.stdout, "test\n");
    }

    #[test]
    fn execute_captures_stderr() {
        let runner = SubprocessAgentRunner::new();
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
        let runner = SubprocessAgentRunner::new();
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
        let runner = SubprocessAgentRunner::new();
        // `cat >/dev/null` consumes stdin so the process stays alive for
        // the write, then exits with code 1.
        let ctx = make_context("sh", &["-c", "cat >/dev/null; exit 1"]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for non-zero exit");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::AgentExecutionFailed(_)),
            "expected AgentExecutionFailed, got {err:?}"
        );
        assert!(err.to_string().contains("exited with code 1"));
    }

    #[test]
    fn execute_timeout() {
        let runner =
            SubprocessAgentRunner::with_timeout(Duration::from_millis(100));
        let ctx = make_context("sleep", &["30"]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for timeout");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout(_)),
            "expected Timeout, got {err:?}"
        );
    }

    /// Verify that `SubprocessAgentRunner` writes `ctx.prompt` to the child
    /// process's stdin. Use `sh -c cat` which reads stdin and writes to stdout.
    #[test]
    fn runner_passes_prompt_via_stdin() {
        let runner = SubprocessAgentRunner::new();
        let prompt = "hello from knot\n";
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), "cat".to_string()],
            prompt: prompt.to_string(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            previous_tie_off: String::new(),
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(output.stdout, prompt, "stdout should contain prompt");
    }

    /// Verify that the full prompt content round-trips through stdin
    /// using `cat /dev/stdin` (alternative way to read stdin).
    #[test]
    fn runner_passes_strand_via_at_syntax() {
        let runner = SubprocessAgentRunner::new();
        let prompt = "strand content from file";
        let ctx = ExecutionContext {
            cli_path: "cat".to_string(),
            cli_args: vec![],
            prompt: prompt.to_string(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("strand.md")),
            event_type: String::new(),
            previous_tie_off: String::new(),
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(
            output.stdout, prompt,
            "cat should echo stdin content exactly"
        );
    }

    /// Verify that `SubprocessAgentRunner` prepends event context to the
    /// prompt when `event_type` or `previous_tie_off` are set.
    #[test]
    fn runner_passes_event_metadata() {
        let runner = SubprocessAgentRunner::new();
        let ctx = ExecutionContext {
            cli_path: "cat".to_string(),
            cli_args: vec![],
            prompt: "Review this file.".to_string(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("doc.md")),
            event_type: "Modified".to_string(),
            previous_tie_off: "Previous review done.".to_string(),
            timeout: None,
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        // Output should contain the event context block
        assert!(
            output.stdout.contains("## Event Context"),
            "output should contain event context header: {}",
            output.stdout
        );
        assert!(
            output.stdout.contains("Event: Modified"),
            "output should contain event type: {}",
            output.stdout
        );
        assert!(
            output.stdout.contains("Strand: doc.md"),
            "output should contain strand path: {}",
            output.stdout
        );
        assert!(
            output.stdout.contains("Previous tie-off:"),
            "output should contain previous tie-off header: {}",
            output.stdout
        );
        assert!(
            output.stdout.contains("Previous review done."),
            "output should contain previous tie-off content: {}",
            output.stdout
        );
        // Original prompt should still be present
        assert!(
            output.stdout.contains("Review this file."),
            "output should still contain original prompt: {}",
            output.stdout
        );
    }

    /// Context timeout of 50ms overrides the runner's 120s default,
    /// killing a long-running process quickly.
    #[test]
    fn execute_context_timeout_override() {
        let runner =
            SubprocessAgentRunner::with_timeout(Duration::from_secs(120));
        let ctx = ExecutionContext {
            cli_path: "sleep".to_string(),
            cli_args: vec!["30".to_string()],
            prompt: "test prompt".to_string(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            previous_tie_off: String::new(),
            timeout: Some(Duration::from_millis(50)),
        };

        let start = std::time::Instant::now();
        let result = runner.execute(ctx);
        let elapsed = start.elapsed();

        assert!(result.is_err(), "should error for timeout");
        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout(_)),
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
            SubprocessAgentRunner::with_timeout(Duration::from_millis(50));
        let ctx = ExecutionContext {
            cli_path: "sleep".to_string(),
            cli_args: vec!["30".to_string()],
            prompt: "test prompt".to_string(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            previous_tie_off: String::new(),
            timeout: None,
        };

        let start = std::time::Instant::now();
        let result = runner.execute(ctx);
        let elapsed = start.elapsed();

        assert!(result.is_err(), "should error for timeout");
        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout(_)),
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
            SubprocessAgentRunner::with_timeout(Duration::from_millis(50));
        // Context allows 3s — enough for `sh -c 'cat >/dev/null; echo ok'`
        let ctx = ExecutionContext {
            cli_path: "sh".to_string(),
            cli_args: vec!["-c".to_string(), "echo ok".to_string()],
            prompt: "test prompt".to_string(),
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            previous_tie_off: String::new(),
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
            SubprocessAgentRunner::with_timeout(Duration::from_millis(100));
        let ctx = make_context("sleep", &["30"]);

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for timeout");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout(_)),
            "expected Timeout, got {err:?}"
        );
    }
}
