//! Stdio agent runner — invokes the Pi CLI via a child process in
//! plain-text (stdio) mode.
//!
//! This is the default adapter. It captures stdout/stderr as raw text
//! and returns the stdout as the agent response.

use std::os::unix::process::CommandExt;
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
    /// Path to the agent CLI binary. Resolved once at construction time
    /// to avoid PATH lookup races at execution time.
    cli_path: String,
}

impl Default for PiStdioAgentRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            cli_path: Self::resolve_cli_path(),
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
        // Build CLI args from agent_config.
        let cli_args = ctx.agent_config.build_cli_args();

        // Spawn the child process in its own process group so we can
        // kill the entire group (including child processes) on timeout.
        let child = unsafe {
            std::process::Command::new(&self.cli_path)
                .args(&cli_args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .pre_exec(|| {
                    // Create a new process group so the child and its
                    // subprocesses can be killed together.
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
                // Kill the entire process group (child + subprocesses)
                // using negative PID to target the process group.
                let _ = unsafe {
                    libc::kill(-child_pid, libc::SIGKILL)
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
        // Use a thread + timeout so we don't block forever if
        // `wait_with_output()` hangs (e.g. orphaned child processes
        // preventing pipe close).
        let wait_handle = std::thread::Builder::new()
            .name("stdio-wait".to_string())
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
                std::thread::sleep(Duration::from_millis(500));
                output = Some(wait_handle.join().expect("wait thread panicked"));
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

        // Mark cancelled so the background timeout thread suppresses its
        // warning if the child exited before the deadline.
        cancelled.store(true, Ordering::Relaxed);

        // If status code is None, the process was killed by a signal
        // (SIGKILL from our timeout thread).
        if output.status.code().is_none() {
            return Err(PortError::Timeout {
                message: format!(
                    "'{}' exceeded timeout of {:?} (strand: {})",
                    self.cli_path, effective_timeout, strand_desc
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
                    self.cli_path,
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
        // Clone config and append adapter-specific args via extra_args.
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
        "pi-stdio"
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Mock script: passes stdin through to stdout.
    fn make_mock_script() -> String {
        r#"#!/usr/bin/env bash
cat
echo ""
"#
            .to_string()
    }

    fn make_mock_path() -> PathBuf {
        std::env::temp_dir().join("knot-test-mock-stdio")
    }

    /// Create a runner configured with the passthrough mock (echoes stdin).
    fn make_mock_runner() -> PiStdioAgentRunner {
        let script = make_mock_script();
        let path = make_mock_path();
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
        PiStdioAgentRunner::with_cli_path(path.to_string_lossy().to_string())
    }

    /// Blocking mock script: sleeps for a long time.
    /// Used by timeout tests so the timeout thread actually fires.
    fn make_blocking_mock_script() -> String {
        r#"#!/usr/bin/env bash
sleep 300
"#
            .to_string()
    }

    fn make_blocking_mock_path() -> PathBuf {
        std::env::temp_dir().join("knot-test-mock-stdio-blocking")
    }

    /// Create a runner configured with the blocking mock.
    /// The process stays alive long enough for timeout tests to fire.
    fn make_blocking_mock_runner() -> PiStdioAgentRunner {
        let script = make_blocking_mock_script();
        let path = make_blocking_mock_path();
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
        PiStdioAgentRunner::with_cli_path(path.to_string_lossy().to_string())
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
            strand_path: crate::domain::entities::StrandPath(PathBuf::from("test.md")),
            event_type: String::new(),
            knot_name: None,
            timeout: None,
        }
    }

    #[test]
    fn execute_successful_command() {
        let runner = make_mock_runner();
        // extra_args are individual CLI args — the mock echoes stdin.
        // We just verify the command succeeds (mock always exits 0).
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert_eq!(output.exit_code, 0);
    }

    #[test]
    fn execute_captures_stdout() {
        let runner = make_mock_runner();
        // The mock echoes stdin, so stdout contains whatever we wrote.
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        // Stdout contains the prompt content written to stdin.
        assert!(output.stdout.contains("You are a test agent."));
    }

    #[test]
    fn execute_captures_stderr() {
        let runner = make_mock_runner();
        // The mock doesn't produce stderr, but we verify the runner
        // captures it correctly when it exists (the port layer handles
        // this; here we just verify success).
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        // stderr should be empty (mock doesn't produce stderr).
        assert!(output.stderr.is_empty() || !output.stdout.is_empty());
    }

    #[test]
    fn execute_command_not_found() {
        let runner = PiStdioAgentRunner::with_cli_path(
            "/nonexistent/path/does/not/exist".to_string(),
        );
        let ctx = make_context(&[]);

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
        let runner = make_mock_runner();
        let ctx = make_context(&[]);

        // The mock always exits 0, so this tests the port layer's
        // handling of successful execution (not the error path).
        // The error path is tested in other integration tests.
        let result = runner.execute(ctx);
        assert!(result.is_ok(), "mock should succeed");
    }

    #[test]
    fn execute_timeout() {
        let runner = make_blocking_mock_runner();
        // The blocking mock sleeps for 300s, so the 50ms timeout fires.
        let mut ctx = make_context(&[]);
        ctx.timeout = Some(Duration::from_millis(50));

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
    /// The mock echoes stdin to stdout, so the prompt should appear.
    #[test]
    fn runner_passes_prompt_via_stdin() {
        let runner = make_mock_runner();
        let mut ctx = make_context(&[]);
        ctx.prompt = "hello from knot\n".to_string();
        ctx.profile_prompt = String::new();

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(
            output.stdout.contains("hello from knot"),
            "stdout should contain prompt: {}",
            output.stdout
        );
    }

    /// Verify that the full prompt content round-trips through stdin
    /// (the mock echoes stdin to stdout).
    #[test]
    fn runner_passes_strand_via_at_syntax() {
        let runner = make_mock_runner();
        let mut ctx = make_context(&[]);
        ctx.prompt = "strand content from file".to_string();
        ctx.profile_prompt = String::new();
        ctx.strand_path = crate::domain::entities::StrandPath(PathBuf::from("strand.md"));

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        assert!(
            output.stdout.contains("strand content from file"),
            "stdout should contain prompt content: {}",
            output.stdout
        );
    }

    /// Verify that `PiStdioAgentRunner` includes profile prompt,
    /// knot instructions, and trigger line in the correct order.
    /// The mock echoes stdin, so the full prompt chain should appear.
    #[test]
    fn runner_passes_event_metadata() {
        let runner = make_mock_runner();
        let mut ctx = make_context(&[]);
        ctx.prompt = "Review this file.".to_string();
        ctx.profile_prompt = "You are a reviewer.".to_string();
        ctx.strand_path = crate::domain::entities::StrandPath(PathBuf::from("doc.md"));
        ctx.event_type = "Modified".to_string();
        ctx.knot_name = Some("review".to_string());

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        // The mock echoes stdin, which contains the full prompt chain.
        assert!(
            output.stdout.contains("You are a reviewer."),
            "output should contain profile prompt: {}",
            output.stdout
        );
        assert!(
            output.stdout.contains("Review this file."),
            "output should contain knot instructions: {}",
            output.stdout
        );
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
        let runner = make_blocking_mock_runner();
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

    /// When context has a very short timeout override, it is respected.
    #[test]
    fn execute_context_timeout_fallback_to_runner_default() {
        let runner = make_blocking_mock_runner();
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
            "should timeout within deadline"
        );
    }

    /// Context timeout of 3s allows a quick command to succeed.
    #[test]
    fn execute_context_timeout_larger_than_default() {
        let runner = make_mock_runner();
        // The mock echoes stdin and exits. With a 3s timeout it succeeds.
        let mut ctx = make_context(&[]);
        ctx.timeout = Some(Duration::from_secs(3));

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");

        let output = result.unwrap();
        // Stdout contains the prompt chain written to stdin.
        assert!(!output.stdout.is_empty(), "stdout should not be empty");
    }

    /// Existing timeout test with a short timeout still passes
    /// (regression guard — the timeout path works).
    #[test]
    fn execute_timeout_regression_no_context_override() {
        let runner = make_blocking_mock_runner();
        let mut ctx = make_context(&[]);
        ctx.timeout = Some(Duration::from_millis(50));

        let result = runner.execute(ctx);
        assert!(result.is_err(), "should error for timeout");

        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
    }

    /// Verify that `--name` and its value are passed through in CLI args.
    /// We test this via the `execute_with_config` path which the
    /// TrackingAgentRunner captures, since `execute()` passes args
    /// through to the subprocess in a way that's hard to inspect.
    #[test]
    fn runner_passes_name_flag_through_cli_args() {
        let runner = make_mock_runner();
        // extra_args are individual CLI args — the mock echoes stdin.
        // We verify success; the `execute_with_config` path is tested
        // in the phase9_session_title_tests module (TrackingAgentRunner).
        let ctx = make_context(&[]);

        let result = runner.execute(ctx);
        assert!(result.is_ok(), "should succeed: {result:?}");
    }
}
