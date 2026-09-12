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
//!
//! **A compaction span counts as activity** (plan 089 D8, extending the
//! plan-081 rule): pi emits no stream bytes while its summarisation call
//! runs, so the event observers call [`touch_activity`] on the span
//! boundaries and [`spawn_watchdog`] skips the inactivity test entirely
//! while a span is open. The total budget is untouched — a wedged
//! compaction is still killed, but on the deadline and reported as a
//! `Timeout`, which is what actually happened.

use std::io::{Read, Result as IoResult};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
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

    /// Record activity now (plan 089 D8 — see [`touch_activity`]).
    pub fn touch(&self) {
        touch_activity(&self.last_activity);
    }

    /// The silence so far (`now - last_activity`).
    pub fn silence(&self) -> Duration {
        let now = now_unix_nanos();
        let last = self.last_activity.load(Ordering::Relaxed);
        Duration::from_nanos(now.saturating_sub(last))
    }
}

/// Stamp `last_activity` with now — an **explicit** activity signal for
/// spans that are legitimately silent (plan 089 D8: pi emits no stream
/// bytes while a compaction's summarisation call runs, and 300 s of
/// silence is a normal compaction on a loaded workstation, not a stall).
///
/// Callers are the stream observers, i.e. the code that *parses* events —
/// the byte-level timer in [`spawn_reader`] stays free of JSON parsing.
pub fn touch_activity(last_activity: &AtomicU64) {
    last_activity.store(now_unix_nanos(), Ordering::Relaxed);
}

/// Should the watchdog kill for **inactivity** on this tick? (plan 081,
/// held by plan 089 D8)
///
/// `silent` is nanos since the last byte (or explicit
/// [`touch_activity`](crate::adapters::live_output::touch_activity) stamp).
/// A held span never fires: pi emits no stream bytes while a compaction's
/// summarisation call runs, so inside a span the byte-level question is not
/// the one worth asking — the total budget is.
///
/// Pure so the rule is testable; testing the watchdog itself would mean
/// killing a real process group from a unit test.
fn inactivity_due(inactivity_timeout: Option<Duration>, silent: u64, held: bool) -> bool {
    !held
        && inactivity_timeout.is_some_and(|window| silent > window.as_nanos() as u64)
}

