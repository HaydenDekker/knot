//! Shared live-output capture + watchdog for the agent adapters
//! (plan 081).
//!
//! Replaces the `wait_with_output()` capture, which buffers all output
//! until exit and cannot observe liveness:
//!
//! - **stdout reader thread** — chunk-reads into a shared accumulated
//!   buffer and stamps `last_activity` (unix nanos) on every non-empty
//!   read. Initialised at spawn, so the pre-first-byte window counts
//!   (a provider that cannot answer the first token within the window
//!   is exactly what we want flagged).
//! - **stderr reader thread** — same, into its own buffer (stderr still
//!   feeds the existing error messages). Stderr bytes also reset the
//!   timer (a process writing diagnostics is alive; being lenient here
//!   avoids spurious kills).
//! - **watchdog thread** — polls every 250 ms against both deadlines:
//!   inactivity first (the more specific diagnosis — it wins when both
//!   elapse), then the total budget.
//!
//! Detection is **byte-level**: any byte on the child's stdout/stderr
//! resets the timer. No JSON parsing is involved in the timer itself;
//! parsing happens only *after* a kill, on the accumulated buffer.

use std::io::{Read, Result as IoResult};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Why the watchdog killed the process group (plan 081).
///
/// The absence of a value (`None` in `LiveOutput::kill_reason`) means
/// the watchdog did not kill: the child either exited on its own
/// (status-first classification ignores the reason) or was killed by
/// an external signal (legacy `Timeout` shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillReason {
    /// No output for the inactivity window.
    Inactivity,
    /// The total wall-clock budget elapsed (existing behaviour).
    Total,
}

/// Shared liveness state for one child process.
pub struct LiveOutput {
    /// stdout drained by the stdout reader thread.
    pub stdout: Arc<Mutex<Vec<u8>>>,
    /// stderr drained by the stderr reader thread.
    pub stderr: Arc<Mutex<Vec<u8>>>,
    /// Unix nanos of the last non-empty read (either stream);
    /// initialised at spawn so the pre-first-byte window counts.
    pub last_activity: Arc<AtomicU64>,
    /// Set by the watchdog (or the join guard) when it kills the group.
    pub kill_reason: Arc<Mutex<Option<KillReason>>>,
}

impl Default for LiveOutput {
    /// Create the state, stamping `last_activity` now (spawn time).
    fn default() -> Self {
        Self {
            stdout: Arc::new(Mutex::new(Vec::new())),
            stderr: Arc::new(Mutex::new(Vec::new())),
            last_activity: Arc::new(AtomicU64::new(now_unix_nanos())),
            kill_reason: Arc::new(Mutex::new(None)),
        }
    }
}

impl LiveOutput {
    /// Create the state, stamping `last_activity` now (spawn time).
    pub fn new() -> Self {
        Self::default()
    }

    /// The silence so far (`now - last_activity`).
    pub fn silence(&self) -> Duration {
        let now = now_unix_nanos();
        let last = self.last_activity.load(Ordering::Relaxed);
        Duration::from_nanos(now.saturating_sub(last))
    }
}

