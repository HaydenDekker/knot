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
//! - **Teardown**: the driver closes stdin when pi is *genuinely done* —
//!   on `agent_settled` (pi's own "the whole prompt finished, post-agent
//!   compaction included" event), or as a fallback for older pi on
//!   `agent_end` with no compaction span open once a short settle window
//!   has passed. It must not close earlier: pi's rpc mode exits on stdin
//!   **EOF**, so an early close cuts off pi's post-`agent_end` work —
//!   which is where auto-compaction runs (plan 089 D5). The main thread
//!   then waits a bounded grace for the process to exit and force-kills
//!   the process group otherwise. A SIGKILL during teardown *after* the
//!   close is a normal exit, not a timeout.
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
    observe_compaction_line, CompactionObserveState, OverflowFailure,
    PiJsonAgentRunner,
};
use crate::application::ports::{
    AgentInvocationMetadata, AgentOutput, AgentRunner, CompactionObservation,
    CompactionRecord, ExecutionContext, PortError, TokenUsage, WrapUpRecord,
};
use crate::domain::entities::StrandPath;
use crate::domain::value_objects::AgentConfig;

/// The steering message queued when the context crosses the wrap-up limit.
///
/// Plan 086: the water-mark handoff note (replacing 084's terminal
/// `WRAP_UP_STEER` wind-down). Reuses the greppable const from
/// `value_objects` so the text is single-sourced.
const WRAP_UP_STEER: &str = crate::domain::value_objects::HANDOFF_NOTE;

/// Bounded grace after the driver closes stdin for the process to exit on
/// its own; beyond this the group is force-killed. The close happens on
/// `agent_settled` (or the `agent_end` fallback — see [`SETTLE_WINDOW`]),
/// never at `agent_end` itself, so this grace never races a compaction.
const TEARDOWN_GRACE: Duration = Duration::from_secs(5);

/// How long the driver waits after `agent_end` before closing stdin on the
/// **fallback** path (a pi old enough not to emit `agent_settled`), and the
/// interval at which the driver re-checks its teardown predicate
/// (plan 089 D5).
///
/// The window exists because pi emits `compaction_start` in the same tick
/// *after* `agent_end`; closing on the `agent_end` line itself (or with no
/// window) closes the pipe one line before the span opens, and pi exits on
/// stdin EOF mid-compaction — the defect that made every threshold
/// compaction end its attempt. A `compaction_start` arriving inside the
/// window defers the close until the span closes.
const SETTLE_WINDOW: Duration = Duration::from_millis(250);

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

/// Teardown / liveness flags exchanged between the driver thread (writer)
/// and the main teardown loop + inactivity watchdog (readers).
///
/// Plan 089 (D4, D5, D8): the *decision* to close stdin lives in exactly
/// one place — the driver's [`teardown_due`] predicate — and this is how the
/// rest of the run learns its outcome:
///
/// - `agent_end` is **sticky** ("a turn finished at some point") and feeds
///   the success classification only; the per-turn teardown decision uses
///   the driver's own turn state.
/// - `teardown_armed` is set the instant the driver closes stdin; the main
///   loop starts [`TEARDOWN_GRACE`] from there, so the 5 s force-kill grace
///   keeps its meaning (a *graceful* process that will not exit) and an
///   in-flight compaction is bounded by the total budget instead.
/// - `open_compaction` is set on `compaction_start` and cleared on
///   `compaction_end`; it holds the teardown open (D4) and, from plan 089
///   D8, the inactivity watchdog too.
#[derive(Clone, Default)]
struct RpcFlags {
    /// A sticky `agent_end` was seen (run classification).
    agent_end: Arc<AtomicBool>,
    /// The driver closed stdin — arm the teardown grace.
    teardown_armed: Arc<AtomicBool>,
    /// A compaction span is open (no matching `compaction_end` yet).
    open_compaction: Arc<AtomicBool>,
}