/// Unix nanos of `SystemTime::now()` (0 if the clock is pre-epoch).
pub(crate) fn now_unix_nanos() -> u64 {
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

/// Spawn a reader thread that drains `src` into `buf` (stamping
/// `last_activity` on every non-empty read, exactly like
/// [`spawn_reader`]) **and** splits complete LF-terminated lines, sending
/// each to `line_tx` (plan 084).
///
/// The byte buffer is the source of truth for the watchdog and for the
/// post-hoc `parse_stdout`; the line channel feeds the incremental session
/// driver. A trailing partial line (no closing newline) is held back, not
/// emitted — pi's JSONL stream always terminates each line with LF, so in
/// practice the held remainder is empty at EOF. A trailing `\r` is trimmed
/// (CRLF tolerance). Malformed lines pass through unchanged; the driver
/// ignores anything that is not a valid JSON event line.
pub fn spawn_line_reader(
    thread_name: &str,
    src: impl Read + Send + 'static,
    buf: &Arc<Mutex<Vec<u8>>>,
    last_activity: &Arc<AtomicU64>,
    line_tx: mpsc::Sender<String>,
) -> IoResult<JoinHandle<()>> {
    let buf = Arc::clone(buf);
    let last_activity = Arc::clone(last_activity);
    std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || {
            let mut src = src;
            let mut chunk = [0u8; 8192];
            let mut partial: Vec<u8> = Vec::new();
            loop {
                match src.read(&mut chunk) {
                    Ok(0) => break, // EOF — all write ends closed
                    Ok(n) => {
                        last_activity.store(now_unix_nanos(), Ordering::Relaxed);
                        buf.lock()
                            .expect("output buffer mutex poisoned")
                            .extend_from_slice(&chunk[..n]);
                        partial.extend_from_slice(&chunk[..n]);
                        // Emit every complete line; hold the tail.
                        while let Some(pos) =
                            partial.iter().position(|&b| b == b'\n')
                        {
                            let line_bytes: Vec<u8> = partial.drain(..=pos).collect();
                            let mut line =
                                String::from_utf8_lossy(&line_bytes).into_owned();
                            if line.ends_with('\r') {
                                line.pop();
                            }
                            // Ignore a send failure — the receiver (driver)
                            // has already exited, and there is nothing left
                            // to do with the line.
                            let _ = line_tx.send(line);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            // A held partial (no closing newline) is intentionally not
            // emitted — see the doc comment.
            let _ = partial;
        })
}

/// Spawn a reader thread that drains `src` into `buf` (stamping
/// `last_activity` on every non-empty read, exactly like
/// [`spawn_reader`]) **and** invokes `on_line` for each complete
/// LF-terminated line (plan 088 — live compaction observation).
///
/// The byte buffer is the source of truth for the watchdog and for the
/// post-hoc `parse_stdout`; `on_line` is an incremental, best-effort
/// hook — the callback sees each complete line as it arrives and must
/// be non-blocking with respect to the main thread (it runs on the
/// reader thread, and the main thread only joins this one after the
/// child exits). A trailing partial line (no closing newline) is held
/// back, not emitted — pi's JSONL stream always terminates each line
/// with LF, so in practice the held remainder is empty at EOF. A
/// trailing `\r` is trimmed (CRLF tolerance).
pub fn spawn_reader_with_lines(
    thread_name: &str,
    src: impl Read + Send + 'static,
    buf: &Arc<Mutex<Vec<u8>>>,
    last_activity: &Arc<AtomicU64>,
    on_line: impl Fn(&str) + Send + 'static,
) -> IoResult<JoinHandle<()>> {
    let buf = Arc::clone(buf);
    let last_activity = Arc::clone(last_activity);
    std::thread::Builder::new()
        .name(thread_name.to_string())
        .spawn(move || {
            let mut src = src;
            let mut chunk = [0u8; 8192];
            let mut partial: Vec<u8> = Vec::new();
            loop {
                match src.read(&mut chunk) {
                    Ok(0) => break, // EOF — all write ends closed
                    Ok(n) => {
                        last_activity.store(now_unix_nanos(), Ordering::Relaxed);
                        buf.lock()
                            .expect("output buffer mutex poisoned")
                            .extend_from_slice(&chunk[..n]);
                        partial.extend_from_slice(&chunk[..n]);
                        // Invoke the callback for every complete line;
                        // hold the tail. (The `buf` lock is already
                        // dropped — the callback must not assume it.
                        // It holds nothing either: its work — loom-log
                        // append, system-event emission — takes its own
                        // locks only inside the calls.)
                        while let Some(pos) =
                            partial.iter().position(|&b| b == b'\n')
                        {
                            let line_bytes: Vec<u8> = partial.drain(..=pos).collect();
                            let mut line =
                                String::from_utf8_lossy(&line_bytes).into_owned();
                            if line.ends_with('\r') {
                                line.pop();
                            }
                            on_line(&line);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            // A held partial (no closing newline) is intentionally not
            // emitted — same convention as [`spawn_line_reader`].
            let _ = partial;
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
///
/// `compaction_hold` (plan 089 D8) suspends the **inactivity** test while a
/// compaction span is open: the stream is silent by design while pi
/// summarises, so the honest answer to "has it written anything lately?"
/// is *no* for the whole span. The total budget is **not** suspended — a
/// wedged compaction still dies on the deadline, with a `Timeout` rather
/// than a fabricated stall. Pass `None` for a runner that does not track
/// spans (the one-shot stdio adapter).
#[allow(clippy::too_many_arguments)] // flat parameter list keeps the two adapter call sites readable
pub fn spawn_watchdog(
    thread_name: &str,
    pgid: i32,
    cli_path: String,
    strand_desc: String,
    total_timeout: Duration,
    inactivity_timeout: Option<Duration>,
    compaction_hold: Option<Arc<AtomicBool>>,
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
                // Inactivity first: the more specific diagnosis wins —
                // unless a compaction span is open (plan 089 D8), in which
                // case the silence is expected and only the total budget
                // applies (a wedged compaction is killed as `Timeout`, which
                // is what actually happened, not as a fabricated stall).
                let held = compaction_hold
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Relaxed));
                let silent =
                    now_unix_nanos().saturating_sub(last_activity.load(Ordering::Relaxed));
                if inactivity_due(inactivity_timeout, silent, held) {
                    let window = inactivity_timeout
                        .expect("inactivity_due is false without a window");
                    {
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

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(300);

    /// Plan 089 (D8) baseline: silence past the window is still a stall.
    #[test]
    fn inactivity_fires_past_the_window() {
        assert!(inactivity_due(Some(WINDOW), WINDOW.as_nanos() as u64 + 1, false));
    }

    /// Plan 089 (D8): inside a compaction span the same silence is not a
    /// stall — the span is activity.
    #[test]
    fn inactivity_is_held_while_a_compaction_span_is_open() {
        assert!(!inactivity_due(Some(WINDOW), WINDOW.as_nanos() as u64 + 1, true));
        // And it resumes the moment the span closes: the watchdog loop keeps
        // measuring from the stamp the span boundary left behind.
        assert!(inactivity_due(Some(WINDOW), WINDOW.as_nanos() as u64 + 1, false));
    }

    /// Silence inside the window never fires, held or not.
    #[test]
    fn silence_inside_the_window_is_not_a_stall() {
        assert!(!inactivity_due(Some(WINDOW), WINDOW.as_nanos() as u64 - 1, false));
        assert!(!inactivity_due(Some(WINDOW), WINDOW.as_nanos() as u64 - 1, true));
    }

    /// No window configured (the default) disables the test entirely.
    #[test]
    fn no_window_never_fires() {
        assert!(!inactivity_due(None, u64::MAX, false));
        assert!(!inactivity_due(None, u64::MAX, true));
    }

    /// [`LiveOutput::touch`] resets the measured silence — this is what the
    /// span boundary stamps (D8) and what the 18-minute compaction needed.
    #[test]
    fn touch_resets_the_measured_silence() {
        let live = LiveOutput::default();
        // Pretend the last byte arrived 10 s ago.
        live.last_activity
            .store(now_unix_nanos() - 10_000_000_000, Ordering::Relaxed);
        assert!(live.silence() >= Duration::from_secs(9));
        live.touch();
        assert!(live.silence() < Duration::from_secs(1), "{:?}", live.silence());
    }
}
