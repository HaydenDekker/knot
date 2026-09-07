//! Plan 083 — consolidated service log: binary-level integration tests.
//!
//! Spawns the `knot` binary with stderr captured and asserts the
//! single-line `[KNOT][EVENT]` / `[KNOT][STATE]` record format on that
//! stream, the absence of the retired `.rig-log` / `.loom-log` files,
//! and the change-driven behaviour of `state.json` (written at
//! startup, rewritten only when the rig actually changes).
//!
//! The per-variant line grammar is unit-tested in
//! `src/adapters/service_log.rs`; these tests pin the end-to-end
//! sequencing and the durable surfaces around the lines.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Resolve the path to the compiled `knot` binary.
fn binary_path() -> String {
    std::env::var("CARGO_BIN_EXE_knot").unwrap_or_else(|_| {
        format!(
            "{}/target/debug/knot",
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string())
        )
    })
}

fn runtime_root(rig_dir: &Path) -> PathBuf {
    knot::domain::knot_file::derive_runtime_root(rig_dir)
}

/// Create a minimal rig at `cwd/dev-rig` (one loom + knot, the `fast`
/// profile, a mock `pi` that records its runs and exits 0 after
/// `sleep_secs`). The `*-rig` suffix is required for rig discovery.
fn create_rig(cwd: &Path, sleep_secs: &str) -> PathBuf {
    let rig = cwd.join("dev-rig");
    let loom = rig.join("review-loom");
    fs::create_dir_all(&loom).unwrap();
    fs::write(
        loom.join("review.md"),
        "---\nname: review\nagent-profile-ref: fast\nstrand-dir: \"./strands\"\ngit-versioned: false\n---\n\nTest knot.\n",
    )
    .unwrap();
    let profiles = rig.join("profiles");
    fs::create_dir_all(&profiles).unwrap();
    fs::write(
        profiles.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\n---\n\nYou are a reviewer.\n",
    )
    .unwrap();
    let pi = rig.join("bin").join("pi");
    fs::create_dir_all(pi.parent().unwrap()).unwrap();
    fs::write(
        &pi,
        format!(
            "#!/usr/bin/env bash\ncat > /dev/null\nsleep {sleep_secs}\necho \"ok\"\nexit 0\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&pi).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&pi, perms).unwrap();
    }
    rig
}

/// Spawn the knot service (auto-discovery of the single rig) with fast
/// timing: 20ms debounce, 2ms check, 100ms state-write tick, and the
/// mock `pi` injected via `KNOT_TEST_CLI_PATH`.
///
/// Wrapped in a guard: if a test panics before stopping the service,
/// the child is killed on drop (no leaked service processes).
struct ServiceGuard(Option<Child>);

impl ServiceGuard {
    fn into_inner(mut self) -> Child {
        self.0
            .take()
            .expect("service guard must own the child")
    }
}

impl Drop for ServiceGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_service(cwd: &Path, pi_path: &Path) -> ServiceGuard {
    ServiceGuard(Some(
        Command::new(binary_path())
            .current_dir(cwd)
            .env("KNOT_TEST_DEBOUNCE_MS", "20")
            .env("KNOT_TEST_CHECK_MS", "2")
            .env("KNOT_STATE_WRITE_MS", "100")
            .env("KNOT_TEST_CLI_PATH", pi_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("should spawn knot service"),
    ))
}

/// SIGINT the service, wait (up to 15s) for a graceful exit, and
/// return the captured output. Falls back to SIGKILL on timeout.
///
/// The pipes are read *after* reaping — the writer ends close when the
/// process exits, so the data remains in the pipe (both streams are
/// small; the 64KB pipe buffer never fills).
fn stop_service(guard: ServiceGuard) -> (std::process::Output, bool) {
    let mut child = guard.into_inner();
    let pid = child.id().to_string();
    #[cfg(unix)]
    {
        let _ = Command::new("kill").args(["-INT", &pid]).status();
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let (status, graceful) = loop {
        match child.try_wait() {
            Ok(Some(s)) => break (s, true),
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let s = child.wait().expect("reap after kill");
                    break (s, false);
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("try_wait failed: {e}"),
        }
    };
    let stderr = child.stderr.take().and_then(|mut s| {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut s, &mut buf)
            .ok()
            .map(|_| buf)
    }).unwrap_or_default();
    let stdout = child.stdout.take().and_then(|mut s| {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut s, &mut buf)
            .ok()
            .map(|_| buf)
    }).unwrap_or_default();
    (
        std::process::Output {
            status,
            stdout: stdout.into_bytes(),
            stderr: stderr.into_bytes(),
        },
        graceful,
    )
}

/// Poll `pred` every 50ms until it returns true or the deadline passes.
fn wait_for(mut pred: impl FnMut() -> bool, ms: u64, msg: &str) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        if pred() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(pred(), "timed out waiting for: {msg}");
}