/// Unix nanos of `SystemTime::now()` (0 if the clock is pre-epoch).
fn now_unix_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Spawn a reader thread that drains `src` into `buf`, stamping
/// `last_activity` on every non-empty read — byte-level: any byte on
/// the stream is activity.
pub fn spawn_reader(
    thread_name: &str,
    src: impl Read + Send + 'static,
    buf: &Arc<Mutex<Vec<u8>>>,
    last_activity: &Arc<AtomicU64>,
) -> IoResult<JoinHandle<()>> {
    let buf = Arc::clone(buf);
    let last_activity = Arc::clone(last_activity);
    std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || {
            let mut src = src;
            let mut chunk = [0u8; 8192];
            loop {
                match src.read(&mut chunk) {
                    Ok(0) => break, // EOF — all write ends closed
                    Ok(n) => {
                        last_activity.store(now_unix_nanos(), Ordering::Relaxed);
                        buf.lock()
                            .expect("output buffer mutex poisoned")
                            .extend_from_slice(&chunk[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        })
}

/// Record the kill reason if no kill has been recorded yet — the first
/// kill wins (the watchdog checks inactivity first, so it wins the
/// race when both deadlines elapse).
fn record_kill(kill_reason: &Arc<Mutex<Option<KillReason>>>, reason: KillReason) {
    let mut guard = kill_reason.lock().expect("kill-reason mutex poisoned");
    if guard.is_none() {
        *guard = Some(reason);
    }
}

/// Spawn the watchdog thread: polls every 250 ms and kills the child's
/// process group (child + subprocesses, `kill(-pgid, SIGKILL)`) when a
/// deadline elapses.
///
/// Inactivity is checked **first** and wins when both elapse (e.g. a
/// fully silent session with equal windows) — it is the more specific
/// diagnosis and carries the restart note. The total-budget warning
/// line is the legacy text, unchanged.
pub fn spawn_watchdog(
    thread_name: &str,
    pgid: i32,
    cli_path: String,
    strand_desc: String,
    total_timeout: Duration,
    inactivity_timeout: Option<Duration>,
    live: &LiveOutput,
    cancelled: Arc<AtomicBool>,
) -> IoResult<JoinHandle<()>> {
    let last_activity = Arc::clone(&live.last_activity);
    let kill_reason = Arc::clone(&live.kill_reason);
    std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || {
            let started = Instant::now();
            loop {
                std::thread::sleep(Duration::from_millis(250));
                // The child exited and the main thread marked it —
                // suppress the kill + warning.
                if cancelled.load(Ordering::Relaxed) {
                    return;
                }
                // Inactivity first: the more specific diagnosis wins.
                if let Some(window) = inactivity_timeout {
                    let silent =
                        now_unix_nanos().saturating_sub(last_activity.load(Ordering::Relaxed));
                    if silent > window.as_nanos() as u64 {
                        record_kill(&kill_reason, KillReason::Inactivity);
                        // Kill the entire process group (child +
                        // subprocesses).
                        let _ = unsafe { libc::kill(-pgid, libc::SIGKILL) };
                        eprintln!(
                            "WARNING: killed '{}' after inactivity — no output for {:?} (strand: {})",
                            cli_path, window, strand_desc
                        );
                        return;
                    }
                }
                if started.elapsed() > total_timeout {
                    record_kill(&kill_reason, KillReason::Total);
                    // Kill the entire process group (child +
                    // subprocesses).
                    let _ = unsafe { libc::kill(-pgid, libc::SIGKILL) };
                    eprintln!(
                        "WARNING: killed '{}' after timeout of {:?} (strand: {})",
                        cli_path, total_timeout, strand_desc
                    );
                    return;
                }
            }
        })
}

/// Wait for the child to exit and both readers to drain to EOF.
///
/// Preserves the legacy 2×-deadline join guard: if the set is not done
/// by `deadline` (e.g. an orphaned grandchild keeps a pipe open),
/// force-kill the process group once and proceed. `cancelled` is set
/// as soon as the exit status is known so the watchdog suppresses its
/// kill + warning.
pub fn join_all(
    wait: JoinHandle<IoResult<ExitStatus>>,
    stdout_reader: JoinHandle<()>,
    stderr_reader: JoinHandle<()>,
    pgid: i32,
    deadline: Duration,
    cancelled: &AtomicBool,
) -> IoResult<ExitStatus> {
    let start = Instant::now();

    // Phase 1: wait for the child to exit. The 2×-deadline join guard
    // force-kills the process group if it doesn't.
    while !wait.is_finished() {
        if start.elapsed() > deadline {
            // Child didn't exit in time — force kill the entire process
            // group (child + subprocesses), then wait.
            let _ = unsafe { libc::kill(-pgid, libc::SIGKILL) };
            std::thread::sleep(Duration::from_millis(500));
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Join once, after the loop. (A child stuck in an uninterruptible
    // state would hang here — the same exposure as the legacy
    // `wait_with_output` join.)
    let status = wait.join().expect("wait thread panicked");
    // The child exited — the watchdog no longer needs to act.
    cancelled.store(true, Ordering::Relaxed);

    // Phase 2: let the readers drain to EOF, bounded by the remainder
    // of the join deadline (a pipe-holding grandchild past the deadline
    // is force-killed with the group — after the group kill the pipes
    // are closed and the readers finish at EOF).
    let drain_budget = deadline
        .saturating_sub(start.elapsed())
        .max(Duration::from_millis(500));
    let drain_start = Instant::now();
    while !stdout_reader.is_finished() || !stderr_reader.is_finished() {
        if drain_start.elapsed() > drain_budget {
            let _ = unsafe { libc::kill(-pgid, libc::SIGKILL) };
            std::thread::sleep(Duration::from_millis(500));
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();

    status
}
