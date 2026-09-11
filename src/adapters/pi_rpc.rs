//! RPC agent runner — invokes the Pi CLI with `--mode rpc` and drives the
//! session over the JSON command/response protocol on stdin/stdout
//! (plan 084).
//!
//! Unlike [`pi_json`](super::pi_json), the prompt is **not** written to
//! stdin as raw text; it is sent as a JSON `prompt` command over the RPC
//! protocol. This lets Knot steer the *running* session mid-turn: when the
//! session's context usage crosses the alias's `ctx-wrap-up-limit`, the
//! driver queues a `steer` command asking the agent to wrap up gracefully
//! (commit the work, update progress notes, record a final tie-off) **before**
//! pi's own compaction rewrites the transcript.
//!
//! ## Protocol
//!
//! - **Startup**: send `get_state` (capture the session id + whether pi's
//!   auto-compaction is enabled), then send the `prompt` command.
//! - **Context sampling**: on each `turn_end` the driver sends
//!   `get_session_stats`; the response's `data.contextUsage.tokens` is the
//!   live context-usage estimate (the same value pi uses for its footer and
//!   compaction decision). `contextUsage` is omitted (or `tokens` is `null`)
//!   when no valid usage is available — those samples are skipped.
//! - **Steer (fire-once)**: the first sample with `tokens >= limit` (while
//!   compaction is enabled) queues a single `steer` command and records a
//!   [`WrapUpRecord`] for the metadata / `ContextWrapUpSteered` event.
//! - **Teardown**: on `agent_end` the driver closes stdin; the main thread
//!   waits a bounded grace for the process to exit and force-kills the
//!   process group otherwise. A SIGKILL during teardown *after* `agent_end`
//!   is a normal exit, not a timeout.
//!
//! ## Threading
//!
//! A dedicated **driver thread** owns the child's stdin and consumes the
//! stdout line channel (fed by [`spawn_line_reader`]); it is the only writer
//! of RPC commands, so there is no stdin lock. The stdout reader, stderr
//! reader, watchdog, and wait threads are the same primitives the JSON
//! runner uses. Output assembly reuses [`pi_json::PiJsonAgentRunner::parse_stdout`]
//! over the accumulated buffer — the RPC event stream is a superset of the
//! JSON stream (`agent_end` still carries the full `messages` array and
//! `compaction_end` events), with token usage and the session id taken from
//! the `get_session_stats` / `get_state` responses instead.

use std::io::{BufWriter, Write};
use std::os::unix::process::CommandExt;
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::adapters::live_output::{
    spawn_line_reader, spawn_reader, spawn_watchdog, KillReason, LiveOutput,
};
use crate::adapters::pi_json::{
    observe_compaction_line, CompactionObserveState, PiJsonAgentRunner,
};
use crate::application::ports::{
    AgentInvocationMetadata, AgentOutput, AgentRunner, CompactionObservation,
    ExecutionContext, PortError, TokenUsage, WrapUpRecord,
};
use crate::domain::entities::StrandPath;
use crate::domain::value_objects::AgentConfig;

/// The steering message queued when the context crosses the wrap-up limit.
///
/// Plan 086: the water-mark handoff note (replacing 084's terminal
/// `WRAP_UP_STEER` wind-down). Reuses the greppable const from
/// `value_objects` so the text is single-sourced.
const WRAP_UP_STEER: &str = crate::domain::value_objects::HANDOFF_NOTE;

/// Bounded grace after `agent_end` for the process to exit on its own
/// (stdin is closed at `agent_end`); beyond this the group is force-killed.
const TEARDOWN_GRACE: Duration = Duration::from_secs(5);

/// pi's default `reserveTokens` (tokens of context window held back from
/// usable context; auto-compaction fires at `contextWindow - reserveTokens`).
/// Knot cannot read pi's configured reserve reliably, so it uses this
/// default only for the once-per-run "lower ctx-wrap-up-limit or raise
/// reserveTokens" warning — never to gate the steer.
const PI_DEFAULT_RESERVE_TOKENS: u64 = 16384;

/// Shared, mutable state the driver thread writes and the main thread reads
/// after the process exits.
#[derive(Debug, Default)]
struct RpcShared {
    /// Session id from the `get_state` response (authoritative in RPC mode —
    /// the RPC stream does not emit a `session` header event).
    session_id: Option<String>,
    /// Token usage from the most recent `get_session_stats` response.
    token_usage: Option<TokenUsage>,
    /// `get_state`'s `autoCompactionEnabled` (whether pi's own
    /// compact-and-retry is active); `None` if not yet observed.
    compaction_enabled: Option<bool>,
    /// The context wrap-up steer, if the driver fired it (fire-once).
    wrap_up: Option<WrapUpRecord>,
}

/// RPC implementation of [`AgentRunner`].
///
/// Appends `--mode rpc` to the shared CLI args (via
/// [`AgentConfig::build_cli_args_core`], which omits the JSON-mode `-p`
/// print prefix) and drives the session over the stdin/stdout JSON command
/// protocol.
#[derive(Debug, Clone)]
pub struct PiRpcAgentRunner {
    /// Maximum duration the agent may run before being killed.
    /// Defaults to 120 seconds.
    timeout: Duration,
    /// Kill the session when it produces no output for this window.
    /// `None` disables the inactivity watchdog.
    inactivity_timeout: Option<Duration>,
    /// Path to the agent CLI binary, resolved once at construction.
    cli_path: String,
}