fn read_state(rig_dir: &Path) -> serde_json::Value {
    let path = runtime_root(rig_dir).join("state.json");
    let content =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

/// Non-panicking variant for polling (returns `None` while the file is
/// absent or not yet valid JSON).
fn try_read_state(rig_dir: &Path) -> Option<serde_json::Value> {
    let path = runtime_root(rig_dir).join("state.json");
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// The `[KNOT][EVENT]` payload lines of a captured stderr (prefix and
/// leading timestamp stripped).
fn event_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter_map(|l| l.split("[KNOT][EVENT] ").nth(1).map(|s| s.to_string()))
        .collect()
}

/// The `[KNOT][STATE]` payload lines of a captured stderr.
fn state_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .filter_map(|l| l.split("[KNOT][STATE] ").nth(1).map(|s| s.to_string()))
        .collect()
}

/// Assert the lines in `expected` occur in order (not necessarily
/// contiguously) within `lines`.
fn assert_in_order(lines: &[String], expected: &[&str], context: &str) {
    let mut from = 0;
    for needle in expected {
        let idx = lines[from..]
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| {
                panic!(
                    "expected '{needle}' after line {from} in {context}:\n  {}",
                    lines.join("\n  ")
                )
            });
        from += idx + 1;
    }
}

/// Count whole lines containing `needle`.
fn count_lines(lines: &[String], needle: &str) -> usize {
    lines.iter().filter(|l| l.contains(needle)).count()
}

// ── Test 1: full run — event sequencing + no log files ────────────────────

#[test]
fn full_run_emits_event_lines_and_no_log_files() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    let rig = create_rig(cwd, "0");
    let pi = rig.join("bin").join("pi");

    let child = spawn_service(cwd, &pi);

    // Let the watchers settle, then trigger a run; the tie-off is the
    // durable evidence to wait for.
    thread::sleep(Duration::from_secs(1));    let strands = cwd.join("strands");
    fs::create_dir_all(&strands).unwrap();
    fs::write(strands.join("s1.md"), "strand one").unwrap();
    let tie_off = runtime_root(&rig).join("review-loom").join("tie-off-review.md");
    wait_for(
        || tie_off.exists(),
        15_000,
        "tie-off written",
    );

    let (output, graceful) = stop_service(child);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let events = event_lines(&stderr);
    let states = state_lines(&stderr);

    // Per-run bracket + discovery + processing, in order. (Discovery
    // emits `KnotRegistered` per knot before the `LoomStarted` bracket.)
    let mut expected = vec![
        "KnotRegistered loom=review-loom knot=review",
        "LoomStarted loom=review-loom",
        "KnotProcessing loom=review-loom knot=review strand=",
        "KnotCompleted loom=review-loom knot=review strand=",
        "StrandProcessed loom=review-loom strand=",
    ];
    if graceful {
        expected.push("LoomStopped loom=review-loom");
    }
    assert_in_order(&events, &expected, "full-run event lines");

    // Exactly one completion for the one strand.
    assert_eq!(
        count_lines(&events, "KnotCompleted"),
        1,
        "one KnotCompleted expected; events:\n  {}",
        events.join("\n  ")
    );

    // Baseline state line + the status delta, in order. (The observed
    // transition is tick-dependent — `idle→completed` when the run
    // finishes between ticks, `processing→completed` when a tick lands
    // mid-run — so only the stable prefix is pinned; the durable
    // outcome is asserted on state.json below.)
    assert_in_order(
        &states,
        &[
            "initial snapshot looms=1 knots=1",
            "change knot review-loom/review: status",
        ],
        "full-run state lines",
    );
    assert!(
        states.iter().any(|l| l.contains("change knot review-loom/review: status")
            && l.contains("completed")),
        "the status delta must land on completed; state lines:\n  {}",
        states.join("\n  ")
    );

    // Plan 083: the retired log files must not exist anywhere under the
    // runtime root.
    let root = runtime_root(&rig);
    assert!(
        !root.join(".rig-log").exists(),
        ".rig-log must not be created"
    );
    assert!(
        !root.join("review-loom").join(".loom-log").exists(),
        ".loom-log must not be created"
    );

    // Durable surface: state.json reflects the completion.
    let state = read_state(&rig);
    let knots = state["looms"][0]["knots"].as_array().unwrap();
    assert_eq!(knots[0]["id"], "review");
    assert_eq!(knots[0]["status"], "completed");
}

// ── Test 2: state.json is change-driven ───────────────────────────────────

