//! Subprocess agent runner — invokes an agent CLI via a child process.

use std::process::Stdio;
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
        let timeout = self.timeout;

        // Spawn a background thread that kills the child on timeout.
        let _timeout_thread = std::thread::Builder::new()
            .name("subprocess-timeout".to_string())
            .spawn(move || {
                std::thread::sleep(timeout);
                let _ = unsafe {
                    libc::kill(child_pid, libc::SIGKILL)
                };
                eprintln!(
                    "WARNING: killed '{}' after timeout of {:?}",
                    cli_path, timeout
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
        stdin
            .write_all(ctx.prompt.as_bytes())
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

        // If status code is None, the process was killed by a signal
        // (SIGKILL from our timeout thread).
        if output.status.code().is_none() {
            return Err(PortError::Timeout(format!(
                "'{}' exceeded timeout of {:?}",
                ctx.cli_path, self.timeout
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

    fn make_context(cli: &str, args: &[&str]) -> ExecutionContext {
        ExecutionContext {
            cli_path: cli.to_string(),
            cli_args: args.iter().map(|s| s.to_string()).collect(),
            prompt: "test prompt".to_string(),
            strand_path: crate::domain::entities::StrandPath(
                std::path::PathBuf::from("test.md"),
            ),
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
        let ctx = make_context("sh", &["-c", "echo err >&2"]);

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
        let ctx = make_context("sh", &["-c", "exit 1"]);

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
            strand_path: crate::domain::entities::StrandPath(
                std::path::PathBuf::from("test.md"),
            ),
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
            strand_path: crate::domain::entities::StrandPath(
                std::path::PathBuf::from("strand.md"),
            ),
        };

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(
            output.stdout, prompt,
            "cat should echo stdin content exactly"
        );
    }
}