impl Default for PiRpcAgentRunner {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            inactivity_timeout: None,
            cli_path: Self::resolve_cli_path(),
        }
    }
}

impl PiRpcAgentRunner {
    /// Create a new runner with the default 120-second timeout.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new runner with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            inactivity_timeout: None,
            cli_path: Self::resolve_cli_path(),
        }
    }

    /// Create a new runner with a custom total timeout and inactivity
    /// window.
    pub fn with_timeouts(timeout: Duration, inactivity_timeout: Option<Duration>) -> Self {
        Self {
            timeout,
            inactivity_timeout,
            cli_path: Self::resolve_cli_path(),
        }
    }

    /// Create a new runner with a specific CLI path (integration tests).
    #[cfg(test)]
    pub fn with_cli_path(cli_path: String) -> Self {
        Self {
            timeout: Duration::from_secs(120),
            inactivity_timeout: None,
            cli_path,
        }
    }

    /// Create a new runner with an explicit CLI path and timeout.
    pub fn with_cli_path_and_timeout(cli_path: String, timeout: Duration) -> Self {
        Self {
            timeout,
            inactivity_timeout: None,
            cli_path,
        }
    }

    /// Create a new runner with an explicit CLI path, total timeout, and
    /// inactivity window (composition root).
    pub fn with_cli_path_and_timeouts(
        cli_path: String,
        timeout: Duration,
        inactivity_timeout: Option<Duration>,
    ) -> Self {
        Self {
            timeout,
            inactivity_timeout,
            cli_path,
        }
    }

    /// Resolve the CLI path: `KNOT_TEST_CLI_PATH` (set by integration test
    /// helpers) or `pi` from PATH.
    fn resolve_cli_path() -> String {
        std::env::var("KNOT_TEST_CLI_PATH").unwrap_or_else(|_| "pi".to_string())
    }

    /// Build the RPC CLI args from the shared core (no `-p`) plus `--mode rpc`.
    fn build_rpc_cli_args(agent_config: &AgentConfig) -> Vec<String> {
        let mut args = agent_config.build_cli_args_core();
        args.push("--mode".to_string());
        args.push("rpc".to_string());
        args
    }

    /// The `execute` body with an optional live compaction observer
    /// (plan 088). `execute` delegates here with `None`;
    /// `execute_with_config_and_observer` passes the usecase's observer.
    /// The driver's line loop feeds the shared observation helper — the
    /// session id is seeded from the `get_state` response (the RPC
    /// stream has no `session` header event).
    fn execute_inner(
        &self,
        ctx: ExecutionContext,
        observer: Option<Arc<dyn Fn(&CompactionObservation) + Send + Sync>>,
    ) -> Result<AgentOutput, PortError> {
        let cli_args = Self::build_rpc_cli_args(&ctx.agent_config);
        let cli_path = self.cli_path.clone();

        // The optional wrap-up limit (plan 084). `None` → no sampling or
        // steering; the driver still captures the session id via
        // `get_state`.
        let limit = ctx.agent_config.ctx_wrap_up_limit;

        // Spawn the child in its own process group (kill the group on
        // timeout).
        let child = unsafe {
            std::process::Command::new(&cli_path)
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
                    cli_path, e
                )));
            }
            Err(e) => {
                return Err(PortError::AgentExecutionFailed {
                    message: format!("failed to spawn '{}': {}", cli_path, e),
                    session_id: None,
                });
            }
        };

        let child_pid = child.id() as i32;
        let strand_desc = ctx.strand_path.0.display().to_string();
        let effective_timeout = ctx.timeout.unwrap_or(self.timeout);

        // Shared liveness state (drained buffers, last activity, kill reason).
        let live = LiveOutput::new();
        // Set once the child has exited so the watchdog suppresses its kill.
        let cancelled = Arc::new(AtomicBool::new(false));

        // Line channel: the stdout reader splits complete lines and feeds
        // them to the driver; the raw buffer is still the source of truth
        // for the watchdog and the post-hoc parse.
        let (line_tx, line_rx) = mpsc::channel::<String>();

        let stdout_reader = spawn_line_reader(
            "rpc-stdout",
            child.stdout.take().expect("stdout was piped"),
            &live.stdout,
            &live.last_activity,
            line_tx,
        )
        .map_err(|e| {
            PortError::AgentExecutionFailed {
                message: format!("failed to spawn stdout reader: {e}"),
                session_id: None,
            }
        })?;
        let stderr_reader = spawn_reader(
            "rpc-stderr",
            child.stderr.take().expect("stderr was piped"),
            &live.stderr,
            &live.last_activity,
        )
        .map_err(|e| {
            PortError::AgentExecutionFailed {
                message: format!("failed to spawn stderr reader: {e}"),
                session_id: None,
            }
        })?;

        // Watchdog: inactivity window first, then the total budget.
        let _watchdog = spawn_watchdog(
            "rpc-watchdog",
            child_pid,
            cli_path.clone(),
            strand_desc.clone(),
            effective_timeout,
            self.inactivity_timeout,
            &live,
            Arc::clone(&cancelled),
        )
        .map_err(|e| {
            PortError::AgentExecutionFailed {
                message: format!("failed to spawn watchdog thread: {e}"),
                session_id: None,
            }
        })?;

        // The driver owns the child's stdin and the line receiver.
        let stdin = BufWriter::new(child.stdin.take().expect("stdin was piped"));
        let profile_prompt = ctx.profile_prompt.clone();
        let prompt_message =
            PiJsonAgentRunner::build_prompt_with_context(&ctx, &profile_prompt);
        let shared = Arc::new(Mutex::new(RpcShared::default()));
        let agent_end_seen = Arc::new(AtomicBool::new(false));
        // Plan 088: shared session-id state for live compaction
        // observation (seeded from the `get_state` response).
        let observe_state = Arc::new(CompactionObserveState::new());

        let driver: JoinHandle<()> = thread::Builder::new()
            .name("rpc-driver".to_string())
            .spawn({
                let shared = Arc::clone(&shared);
                let agent_end_seen = Arc::clone(&agent_end_seen);
                let observer = observer.clone();
                let observe_state = Arc::clone(&observe_state);
                move || {
                    run_rpc_driver(
                        stdin,
                        line_rx,
                        prompt_message,
                        limit,
                        shared,
                        agent_end_seen,
                        observer,
                        observe_state,
                    );
                }
            })
            .expect("failed to spawn driver thread");

        // Wait thread: get the exit status.
        let wait_handle: JoinHandle<std::io::Result<std::process::ExitStatus>> =
            thread::Builder::new()
                .name("rpc-wait".to_string())
                .spawn(move || child.wait())
                .expect("failed to spawn wait thread");

        // Teardown: exit early on `agent_end` (giving the grace window for a
        // clean exit), otherwise bound by the total budget.
        let start = Instant::now();
        let mut agent_end_at: Option<Instant> = None;
        loop {
            if agent_end_seen.load(Ordering::Relaxed) {
                if agent_end_at.is_none() {
                    agent_end_at = Some(Instant::now());
                }
                if wait_handle.is_finished() {
                    break;
                }
                if agent_end_at.is_some_and(|t| t.elapsed() >= TEARDOWN_GRACE) {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
                continue;
            }
            if wait_handle.is_finished() || start.elapsed() >= effective_timeout {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        // If the process is still alive, force-kill the whole group.
        if !wait_handle.is_finished() {
            let _ = unsafe { libc::kill(-child_pid, libc::SIGKILL) };
            for _ in 0..100 {
                if wait_handle.is_finished() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
        let status = wait_handle
            .join()
            .expect("wait thread panicked")
            .map_err(|e| {
                PortError::AgentExecutionFailed {
                    message: format!("failed to wait for '{cli_path}': {e}"),
                    session_id: None,
                }
            })?;

        // The child exited — stop the watchdog.
        cancelled.store(true, Ordering::Relaxed);

        // Drain the readers to EOF (bounded), then join the driver (it exits
        // when the line channel disconnects).
        let drain_start = Instant::now();
        while (
            !stdout_reader.is_finished() || !stderr_reader.is_finished()
        ) && drain_start.elapsed() < Duration::from_secs(5)
        {
            thread::sleep(Duration::from_millis(20));
        }
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        let _ = driver.join();

        let stdout_bytes = live.stdout.lock().expect("stdout buffer poisoned").clone();
        let stderr_bytes = live.stderr.lock().expect("stderr buffer poisoned").clone();
        let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();
        let raw_stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();

        let saw_agent_end = agent_end_seen.load(Ordering::Relaxed);

        // Parse the accumulated buffer (response text, compactions, error
        // message, and — as a fallback — any session id). Token usage and the
        // session id prefer the driver's `get_session_stats` / `get_state`
        // values (RPC `agent_end` carries no `usage` and there is no
        // `session` header event).
        let (
            had_parse_error,
            parsed_session,
            response_text,
            parsed_usage,
            compactions,
            compaction_starts,
            error_message,
        ) = PiJsonAgentRunner::parse_stdout(&raw_stdout);
        let (driver_session, driver_usage, driver_wrap_up) = {
            let s = shared.lock().expect("shared state poisoned");
            (s.session_id.clone(), s.token_usage, s.wrap_up.clone())
        };
        let session_id = driver_session.or(parsed_session);
        let token_usage = driver_usage.or(parsed_usage);

        // Success path: the agent emitted `agent_end` (or exited cleanly).
        if saw_agent_end || status.code() == Some(0) {
            if raw_stdout.trim().is_empty() {
                return Ok(AgentOutput {
                    stdout: String::new(),
                    stderr,
                    exit_code: status.code().unwrap_or(-1),
                    metadata: None,
                });
            }
            if had_parse_error {
                // Graceful degradation — treat as plain text.
                return Ok(AgentOutput {
                    stdout: raw_stdout,
                    stderr,
                    exit_code: status.code().unwrap_or(-1),
                    metadata: None,
                });
            }
            if response_text.trim().is_empty() {
                if let Some(rec) = PiJsonAgentRunner::terminal_overflow(&compactions) {
                    return Err(PortError::ContextLimitReached {
                        message: rec
                            .error
                            .clone()
                            .unwrap_or_else(|| {
                                "session context cannot fit the model window even after compaction"
                                    .to_string()
                            }),
                        session_id,
                    });
                }
                if error_message
                    .as_deref()
                    .is_some_and(PiJsonAgentRunner::is_context_overflow_message)
                {
                    let err_msg = error_message.as_deref().unwrap_or_default();
                    return Err(PortError::ContextLimitReached {
                        message: format!(
                            "context overflow, but pi auto-compaction did \n                             not run ({err_msg}). Enable compaction for \n                             rig sessions with a project-level \n                             .pi/settings.json: \n                             {{\"compaction\": {{\"enabled\": true}}}}"
                        ),
                        session_id,
                    });
                }
            }
            return Ok(AgentOutput {
                stdout: response_text,
                stderr,
                exit_code: status.code().unwrap_or(-1),
                metadata: Some(AgentInvocationMetadata {
                    session_id,
                    token_usage,
                    compactions,
                    compaction_starts,
                    wrap_up: driver_wrap_up,
                }),
            });
        }

        // Failed path: the process ended without `agent_end`.
        match status.code() {
            Some(code) => Err(PortError::AgentExecutionFailed {
                message: format!(
                    "'{}' exited with code {}: {}",
                    cli_path,
                    code,
                    if stderr.is_empty() {
                        raw_stdout.clone()
                    } else {
                        stderr.clone()
                    }
                ),
                session_id,
            }),
            None => {
                // Killed by a signal. A teardown kill after `agent_end` is a
                // normal exit (handled above); here it means a watchdog or
                // external kill with no clean completion.
                let reason = *live.kill_reason.lock().expect("kill reason poisoned");
                match reason {
                    Some(KillReason::Inactivity) => {
                        let blocked_call =
                            PiJsonAgentRunner::parse_blocked_call(&raw_stdout);
                        let silent_secs = live.silence().as_secs();
                        let window_secs = self
                            .inactivity_timeout
                            .map(|w| w.as_secs())
                            .unwrap_or(0);
                        let blocked_part = blocked_call
                            .as_deref()
                            .map(|b| format!(", last call: {b}"))
                            .unwrap_or_default();
                        Err(PortError::AgentInactivity {
                            message: format!(
                                "no output for {silent_secs}s (inactivity window {window_secs}s){blocked_part} ({cli_path}, strand: {strand_desc})"
                            ),
                            silent_secs,
                            window_secs,
                            blocked_call,
                            session_id,
                        })
                    }
                    Some(KillReason::Total) | None => Err(PortError::Timeout {
                        message: format!(
                            "'{}' exceeded timeout of {:?} (strand: {})",
                            cli_path, effective_timeout, strand_desc
                        ),
                        session_id,
                    }),
                }
            }
        }
    }

}

/// Write one JSON line to the RPC stdin writer and flush it.
///
/// Returns `false` on a write/flush error (broken pipe — the child is
/// dying); callers treat a failed send as a no-op rather than an error,
/// because once the process is terminating there is nothing left to do.
fn send_rpc_line(stdin: &mut Option<BufWriter<ChildStdin>>, line: &str) -> bool {
    match stdin.as_mut() {
        Some(w) => match writeln!(w, "{line}") {
            Ok(()) => w.flush().is_ok(),
            Err(_) => false,
        },
        None => false,
    }
}

impl AgentRunner for PiRpcAgentRunner {
    fn execute(&self, ctx: ExecutionContext) -> Result<AgentOutput, PortError> {
        self.execute_inner(ctx, None)
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
        // The session title is a CLI-level arg (supported in RPC mode). The
        // strand content is delivered via the stdin prompt, not a `@file`
        // CLI arg — see below.
        let mut config = agent_config.clone();
        let strand_filename = strand_path
            .0
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let session_title = format!(
            "{} triggered by {} on {}",
            knot_name.as_deref().unwrap_or("unknown"),
            event_type,
            strand_filename
        );
        config.extra_args.push("--name".to_string());
        config.extra_args.push(session_title);
        // RPC mode rejects `@file` CLI args (pi: "Error: @file arguments
        // are not supported in RPC mode"), so the strand content is
        // delivered through the stdin prompt instead of as a CLI arg —
        // mirroring the effect the JSON/stdio runners get from pi's
        // `@file` expansion.
        let mut prompt = prompt;
        if let Some(ref file_path) = strand_file_ref {
            match std::fs::read_to_string(&file_path.0) {
                Ok(content) => {
                    prompt.push_str("\n\n");
                    prompt.push_str(&content);
                }
                Err(err) => {
                    eprintln!(
                        "WARNING: pi-rpc could not read strand file {} for prompt injection: {err}",
                        file_path.0.display()
                    );
                }
            }
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

    /// Plan 088: the observer variant — same CLI-arg and prompt
    /// injection as [`Self::execute_with_config`], but the compaction
    /// observer runs live on the driver's line stream.
    fn execute_with_config_and_observer(
        &self,
        agent_config: &AgentConfig,
        strand_path: StrandPath,
        strand_file_ref: Option<StrandPath>,
        prompt: String,
        profile_prompt: String,
        event_type: String,
        knot_name: Option<String>,
        timeout: Option<Duration>,
        observer: Option<Arc<dyn Fn(&CompactionObservation) + Send + Sync>>,
    ) -> Result<AgentOutput, PortError> {
        let mut config = agent_config.clone();
        let strand_filename = strand_path
            .0
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let session_title = format!(
            "{} triggered by {} on {}",
            knot_name.as_deref().unwrap_or("unknown"),
            event_type,
            strand_filename
        );
        config.extra_args.push("--name".to_string());
        config.extra_args.push(session_title);
        let mut prompt = prompt;
        if let Some(ref file_path) = strand_file_ref {
            match std::fs::read_to_string(&file_path.0) {
                Ok(content) => {
                    prompt.push_str("\n\n");
                    prompt.push_str(&content);
                }
                Err(err) => {
                    eprintln!(
                        "WARNING: pi-rpc could not read strand file {} for prompt injection: {err}",
                        file_path.0.display()
                    );
                }
            }
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
        self.execute_inner(ctx, observer)
    }

    fn runner_type(&self) -> &str {
        "pi-rpc"
    }
}

/// The RPC session driver: owns the stdin writer and the line receiver,
/// sends `get_state` + `prompt`, then samples context on `turn_end` and
/// fires the wrap-up steer once the limit is crossed.
///
/// On `agent_end` the driver **closes stdin** (dropping the writer) to
/// signal pi to exit, but keeps draining the channel so a `get_session_stats`
/// response that lands after `agent_end` is still processed (it may carry
/// the sample that trips the steer). The driver returns when the line
/// channel disconnects (the stdout reader hit EOF — the process has exited).
fn run_rpc_driver(
    stdin: BufWriter<ChildStdin>,
    line_rx: mpsc::Receiver<String>,
    prompt_message: String,
    limit: Option<u64>,
    shared: Arc<Mutex<RpcShared>>,
    agent_end_seen: Arc<AtomicBool>,
    observer: Option<Arc<dyn Fn(&CompactionObservation) + Send + Sync>>,
    observe_state: Arc<CompactionObserveState>,
) {
    let mut stdin = Some(stdin);

    // One-time commands: capture state, then start the prompt.
    send_rpc_line(&mut stdin, &serde_json::json!({"id":"get-state-1","type":"get_state"}).to_string());
    send_rpc_line(
        &mut stdin,
        &serde_json::json!({"id":"prompt-1","type":"prompt","message":prompt_message}).to_string(),
    );

    let mut fired = false;
    let mut warned = false;
    let mut stats_in_flight: Option<String> = None;
    let mut stats_counter: u64 = 0;

    while let Ok(line) = line_rx.recv() {
        if line.trim().is_empty() {
            continue;
        }
        // Plan 088: live compaction observation — the driver owns the
        // line stream; the helper prefix-filters and no-ops when no
        // observer is attached.
        observe_compaction_line(&line, &observe_state, &observer);
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue, // malformed line — pass through, ignore
        };
        let t = v.get("type").and_then(|t| t.as_str());

        match t {
            Some("response") => {
                match v.get("command").and_then(|c| c.as_str()) {
                    Some("get_state") => {
                        let mut s = shared.lock().expect("shared state poisoned");
                        if let Some(sid) =
                            v.pointer("/data/sessionId").and_then(|s| s.as_str())
                        {
                            s.session_id = Some(sid.to_string());
                            // Plan 088: seed the live compaction
                            // observation state (the RPC stream has no
                            // `session` header event).
                            observe_state
                                .set_session_id(Some(sid.to_string()));
                        }
                        if let Some(en) = v
                            .pointer("/data/autoCompactionEnabled")
                            .and_then(|b| b.as_bool())
                        {
                            s.compaction_enabled = Some(en);
                        }
                        drop(s);
                    }
                    Some("get_session_stats") => {
                        stats_in_flight = None;
                        // Refresh the token usage from the sample.
                        if let Some(toks) = v.pointer("/data/tokens").and_then(|t| t.as_object()) {
                            let usage = TokenUsage {
                                input: toks.get("input").and_then(|x| x.as_u64()).unwrap_or(0),
                                output: toks.get("output").and_then(|x| x.as_u64()).unwrap_or(0),
                                cache_read: toks
                                    .get("cacheRead")
                                    .and_then(|x| x.as_u64())
                                    .unwrap_or(0),
                                cache_write: toks
                                    .get("cacheWrite")
                                    .and_then(|x| x.as_u64())
                                    .unwrap_or(0),
                                total: toks.get("total").and_then(|x| x.as_u64()).unwrap_or(0),
                            };
                            shared.lock().expect("shared state poisoned").token_usage =
                                Some(usage);
                        }
                        // `contextUsage` is omitted (or `tokens` is null) when
                        // no valid usage is available — skip the sample.
                        let cu = v.pointer("/data/contextUsage").and_then(|c| c.as_object());
                        if let (Some(limit), Some(cu)) = (limit, cu) {
                            let tokens = cu.get("tokens").and_then(|x| x.as_u64());
                            let window = cu.get("contextWindow").and_then(|x| x.as_u64());
                            if let Some(tokens) = tokens {
                                // pi's own runtime report of whether
                                // auto-compaction is on for this session
                                // (`get_state`); `None` until the state
                                // response arrives, then defaults on.
                                let compaction_on = shared
                                    .lock()
                                    .expect("shared state poisoned")
                                    .compaction_enabled
                                    .unwrap_or(true);
                                // Warn once when the limit sits at/above pi's
                                // auto-compaction point — pi may compact
                                // before the limit is reached. Only relevant
                                // while pi's own auto-compaction is enabled.
                                let near = match window {
                                    Some(w) => {
                                        limit >= w.saturating_sub(PI_DEFAULT_RESERVE_TOKENS)
                                    }
                                    None => true,
                                };
                                if !warned && compaction_on && near {
                                    eprintln!(
                                        "[knot] pi-rpc: ctx-wrap-up-limit ({limit}) is within {PI_DEFAULT_RESERVE_TOKENS} tokens of the context window; pi may compact before the wrap-up limit is reached — lower ctx-wrap-up-limit or raise pi's reserveTokens"
                                    );
                                    warned = true;
                                }
                                // Steer once when the context crosses the
                                // limit (regardless of compaction — the
                                // steer is the remedy whether or not pi will
                                // auto-compact).
                                if !fired && tokens >= limit && send_rpc_line(
                                    &mut stdin,
                                    &serde_json::json!({"type":"steer","message":WRAP_UP_STEER}).to_string(),
                                ) {
                                    fired = true;
                                    shared
                                        .lock()
                                        .expect("shared state poisoned")
                                        .wrap_up = Some(WrapUpRecord {
                                            context_tokens: tokens,
                                            limit,
                                            mechanism: "steer".to_string(),
                                        });
                                }
                            }
                        }
                    }
                    // `prompt` / `steer` / other responses need no action.
                    _ => {}
                }
            }
            Some("turn_end") if stats_in_flight.is_none() => {
                // Always sample on a completed turn: it refreshes the token
                // usage (observability, parity with the JSON runner) and —
                // when a limit is set — is the steer decision point. The
                // match guard coalesces so we never have two polls in flight.
                stats_counter += 1;
                let id = format!("stats-{stats_counter}");
                let cmd = serde_json::json!({"id":id.clone(),"type":"get_session_stats"}).to_string();
                if send_rpc_line(&mut stdin, &cmd) {
                    stats_in_flight = Some(id);
                }
            }
            Some("agent_end") => {
                agent_end_seen.store(true, Ordering::Relaxed);
                // Close stdin to signal pi to exit; keep draining so a late
                // stats response is still processed. Dropping the `Option`
                // closes the pipe; the driver returns once the channel
                // disconnects (after the process exits).
                stdin = None;
            }
            _ => {}
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value_objects::AgentConfig;
    use std::path::PathBuf;

    /// Write an executable mock script at a temp path; returns (path, dir).
    fn mock_script(script: &str) -> (PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mock-pi-rpc");
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        (path, dir)
    }

    /// Build an `ExecutionContext` for the runner.
    fn ctx(prompt: &str, limit: Option<u64>) -> ExecutionContext {
        ExecutionContext {
            agent_config: AgentConfig {
                goal: "test".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
                ctx_wrap_up_limit: limit,
            },
            prompt: prompt.to_string(),
            profile_prompt: String::new(),
            strand_path: StrandPath(PathBuf::from("rig/looms/x-knot/strands/y.strand")),
            event_type: "Created".to_string(),
            knot_name: Some("x".to_string()),
            timeout: None,
        }
    }

    #[test]
    fn runner_type_is_pi_rpc() {
        let runner = PiRpcAgentRunner::new();
        assert_eq!(runner.runner_type(), "pi-rpc");
    }

    #[test]
    fn rpc_args_use_mode_rpc_without_print_flag() {
        let config = AgentConfig {
            goal: "g".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            tools: vec!["bash".to_string()],
            extra_args: vec!["--name".to_string(), "t".to_string()],
            thinking_level: Some(crate::domain::value_objects::ThinkingLevel::Medium),
            ctx_wrap_up_limit: None,
        };
        let args = PiRpcAgentRunner::build_rpc_cli_args(&config);
        // No `-p` (RPC takes the prompt over the stdin protocol).
        assert!(!args.iter().any(|a| a == "-p"));
        assert!(args.iter().any(|a| a == "--model"));
        assert!(args.iter().any(|a| a == "bash"));
        let mode_idx = args.iter().position(|a| a == "--mode").unwrap();
        assert_eq!(args[mode_idx + 1], "rpc");
    }

    /// The mock speaks the minimal RPC protocol: it records every stdin
    /// command to `stdin_log`, answers `get_state` / `get_session_stats` /
    /// `steer`, and emits a single turn. `agent_end` is emitted either right
    /// after the turn (mode `"prompt"`) or after the stats response (mode
    /// `"stats"`); the latter mirrors real pi where the stats sample for a
    /// turn_end poll lands before the agent finishes.
    fn rpc_mock_script(stdin_log: &str, tokens: u64, window: u64, mode: &str, compaction: &str) -> String {
        format!(
            r#"#!/usr/bin/env bash
log="{stdin_log}"
mode="{mode}"
emitted_end=0
emit_end() {{
  if [ "$emitted_end" = "0" ]; then
    echo '{{"type":"agent_end","messages":[{{"role":"assistant","stopReason":"stop","content":[{{"type":"text","text":"wrapped up cleanly"}}]}}]}}'
    emitted_end=1
  fi
}}
while IFS= read -r line; do
  echo "$line" >> "$log"
  case "$line" in
    *get_state*)
      echo '{{"type":"response","command":"get_state","success":true,"data":{{"sessionId":"sess-rpc-1","autoCompactionEnabled":{compaction}}}}}'
      ;;
    *get_session_stats*)
      echo '{{"type":"response","command":"get_session_stats","success":true,"data":{{"sessionId":"sess-rpc-1","tokens":{{"input":1000,"output":200,"cacheRead":0,"cacheWrite":0,"total":1200}},"contextUsage":{{"tokens":{tokens},"contextWindow":{window},"percent":40}}}}}}'
      if [ "$mode" = "stats" ]; then emit_end; fi
      ;;
    *prompt*)
      echo '{{"type":"agent_start"}}'
      echo '{{"type":"turn_start"}}'
      echo '{{"type":"turn_end","turnIndex":0,"message":{{}},"toolResults":[]}}'
      if [ "$mode" = "prompt" ]; then emit_end; fi
      ;;
    *steer*)
      echo '{{"type":"response","command":"steer","success":true}}'
      ;;
  esac
done
"#
        )
    }

    #[test]
    fn success_captures_response_session_and_usage() {
        let (path, dir) = mock_script("placeholder");
        let log = dir.path().join("stdin.log");
        let script = rpc_mock_script(log.to_str().unwrap(), 60_000, 200_000, "prompt", "true");
        std::fs::write(&path, &script).unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let out = runner.execute(ctx("do the thing", None)).unwrap();
        assert_eq!(out.stdout, "wrapped up cleanly");
        let meta = out.metadata.as_ref().unwrap();
        assert_eq!(meta.session_id.as_deref(), Some("sess-rpc-1"));
        // Stats are polled on turn_end even without a limit -> usage captured.
        assert_eq!(meta.token_usage.unwrap().total, 1200);
        assert!(meta.wrap_up.is_none());
        // The driver must have sent get_state, the prompt, and a stats poll.
        let sent = std::fs::read_to_string(&log).unwrap();
        assert!(sent.contains("get_state"));
        assert!(sent.contains("prompt"));
        assert!(sent.contains("get_session_stats"));
        assert!(!sent.contains("steer"));
    }

    #[test]
    fn steer_fires_once_when_limit_crossed() {
        let (path, dir) = mock_script("placeholder");
        let log = dir.path().join("stdin.log");
        // tokens 150000 >= limit 140000 -> fire; stats lands before agent_end.
        let script = rpc_mock_script(log.to_str().unwrap(), 150_000, 200_000, "stats", "true");
        std::fs::write(&path, &script).unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let out = runner.execute(ctx("do the thing", Some(140_000))).unwrap();
        let meta = out.metadata.as_ref().unwrap();
        let wrap = meta.wrap_up.as_ref().unwrap();
        assert_eq!(wrap.context_tokens, 150_000);
        assert_eq!(wrap.limit, 140_000);
        // Exactly one steer command on the stdin log.
        let sent = std::fs::read_to_string(&log).unwrap();
        let steer_count = sent.matches("\"steer\"").count();
        assert_eq!(steer_count, 1);
    }

    #[test]
    fn no_steer_when_limit_not_crossed() {
        let (path, dir) = mock_script("placeholder");
        let log = dir.path().join("stdin.log");
        // tokens 60000 < limit 140000 -> no fire.
        let script = rpc_mock_script(log.to_str().unwrap(), 60_000, 200_000, "stats", "false");
        std::fs::write(&path, &script).unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let out = runner.execute(ctx("do the thing", Some(140_000))).unwrap();
        let meta = out.metadata.as_ref().unwrap();
        assert!(meta.wrap_up.is_none());
        let sent = std::fs::read_to_string(&log).unwrap();
        assert!(!sent.contains("\"steer\""));
    }

    #[test]
    fn steer_fires_even_when_compaction_disabled() {
        // Plan 084: the steer is the remedy regardless of whether pi will
        // auto-compact, so a disabled compaction must NOT suppress it. The
        // once-per-run compaction warning, by contrast, is gated on it.
        let (path, dir) = mock_script("placeholder");
        let log = dir.path().join("stdin.log");
        let script = rpc_mock_script(log.to_str().unwrap(), 150_000, 200_000, "stats", "false");
        std::fs::write(&path, &script).unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let out = runner.execute(ctx("do the thing", Some(140_000))).unwrap();
        let meta = out.metadata.as_ref().unwrap();
        assert!(meta.wrap_up.is_some());
        let sent = std::fs::read_to_string(&log).unwrap();
        assert_eq!(sent.matches("\"steer\"").count(), 1);
    }

    /// Plan 088 (D4): the mock's compaction-stream variants. `recovered`
    /// mirrors pi's overflow recovery that succeeds (a `compaction_start`
    /// + `compaction_end` span, then a clean `agent_end`); `terminal`
    /// mirrors recovery that fails (two `compaction_end` records —
    /// willRetry true, then willRetry false with the error message — and
    /// an `agent_end` with `stopReason: "error"` and no final text).
    /// JSON-mode parity: the same streams are what the pi_json overflow
    /// tests feed the parser.
    fn rpc_compaction_script(stdin_log: &str, variant: &str) -> String {
        format!(
            r#"#!/usr/bin/env bash
log="{stdin_log}"
variant="{variant}"
while IFS= read -r line; do
  echo "$line" >> "$log"
  case "$line" in
    *get_state*)
      echo '{{"type":"response","command":"get_state","success":true,"data":{{"sessionId":"sess-compact","autoCompactionEnabled":true}}}}'
      ;;
    *prompt*)
      echo '{{"type":"agent_start"}}'
      echo '{{"type":"turn_start"}}'
      if [ "$variant" = "recovered" ]; then
        echo '{{"type":"compaction_start","reason":"auto"}}'
        echo '{{"type":"compaction_end","reason":"auto","result":{{"summary":"s","firstKeptEntryId":"e","tokensBefore":120000,"details":{{}}}},"aborted":false,"willRetry":true}}'
        echo '{{"type":"turn_end","turnIndex":0,"message":{{}},"toolResults":[]}}'
        echo '{{"type":"agent_end","messages":[{{"role":"assistant","stopReason":"stop","content":[{{"type":"text","text":"done after compaction"}}]}}]}}'
      else
        echo '{{"type":"compaction_start","reason":"overflow"}}'
        echo '{{"type":"compaction_end","reason":"overflow","result":{{"summary":"s","firstKeptEntryId":"e","tokensBefore":150000,"details":{{}}}},"aborted":false,"willRetry":true}}'
        echo '{{"type":"compaction_end","reason":"overflow","aborted":false,"willRetry":false,"errorMessage":"Context overflow recovery failed after one compact-and-retry attempt. The session context is still too large."}}'
        echo '{{"type":"agent_end","messages":[{{"role":"assistant","stopReason":"error","errorMessage":"prompt is too long","content":[{{"type":"text","text":"context overflow"}}]}}]}}'
      fi
      ;;
  esac
done
"#
        )
    }

    /// Plan 088 (D4): overflow recovery that succeeds over the RPC stream
    /// is a **success** — the compaction end (reason `auto`, tokens
    /// before, no error, not aborted) is recorded, and the start reason
    /// is captured in `compaction_starts`.
    #[test]
    fn rpc_overflow_recovered_is_success() {
        let (path, dir) = mock_script("placeholder");
        let log = dir.path().join("stdin.log");
        std::fs::write(
            &path,
            &rpc_compaction_script(log.to_str().unwrap(), "recovered"),
        )
        .unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let out = runner.execute(ctx("do the thing", None)).unwrap();
        assert_eq!(out.stdout, "done after compaction");
        let meta = out.metadata.as_ref().unwrap();
        assert_eq!(meta.session_id.as_deref(), Some("sess-compact"));
        assert_eq!(meta.compactions.len(), 1, "one recovered compaction");
        let rec = &meta.compactions[0];
        assert_eq!(rec.reason, "auto");
        assert_eq!(rec.tokens_before, Some(120_000));
        assert!(rec.error.is_none());
        assert!(!rec.aborted);
        assert_eq!(meta.compaction_starts, vec!["auto".to_string()]);
    }

    /// Plan 088 (D4): a terminal overflow over the RPC stream (recovery
    /// ran and failed — `willRetry: false` with the error message, no
    /// final response text) is classified as `ContextLimitReached`
    /// carrying the session ID — JSON-mode parity (pi_json
    /// `test_json_runner_terminal_overflow_returns_context_limit_reached`).
    #[test]
    fn rpc_terminal_overflow_returns_context_limit_reached() {
        let (path, dir) = mock_script("placeholder");
        let log = dir.path().join("stdin.log");
        std::fs::write(
            &path,
            &rpc_compaction_script(log.to_str().unwrap(), "terminal"),
        )
        .unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let result = runner.execute(ctx("do the thing", None));
        let err = result
            .expect_err("terminal overflow must fail")
            ;
        assert!(
            matches!(err, PortError::ContextLimitReached { .. }),
            "expected ContextLimitReached, got {err:?}"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-compact"),
            "the error carries the session id from get_state"
        );
        assert!(
            err.to_string().contains("Context overflow recovery failed"),
            "the error carries pi's recovery-failure message: {err:?}"
        );
    }

    /// Plan 088 (D4 + D5): with the observer attached, the RPC driver
    /// reports the compaction span live — `Started` (reason `auto`) then
    /// `Ended` (reason + error) — and both carry the session id seeded
    /// from the `get_state` response (the RPC stream has no `session`
    /// header event).
    #[test]
    fn rpc_compaction_end_reasons_recorded() {
        let (path, dir) = mock_script("placeholder");
        let log = dir.path().join("stdin.log");
        std::fs::write(
            &path,
            &rpc_compaction_script(log.to_str().unwrap(), "recovered"),
        )
        .unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());

        let seen: Arc<Mutex<Vec<CompactionObservation>>> =
            Arc::new(Mutex::new(Vec::new()));
        let seen_c = Arc::clone(&seen);
        let observer: Arc<dyn Fn(&CompactionObservation) + Send + Sync> =
            Arc::new(move |o: &CompactionObservation| {
                seen_c.lock().unwrap().push(o.clone());
            });

        let out = runner
            .execute_inner(ctx("do the thing", None), Some(observer))
            .unwrap();
        assert_eq!(out.stdout, "done after compaction");

        let observations = seen.lock().unwrap();
        assert_eq!(observations.len(), 2, "span start + end: {observations:?}");
        match &observations[0] {
            CompactionObservation::Started {
                session_id,
                reason,
            } => {
                assert_eq!(
                    session_id.as_deref(),
                    Some("sess-compact"),
                    "session id seeded from get_state"
                );
                assert_eq!(reason, "auto");
            }
            other => panic!("Expected Started, got {other:?}"),
        }
        match &observations[1] {
            CompactionObservation::Ended {
                session_id,
                record,
            } => {
                assert_eq!(session_id.as_deref(), Some("sess-compact"));
                assert_eq!(record.reason, "auto");
                assert!(record.error.is_none(), "recovered span has no error");
                assert!(!record.aborted);
            }
            other => panic!("Expected Ended, got {other:?}"),
        }
    }
}