#[test]
fn state_json_written_at_startup_and_only_on_change() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    let rig = create_rig(cwd, "0");
    let pi = rig.join("bin").join("pi");

    let child = spawn_service(cwd, &pi);

    // Baseline: state.json exists with the discovered loom.
    let state_path = runtime_root(&rig).join("state.json");
    wait_for(
        || {
            try_read_state(&rig)
                .and_then(|s| s["looms"].as_array().map(|l| l.len()))
                == Some(1)
        },
        10_000,
        "baseline state.json",
    );

    // Idle: the file is untouched — neither content nor mtime changes
    // across several state-write ticks.
    thread::sleep(Duration::from_millis(300));
    let before = fs::read(&state_path).unwrap();
    let mtime_before = fs::metadata(&state_path).unwrap().modified().ok();
    thread::sleep(Duration::from_millis(700));
    let after = fs::read(&state_path).unwrap();
    let mtime_after = fs::metadata(&state_path).unwrap().modified().ok();
    assert_eq!(before, after, "state.json content must be stable while idle");
    assert_eq!(
        mtime_before, mtime_after,
        "state.json mtime must not churn while idle"
    );

    // Real change: a strand appears → queue+ delta → rewrite.
    thread::sleep(Duration::from_secs(1)); // let the watchers settle
    let strands = cwd.join("strands");
    fs::create_dir_all(&strands).unwrap();
    fs::write(strands.join("s1.md"), "strand one").unwrap();
    wait_for(
        || try_read_state(&rig)
            .and_then(|s| s["strand_queue"].as_array().map(|q| q.len() >= 1))
            .unwrap_or(false),
        10_000,
        "queue+ reflected in state.json",
    );

    // The mock pi exits instantly; wait for the run to finish.
    let tie_off = runtime_root(&rig).join("review-loom").join("tie-off-review.md");
    wait_for(|| tie_off.exists(), 15_000, "tie-off written");

    let (output, _graceful) = stop_service(child);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let states = state_lines(&stderr);

    // Exactly one baseline; the queue delta for s1 is logged; and the
    // queue- (drain) delta follows.
    assert_eq!(
        count_lines(&states, "initial snapshot"),
        1,
        "one baseline expected; state lines:\n  {}",
        states.join("\n  ")
    );
    assert_eq!(
        count_lines(&states, "change queue+"),
        1,
        "one queue+ expected; state lines:\n  {}",
        states.join("\n  ")
    );
    assert_eq!(
        count_lines(&states, "change queue-"),
        1,
        "one queue- expected; state lines:\n  {}",
        states.join("\n  ")
    );
    // Pure queue churn must not leak unchanged aspects into the delta.
    assert!(
        !states.iter().any(|l| l.contains("change profile")),
        "no profile delta expected; state lines:\n  {}",
        states.join("\n  ")
    );
    assert!(
        !states.iter().any(|l| l.contains("change loom")),
        "no loom delta expected; state lines:\n  {}",
        states.join("\n  ")
    );
}

// ── Test 3: burst — two strands, per-strand queue deltas ──────────────────

#[test]
fn burst_two_strands_queue_plus_then_minus() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    // Slow mock pi: both events stay queued long enough for a state
    // tick to observe the full burst.
    let rig = create_rig(cwd, "1.5");
    let pi = rig.join("bin").join("pi");

    let child = spawn_service(cwd, &pi);
    wait_for(
        || {
            try_read_state(&rig)
                .and_then(|s| s["looms"].as_array().map(|l| l.len()))
                == Some(1)
        },
        10_000,
        "baseline state.json",
    );
    thread::sleep(Duration::from_secs(1)); // let the watchers settle

    // Burst: two strands back to back.
    let strands = cwd.join("strands");
    fs::create_dir_all(&strands).unwrap();
    fs::write(strands.join("a.md"), "strand a").unwrap();
    fs::write(strands.join("b.md"), "strand b").unwrap();

    // Wait until both have drained.
    wait_for(
        || try_read_state(&rig)
            .and_then(|s| s["strand_queue"].as_array().map(|q| q.is_empty()))
            .unwrap_or(false),
        60_000,
        "both strands drained",
    );

    let (output, _graceful) = stop_service(child);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let states = state_lines(&stderr);

    // Both strands queued (+) and drained (−), one line each.
    assert_eq!(
        count_lines(&states, "change queue+"),
        2,
        "two queue+ expected; state lines:\n  {}",
        states.join("\n  ")
    );
    assert_eq!(
        count_lines(&states, "change queue-"),
        2,
        "two queue- expected; state lines:\n  {}",
        states.join("\n  ")
    );
    // Each strand path appears in its own + and − lines.
    let a = strands.join("a.md");
    let b = strands.join("b.md");
    assert!(states.iter().any(|l| {
        l.starts_with("change queue+") && l.contains(a.to_string_lossy().as_ref())
    }));
    assert!(states.iter().any(|l| {
        l.starts_with("change queue+") && l.contains(b.to_string_lossy().as_ref())
    }));
    assert!(states.iter().any(|l| {
        l.starts_with("change queue-") && l.contains(a.to_string_lossy().as_ref())
    }));
    assert!(states.iter().any(|l| {
        l.starts_with("change queue-") && l.contains(b.to_string_lossy().as_ref())
    }));
    // Still no baseline drift, no other aspects.
    assert_eq!(count_lines(&states, "initial snapshot"), 1);
    assert!(
        !states.iter().any(|l| l.contains("change profile")),
        "no profile delta expected; state lines:\n  {}",
        states.join("\n  ")
    );
    assert!(
        !states.iter().any(|l| l.contains("change loom")),
        "no loom delta expected; state lines:\n  {}",
        states.join("\n  ")
    );
}