/// The driver's per-turn view of the session (plan 089 D5).
///
/// Reset when the driver asks the session to continue in-place, so the
/// teardown predicate always describes the *current* turn rather than the
/// first one.
#[derive(Default)]
struct TurnState {
    /// `agent_end` seen for the current turn.
    agent_end: bool,
    /// When the current turn's `agent_end` arrived (start of the fallback
    /// settle window), if it has.
    agent_end_at: Option<Instant>,
    /// `agent_settled` seen for the current turn — pi has finished the
    /// whole prompt, post-agent compaction included (plan 089 D5).
    settled: bool,
}

/// The one teardown predicate (plan 089 D5): close stdin when pi is done
/// with the turn and nothing is still running inside it.
///
/// `agent_settled` is authoritative (pi ≥ 0.78). Without it, fall back to
/// `agent_end` once no compaction span is open and the settle window has
/// elapsed — pi emits `compaction_start` in the same tick *after*
/// `agent_end`, so the window is what keeps the fallback from re-creating
/// the mid-compaction kill on older pi.
fn teardown_due(turn: &TurnState, compaction_open: bool) -> bool {
    if turn.settled {
        return true;
    }
    turn.agent_end
        && !compaction_open
        && turn
            .agent_end_at
            .is_some_and(|at| at.elapsed() >= SETTLE_WINDOW)
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
        // Plan 089 (D4 + D5): the teardown flags. The driver decides when to
        // close stdin ([`teardown_due`]) and arms the grace here; an open
        // compaction span holds the close (and the force-kill) until the
        // span closes, bounded by the total budget.
        let flags = RpcFlags::default();
        // Plan 088: shared session-id state for live compaction
        // observation (seeded from the `get_state` response).
        let observe_state = Arc::new(CompactionObserveState::new());

        let driver: JoinHandle<()> = thread::Builder::new()
            .name("rpc-driver".to_string())
            .spawn({
                let shared = Arc::clone(&shared);
                let flags = flags.clone();
                let observer = observer.clone();
                let observe_state = Arc::clone(&observe_state);
                move || {
                    run_rpc_driver(
                        stdin,
                        line_rx,
                        prompt_message,
                        limit,
                        shared,
                        flags,
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

        // Teardown: the driver closes stdin when pi is genuinely done
        // (`agent_settled`, or the `agent_end` + settle-window fallback with
        // no compaction span open — plan 089 D5) and sets `teardown_armed`.
        // Only then does the main loop start the [`TEARDOWN_GRACE`] clock for
        // a clean exit; until it is armed the run is bound by the total
        // budget alone, so a long in-flight compaction is never raced
        // (plan 089 D4).
        let start = Instant::now();
        let mut armed_at: Option<Instant> = None;
        loop {
            if flags.teardown_armed.load(Ordering::Relaxed) {
                let at = *armed_at.get_or_insert_with(Instant::now);
                if wait_handle.is_finished() {
                    break;
                }
                if at.elapsed() >= TEARDOWN_GRACE {
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

        let saw_agent_end = flags.agent_end.load(Ordering::Relaxed);

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
                if let Some(failure) = PiJsonAgentRunner::classify_overflow_failure(
                    &compactions,
                    &compaction_starts,
                    error_message.as_deref(),
                ) {
                    return Err(match failure {
                        // Plan 089: interrupted auto-compact — resumable.
                        OverflowFailure::CompactionInterrupted { reason } => {
                            PortError::CompactionInterrupted {
                                message: format!(
                                    "pi's in-process overflow compaction (reason: {reason}) was interrupted — the process stopped before it completed; an out-of-band manual compact may still recover the session"
                                ),
                                reason,
                                session_id,
                            }
                        }
                        OverflowFailure::ContextLimitReached { message } => {
                            PortError::ContextLimitReached { message, session_id }
                        }
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

    /// Plan 089: out-of-band manual compaction on an interrupted session.
    ///
    /// Opens the session via `pi --mode rpc --session <id>` (the session
    /// restores its own model), issues the `compact` RPC command (carrying
    /// `custom_instructions` as the summarisation guidance), and awaits the
    /// `compaction_end { reason: "manual" }` event, bounded by the runner's
    /// timeout. On success returns the [`CompactionRecord`] (reason
    /// `"manual"`, `tokens_before` from `compaction_end.result.tokensBefore`);
    /// on a missing end within the timeout, a `success: false` command
    /// response, an `errorMessage`, or an `aborted` compact it returns
    /// [`PortError::ManualCompactionFailed`] (terminal — the context is
    /// over-full even after an explicit compact, so a re-entry would overflow
    /// again).
    fn manual_compact(
        &self,
        ctx: &ExecutionContext,
        session_id: &str,
        custom_instructions: &str,
    ) -> Result<CompactionRecord, PortError> {

        let failed = |message: String| PortError::ManualCompactionFailed {
            message,
            session_id: Some(session_id.to_string()),
        };

        // Build the RPC args the same way `execute` does (core + `--mode
        // rpc`), then open the existing session by id.
        let mut cli_args = Self::build_rpc_cli_args(&ctx.agent_config);
        cli_args.push("--session".to_string());
        cli_args.push(session_id.to_string());

        // Spawn in its own process group so teardown can kill child +
        // subprocesses (RPC mode stays open after the compact).
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
                return Err(failed(format!(
                    "failed to spawn '{}' for manual compact: {}",
                    self.cli_path, e
                )));
            }
        };

        let child_pid = child.id() as i32;
        let mut stdin =
            Some(BufWriter::new(child.stdin.take().expect("stdin was piped")));

        let live = LiveOutput::new();
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let stdout_reader = spawn_line_reader(
            "manual-compact-stdout",
            child.stdout.take().expect("stdout was piped"),
            &live.stdout,
            &live.last_activity,
            line_tx,
        );
        // Drain stderr so the child never blocks on a full stderr pipe.
        let stderr_reader = spawn_reader(
            "manual-compact-stderr",
            child.stderr.take().expect("stderr was piped"),
            &live.stderr,
            &live.last_activity,
        );

        let result = (|| -> Result<CompactionRecord, PortError> {
            // Issue the compact command (with the operator's summarisation
            // instructions, when any).
            let mut compact_cmd =
                serde_json::json!({"id":"manual-compact","type":"compact"});
            if !custom_instructions.is_empty() {
                compact_cmd["customInstructions"] =
                    serde_json::json!(custom_instructions);
            }
            if !send_rpc_line(&mut stdin, &compact_cmd.to_string()) {
                return Err(failed(
                    "failed to send the compact command to pi".to_string(),
                ));
            }

            let deadline = Instant::now() + self.timeout;
            // The `compact` command response and the `compaction_end` event
            // both report the pre-compact token count; prefer the event
            // (the plan's contract) and fall back to the command response.
            let mut tokens_from_response: Option<u64> = None;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(failed(format!(
                        "manual compact did not complete within {}s (no compaction_end)",
                        self.timeout.as_secs()
                    )));
                }
                match line_rx.recv_timeout(remaining) {
                    Ok(line) => {
                        let Ok(value) =
                            serde_json::from_str::<serde_json::Value>(&line)
                        else {
                            continue;
                        };
                        let typ = value
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        match typ {
                            "response" => {
                                let command = value
                                    .get("command")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("");
                                if command != "compact" {
                                    continue;
                                }
                                if value.get("success").and_then(|s| s.as_bool())
                                    == Some(false)
                                {
                                    let error = value
                                        .get("error")
                                        .and_then(|e| e.as_str())
                                        .unwrap_or("compact command rejected")
                                        .to_string();
                                    return Err(failed(error));
                                }
                                tokens_from_response = value
                                    .get("data")
                                    .and_then(|d| d.get("tokensBefore"))
                                    .and_then(|t| t.as_u64());
                            }
                            "compaction_end" => {
                                let reason = value
                                    .get("reason")
                                    .and_then(|r| r.as_str())
                                    .unwrap_or("");
                                if reason != "manual" {
                                    continue; // not our compact
                                }
                                if value.get("aborted").and_then(|a| a.as_bool())
                                    == Some(true)
                                {
                                    return Err(failed(
                                        "manual compact was aborted by pi".to_string(),
                                    ));
                                }
                                if let Some(err) =
                                    value.get("errorMessage").and_then(|e| e.as_str())
                                {
                                    return Err(failed(err.to_string()));
                                }
                                let tokens_before = value
                                    .get("result")
                                    .and_then(|r| r.get("tokensBefore"))
                                    .and_then(|t| t.as_u64())
                                    .or(tokens_from_response);
                                return Ok(CompactionRecord {
                                    reason: "manual".to_string(),
                                    tokens_before,
                                    will_retry: false,
                                    error: None,
                                    aborted: false,
                                });
                            }
                            _ => {}
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        return Err(failed(format!(
                            "manual compact did not complete within {}s (no compaction_end)",
                            self.timeout.as_secs()
                        )));
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        // pi exited without a compaction_end — the recovery
                        // never completed.
                        return Err(failed(
                            "pi exited before the manual compact completed (no compaction_end)"
                                .to_string(),
                        ));
                    }
                }
            }
        })();

        // Terminate the child (and any subprocesses) regardless of outcome —
        // RPC mode stays open after the compact.
        let _ = unsafe { libc::kill(-child_pid, libc::SIGKILL) };
        let _ = child.wait();
        if let Ok(h) = stdout_reader {
            let _ = h.join();
        }
        if let Ok(h) = stderr_reader {
            let _ = h.join();
        }

        result
    }
}

/// The RPC session driver: owns the stdin writer and the line receiver,
/// sends `get_state` + `prompt`, then samples context on `turn_end` and
/// fires the wrap-up steer once the limit is crossed.
///
/// The driver **closes stdin** when the turn is really over — on
/// `agent_settled`, or on `agent_end` with no compaction span open once the
/// [`SETTLE_WINDOW`] has passed (plan 089 D5, [`teardown_due`]) — and keeps
/// draining the channel afterwards so a `get_session_stats` response that
/// lands late is still processed (it may carry the sample that trips the
/// steer). Closing any earlier would end the run inside pi's post-
/// `agent_end` compaction: pi's rpc mode treats stdin EOF as shutdown.
/// The driver returns when the line channel disconnects (the stdout reader
/// hit EOF — the process has exited).
#[allow(clippy::too_many_arguments)] // flat parameter list keeps the single driver call site readable
fn run_rpc_driver(
    stdin: BufWriter<ChildStdin>,
    line_rx: mpsc::Receiver<String>,
    prompt_message: String,
    limit: Option<u64>,
    shared: Arc<Mutex<RpcShared>>,
    flags: RpcFlags,
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
    // Plan 089 (D5): the current turn's teardown state. The loop ticks on a
    // [`SETTLE_WINDOW`]-sized timeout so the fallback close happens even
    // when pi goes silent after `agent_end` (the old-pi shape, where no
    // `agent_settled` ever arrives).
    let mut turn = TurnState::default();

    loop {
        let line = match line_rx.recv_timeout(SETTLE_WINDOW) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No stream bytes for a tick: the settle window may have
                // elapsed with the turn finished and no compaction open.
                if stdin.is_some()
                    && teardown_due(&turn, flags.open_compaction.load(Ordering::Relaxed))
                {
                    stdin = None;
                    flags.teardown_armed.store(true, Ordering::Relaxed);
                }
                continue;
            }
            // The stdout reader hit EOF — the process exited.
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
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
                // A turn finished and its text is in the buffer. This is
                // **not** the end of the run: pi still runs its post-agent
                // work (auto-compaction) in the same awaited prompt and
                // emits `agent_settled` when that is done (plan 089 D5), so
                // stdin stays open and the settle window starts here.
                flags.agent_end.store(true, Ordering::Relaxed);
                turn.agent_end = true;
                turn.agent_end_at = Some(Instant::now());
            }
            // Plan 089 (D5): pi finished the whole prompt — post-agent
            // compaction included. This is the authoritative teardown
            // signal; the loop below closes stdin on it.
            Some("agent_settled") => {
                turn.settled = true;
            }
            // Plan 089 (D4): an in-flight compaction holds the teardown —
            // stdin stays open (pi exits on EOF) and the main loop does not
            // start the force-kill grace. `compaction_start` opens the
            // span, the matching `compaction_end` closes it. pi runs
            // compactions sequentially, so a single flag is well-defined
            // across repeated spans.
            Some("compaction_start") => {
                flags.open_compaction.store(true, Ordering::Relaxed);
            }
            Some("compaction_end") => {
                flags.open_compaction.store(false, Ordering::Relaxed);
            }
            _ => {}
        }

        // Teardown check after every line: pi may settle (or end the turn,
        // with no compaction open) on this very line.
        if stdin.is_some()
            && teardown_due(&turn, flags.open_compaction.load(Ordering::Relaxed))
        {
            // Dropping the writer closes the pipe — pi's rpc mode exits on
            // EOF. The driver keeps draining until the channel disconnects.
            stdin = None;
            flags.teardown_armed.store(true, Ordering::Relaxed);
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
      elif [ "$variant" = "attempted" ]; then
        echo '{{"type":"compaction_start","reason":"overflow"}}'
        echo '{{"type":"agent_end","messages":[{{"role":"assistant","stopReason":"error","errorMessage":"400 request (150001 tokens) exceeds the available context size (150000 tokens), try increasing it","content":[]}}]}}'
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

    /// Plan 089: the exact shape of the pwa-todo-3 incident over the RPC
    /// stream — a `compaction_start { overflow }` observed but **no**
    /// `compaction_end` (the pi process died mid-compaction). This is now
    /// classified as the resumable `PortError::CompactionInterrupted` (not the
    /// terminal `ContextLimitReached`), mirroring the JSON-runner parity.
    #[test]
    fn rpc_overflow_compaction_attempted_but_could_not_fit() {
        let (path, dir) = mock_script("placeholder");
        let log = dir.path().join("stdin.log");
        std::fs::write(
            &path,
            &rpc_compaction_script(log.to_str().unwrap(), "attempted"),
        )
        .unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        // The span is never closed and the mock does not die on its own, so
        // the run is bounded by its total budget (plan 089 D4/D5: an open
        // compaction holds stdin open and the deadline is the hard bound —
        // a real pi crash exits the process and ends the run immediately).
        // Kept short so the test does not wait the 120 s default.
        let mut c = ctx("do the thing", None);
        c.timeout = Some(Duration::from_millis(1200));
        let result = runner.execute(c);
        assert!(
            result.is_err(),
            "interrupted overflow should fail, got: {result:?}"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, PortError::CompactionInterrupted { .. }),
            "expected CompactionInterrupted (interrupted auto-compact), got {err:?}"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-compact"),
            "the error carries the session id from get_state"
        );
        // Resumable (a session was captured): the plan 089 manual-compact
        // recovery applies.
        assert!(
            err.is_resumable(),
            "interrupted compact with a session is resumable"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("interrupted"),
            "the error states the compaction was interrupted: {err:?}"
        );
        assert!(
            msg.contains("overflow"),
            "the error carries the compaction reason: {err:?}"
        );
    }

    // ── Plan 089 (D2): out-of-band manual compaction ──

    /// Plan 089 (D2): a manual `compact` on an interrupted session that
    /// completes emits a `compaction_end { reason: "manual" }` whose
    /// `result.tokensBefore` is reported as the pre-compact context size.
    #[test]
    fn rpc_manual_compact_success() {
        let (path, _dir) = mock_script("placeholder");
        let script = r#"#!/usr/bin/env bash
read -r _
echo '{"type":"compaction_end","reason":"manual","result":{"summary":"s","firstKeptEntryId":"x","tokensBefore":120000,"details":{}},"aborted":false,"willRetry":false}'
"#;
        std::fs::write(&path, script).unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let result = runner.manual_compact(&ctx("do the thing", None), "sess-int", "keep task state");
        let rec = result.expect("manual compact should succeed");
        assert_eq!(rec.reason, "manual");
        assert_eq!(rec.tokens_before, Some(120_000));
        assert!(!rec.aborted, "a successful manual compact is not aborted");
        assert!(rec.error.is_none(), "a successful manual compact carries no error");
    }

    /// Plan 089 (D2): a manual `compact` that never completes (no
    /// `compaction_end` within the deadline) is a terminal
    /// `ManualCompactionFailed` (the context is over-full even after the
    /// explicit compact, so a re-entry would overflow again).
    #[test]
    fn rpc_manual_compact_timeout() {
        let (path, _dir) = mock_script("placeholder");
        // Reads the compact command, then hangs — never emits a
        // compaction_end.
        let script = r#"#!/usr/bin/env bash
read -r _
sleep 10
"#;
        std::fs::write(&path, script).unwrap();
        // A short deadline so the test does not wait the 120s default.
        let runner = PiRpcAgentRunner::with_cli_path_and_timeout(
            path.to_string_lossy().to_string(),
            Duration::from_millis(400),
        );
        let result = runner.manual_compact(&ctx("do the thing", None), "sess-int", "");
        let err = result.expect_err("a manual compact that never ends should fail");
        assert!(
            matches!(err, PortError::ManualCompactionFailed { .. }),
            "expected ManualCompactionFailed, got {err:?}"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-int"),
            "the error carries the session id"
        );
        assert!(
            !err.is_resumable(),
            "a failed manual compact is terminal (not resumable)"
        );
    }

    // ── Plan 089 (D4): the in-flight-compaction teardown hold ──

    /// Plan 089 (D4): a stray `compaction_end` that lands after `agent_end`
    /// (with no preceding `compaction_start`) must not break a clean run —
    /// the run still returns success. The driver's end-arm is a no-op when no
    /// compaction is open, so teardown proceeds normally at `agent_end`.
    #[test]
    fn rpc_d4_agent_end_then_stray_compaction_end_is_success() {
        let (path, _dir) = mock_script("placeholder");
        let script = r#"#!/usr/bin/env bash
echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"done"}]}]}'
echo '{"type":"compaction_end","reason":"threshold","result":{"tokensBefore":100000},"aborted":false,"willRetry":false}'
"#;
        std::fs::write(&path, script).unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let out =
            runner.execute(ctx("do the thing", None)).expect("clean run must succeed");
        assert!(
            out.stdout.contains("done"),
            "final text should be the agent_end text: {}",
            out.stdout
        );
    }

    /// Plan 089 (D4): when a `compaction_start` is open at `agent_end` (an
    /// in-flight compaction), teardown is **held** until the deadline rather
    /// than force-killing on the short `agent_end` grace — so the run is
    /// bounded by the total timeout, not the 5s grace. The mock emits
    /// `compaction_start { overflow }` + a clean `agent_end`, then holds
    /// (no `compaction_end`, no exit); the run must wait for the deadline
    /// (here 1200ms) and still succeed (the clean final answer is kept).
    ///
    /// Note the event order here is **inverted** with respect to real pi,
    /// which emits `compaction_start` *after* `agent_end` (plan 089 D5). The
    /// test is kept for the flag-hold behaviour on that order; the pi-shaped
    /// order is covered by the D5 tests below with the stdin-aware mock.
    #[test]
    fn rpc_d4_open_compaction_holds_teardown_until_deadline() {
        let (path, _dir) = mock_script("placeholder");
        let script = r#"#!/usr/bin/env bash
echo '{"type":"compaction_start","reason":"overflow"}'
echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"held"}]}]}'
sleep 10
"#;
        std::fs::write(&path, script).unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let mut c = ctx("do the thing", None);
        c.timeout = Some(Duration::from_millis(1200));

        let start = Instant::now();
        let out = runner.execute(c).expect("clean run must succeed");
        let elapsed = start.elapsed();

        assert!(
            out.stdout.contains("held"),
            "final text should be the agent_end text: {}",
            out.stdout
        );
        assert!(
            elapsed >= Duration::from_millis(900),
            "the open-compaction hold should wait for the deadline (got {elapsed:?})"
        );
        assert!(
            elapsed < Duration::from_millis(4000),
            "teardown should be bounded by the deadline, not the 5s grace (got {elapsed:?})"
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

    // ── Plan 089 (D5): settle-based teardown + the stdin hold ──

    /// Build a **stdin-aware** mock that models pi's rpc mode (plan 089
    /// D5) closely enough to catch the teardown bug:
    ///
    /// - `body` (pi's stream, in pi's real event order — `agent_end` first,
    ///   `compaction_start` a tick later) is emitted by a background writer
    ///   that inherits stdout;
    /// - the foreground loop reads stdin until **EOF**, answering
    ///   `get_state`, appending every command to `log` and the literal line
    ///   `eof` when the pipe closes;
    /// - at EOF it **kills the writer and exits** — pi's
    ///   `process.stdin.on("end", … shutdown)` mid-work, so an early close
    ///   loses the rest of the stream exactly like the real crash.
    ///
    /// A mock that ignores stdin (or emits its whole stream before reading)
    /// cannot model pi: it neither dies on EOF nor loses events when it does,
    /// which is why the plan-089 phase-4 tests passed while the defect stayed
    /// live (D5's "why the D4 tests missed this").
    fn pi_like_rpc_mock(log: &str, body: &str) -> String {
        // Built by substitution (not `format!`) so the bash braces and the
        // JSON braces need no escaping.
        let template = r#"#!/usr/bin/env bash
log="__LOG__"
: > "$log"
{
__BODY__
} &
writer=$!
while IFS= read -r line; do
  echo "$line" >> "$log"
  case "$line" in
    *get_state*)
      echo '{"type":"response","command":"get_state","success":true,"data":{"sessionId":"sess-d5","autoCompactionEnabled":true}}'
      ;;
  esac
done
echo eof >> "$log"
kill $writer 2>/dev/null
exit 0
"#;
        template.replace("__LOG__", log).replace("__BODY__", body)
    }

    /// Run a pi-shaped mock and return the runner's result plus the elapsed
    /// wall time.
    fn run_pi_like(
        script: &str,
        timeout: Duration,
    ) -> (Result<AgentOutput, PortError>, Duration) {
        let (path, _dir) = mock_script("placeholder");
        std::fs::write(&path, script).unwrap();
        let runner = PiRpcAgentRunner::with_cli_path(path.to_string_lossy().to_string());
        let mut c = ctx("do the thing", None);
        c.timeout = Some(timeout);
        let start = Instant::now();
        let result = runner.execute(c);
        (result, start.elapsed())
    }

    /// Plan 089 (D5): scenario A — pi ends the turn with no text, then runs
    /// its post-`agent_end` **threshold** compaction. The driver must not
    /// close stdin (pi exits on EOF) while that work is in flight: the
    /// `compaction_end` is observed and the process exits cleanly instead of
    /// dying ~1 s into the summarisation. Under the pre-D5 code the mock
    /// exits at the `agent_end` close and never reaches `compaction_start`.
    #[test]
    fn rpc_agent_end_empty_then_threshold_compaction_completes() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("stdin.log");
        let script = pi_like_rpc_mock(
            log.to_str().unwrap(),
            r#"echo '{"type":"agent_start"}'
echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[]}]}'
sleep 0.05
echo '{"type":"compaction_start","reason":"threshold"}'
sleep 0.4
echo '{"type":"compaction_end","reason":"threshold","result":{"summary":"s","firstKeptEntryId":"e","tokensBefore":140000,"details":{}},"aborted":false,"willRetry":false}'
echo '{"type":"agent_settled"}'
"#,
        );
        let (result, elapsed) = run_pi_like(&script, Duration::from_secs(15));
        let out = result.expect("a compaction that completes must not fail the run");
        let meta = out.metadata.as_ref().expect("metadata");
        assert_eq!(
            meta.compactions.len(),
            1,
            "the post-agent_end compaction_end must be observed, not killed mid-span"
        );
        assert_eq!(meta.compactions[0].reason, "threshold");
        assert_eq!(meta.compactions[0].tokens_before, Some(140_000));
        assert_eq!(
            out.exit_code, 0,
            "the process exits cleanly on the closed stdin (no force-kill)"
        );
        let sent = std::fs::read_to_string(&log).unwrap();
        assert!(
            sent.contains("eof"),
            "Knot closes stdin once the session settles: {sent}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "the run ends on settle, not on a deadline (got {elapsed:?})"
        );
    }

    /// Plan 089 (D5): the clean single-turn path keeps working — one prompt
    /// is sent, stdin is closed on `agent_settled`, and the process exits
    /// inside the grace (no force-kill, no second prompt).
    #[test]
    fn rpc_closes_stdin_on_agent_settled() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("stdin.log");
        let script = pi_like_rpc_mock(
            log.to_str().unwrap(),
            r#"echo '{"type":"agent_start"}'
echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"all done"}]}]}'
echo '{"type":"agent_settled"}'
"#,
        );
        let (result, elapsed) = run_pi_like(&script, Duration::from_secs(15));
        let out = result.expect("clean run");
        assert_eq!(out.stdout, "all done");
        assert_eq!(out.exit_code, 0);
        let sent = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            sent.matches("\"prompt\"").count(),
            1,
            "exactly one prompt command: {sent}"
        );
        assert!(sent.contains("eof"), "Knot closed the stdin: {sent}");
        assert!(
            elapsed < Duration::from_secs(3),
            "teardown is on `agent_settled`, not on a timeout (got {elapsed:?})"
        );
    }

    /// Plan 089 (D5): a pi old enough to omit `agent_settled` still gets a
    /// teardown — `agent_end` plus the settle window closes the pipe, well
    /// inside the deadline.
    #[test]
    fn rpc_fallback_settle_window_closes_stdin_without_settle_event() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("stdin.log");
        let script = pi_like_rpc_mock(
            log.to_str().unwrap(),
            r#"echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"old pi"}]}]}'
"#,
        );
        let (result, elapsed) = run_pi_like(&script, Duration::from_secs(15));
        let out = result.expect("old-pi shape still succeeds");
        assert_eq!(out.stdout, "old pi");
        assert_eq!(out.exit_code, 0);
        assert!(
            std::fs::read_to_string(&log).unwrap().contains("eof"),
            "the fallback closes stdin after the settle window"
        );
        assert!(
            elapsed < Duration::from_secs(3),
            "settle window + exit, not the 15s deadline (got {elapsed:?})"
        );
    }

    /// Plan 089 (D5): a `compaction_start` arriving **inside** the settle
    /// window defers the fallback close — the span is observed and the run
    /// still tears down afterwards (pi emits the start in the same tick
    /// after `agent_end`, so this ordering is the normal one).
    #[test]
    fn rpc_fallback_settle_window_defers_for_compaction_start() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("stdin.log");
        let script = pi_like_rpc_mock(
            log.to_str().unwrap(),
            r#"echo '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"held"}]}]}'
sleep 0.05
echo '{"type":"compaction_start","reason":"overflow"}'
sleep 0.4
echo '{"type":"compaction_end","reason":"overflow","result":{"summary":"s","firstKeptEntryId":"e","tokensBefore":150000,"details":{}},"aborted":false,"willRetry":false}'
"#,
        );
        let (result, elapsed) = run_pi_like(&script, Duration::from_secs(15));
        let out = result.expect("deferred close still succeeds");
        assert_eq!(out.stdout, "held");
        let meta = out.metadata.as_ref().expect("metadata");
        assert_eq!(
            meta.compactions.len(),
            1,
            "the span opened inside the settle window is still observed"
        );
        assert!(
            elapsed >= Duration::from_millis(400),
            "the close waited for the span, not the 250ms window (got {elapsed:?})"
        );
        assert!(
            std::fs::read_to_string(&log).unwrap().contains("eof"),
            "stdin is closed once the span closes"
        );
    }
}