// ── Test 4: step mode — baseline + delta lines, post-step state ───────────

/// Pre-queue an event by writing its queue file to
/// `tie-offs/dev-rig/events/<id>.json` (the disk is the queue; step
/// loads persisted events at startup).
fn queue_event(rig_dir: &Path, id: &str, strand_path: &Path) {
    let events = runtime_root(rig_dir).join("events");
    fs::create_dir_all(&events).unwrap();
    let json = serde_json::json!({
        "id": id,
        "kind": "Created",
        "loom_id": "review-loom",
        "knot_id": "review",
        "strand_path": strand_path.to_string_lossy(),
        "queued_at": "2026-01-01T00:00:00",
    });
    fs::write(events.join(format!("{id}.json")), json.to_string()).unwrap();
}

#[test]
fn step_mode_baseline_and_delta_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    let rig = create_rig(cwd, "0");
    let pi = rig.join("bin").join("pi");

    // The pre-queued event names a strand path the agent never reads
    // (the mock pi ignores args); it only has to be a valid path.
    let strands = cwd.join("strands");
    fs::create_dir_all(&strands).unwrap();
    let s1 = strands.join("s1.md");
    fs::write(&s1, "strand one").unwrap();
    queue_event(&rig, "1000000000001-aaaa", &s1);

    let output = Command::new(binary_path())
        .current_dir(cwd)
        .args(["step"])
        .env("KNOT_TEST_DEBOUNCE_MS", "20")
        .env("KNOT_TEST_CHECK_MS", "2")
        .env("KNOT_STATE_WRITE_MS", "100")
        .env("KNOT_TEST_CLI_PATH", pi)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should execute knot step");

    assert!(
        output.status.success(),
        "step must succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let events = event_lines(&stderr);
    let states = state_lines(&stderr);

    // One step session: discovery → processing → drain → shutdown, all
    // bracketed by LoomStarted / LoomStopped (KnotRegistered precedes
    // the LoomStarted bracket).
    assert_in_order(
        &events,
        &[
            "KnotRegistered loom=review-loom knot=review",
            "LoomStarted loom=review-loom",
            "KnotProcessing loom=review-loom knot=review strand=",
            "KnotCompleted loom=review-loom knot=review strand=",
            "StrandProcessed loom=review-loom strand=",
            "LoomStopped loom=review-loom",
        ],
        "step event lines",
    );

    // State lines: the baseline sees the loaded queue entry (queue=1),
    // then the drain and the status change.
    assert!(
        states.iter().any(|l| l.contains("initial snapshot") && l.contains("queue=1")),
        "baseline must show the loaded queue entry; state lines:\n  {}",
        states.join("\n  ")
    );
    assert_eq!(count_lines(&states, "initial snapshot"), 1);
    // The baseline precedes the deltas; the drain and the status delta
    // both land (their mutual order is tick-dependent).
    assert!(
        states.iter().any(|l| l.contains("initial snapshot")),
        "baseline expected; state lines:\n  {}",
        states.join("\n  ")
    );
    let snapshot_idx = states
        .iter()
        .position(|l| l.contains("initial snapshot"))
        .unwrap();
    assert!(
        states[snapshot_idx + 1..]
            .iter()
            .any(|l| l.contains("change queue-")),
        "queue drain expected after the baseline; state lines:\n  {}",
        states.join("\n  ")
    );
    assert!(
        states[snapshot_idx + 1..]
            .iter()
            .any(|l| l.contains("change knot review-loom/review: status")),
        "knot status delta expected after the baseline; state lines:\n  {}",
        states.join("\n  ")
    );
    assert!(
        states.iter().all(|l| !l.contains("change profile")),
        "no profile delta expected; state lines:\n  {}",
        states.join("\n  ")
    );

    // Post-step durable state: completed, queue empty.
    let state = read_state(&rig);
    let knots = state["looms"][0]["knots"].as_array().unwrap();
    assert_eq!(knots[0]["status"], "completed");
    assert!(
        state["strand_queue"].as_array().map(|q| q.is_empty()).unwrap_or(true),
        "queue must be drained after the step"
    );
}
