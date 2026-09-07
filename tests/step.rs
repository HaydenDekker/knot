//! Acceptance tests for the `knot step` lifecycle — Phase 4 of plan 073.
//!
//! Full startup, single execution, graceful stop. Two levels:
//!
//! **Lib-level** (via `knot::step_knot` on a dedicated thread + runtime):
//! - `step_processes_head_and_leaves_rest` — two queued events → exactly
//!   one agent run; head event file removed; second event still in
//!   `events/`; no tie-off for the second
//! - `step_event_targets_specific` — `--event` naming the *second* event
//!   → only that one processed; the head remains queued
//! - `step_captures_dispatched_events_without_executing` — the producer
//!   knot dispatches an event to a consumer during the step → after the
//!   step the consumer's event file is in `events/` (debounce flushed at
//!   shutdown) and the consumer's tie-off was not written
//! - `step_writes_state_json` — `state.json` reflects the post-step queue
//! - `step_does_not_clear_logs` — two sequential steps; the second step's
//!   loom-log still contains the first step's `KnotCompleted`
//!
//! **Binary-level** (via `CARGO_BIN_EXE_knot`, `tests/rig_cli.rs` pattern):
//! - `step_event_unknown_lists_queue_and_fails` — exit 1; stderr lists
//!   the queued ids
//! - `step_empty_queue_noop` — "queue empty", exit 0, no agent run,
//!   prompt exit
//! - `step_rig_flag_targets_named_rig` — `--rig` targets the named rig
//!   among several
//! - `step_multiple_rigs_without_flag_is_error` / `step_zero_rigs_is_error`
//!   — step rig resolution is stricter than the service (no implicit
//!   `rig/` creation)
//!
//! Queued events are pre-seeded by writing the queue files directly to
//! `tie-offs/<rig>/events/` — the disk is the queue, and
//! `load_persisted()` picks them up at step startup (deterministic FIFO,
//! no watcher/debounce wait).

mod helpers;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

// ── Mock `pi` scripts ──────────────────────────────────────────────────────

/// Mock `pi`: records one line per invocation (tagged with the `@<path>`
/// strand argument) and echoes a completion message naming the strand.
const PI_RUNS: &str = "#!/usr/bin/env bash\n\
     cat > /dev/null\n\
     file=\"\"\n\
     for a in \"$@\"; do\n\
       case \"$a\" in @*) file=\"${a#@}\" ;; esac\n\
     done\n\
     echo \"run $file\" >> \"__RUNS_LOG__\"\n\
     echo \"review complete for $file\"\n\
     exit 0\n";

/// Mock `pi` (producer): emits a tie-off containing a `PlanCreated`
/// event block — dispatched to consumer knots subscribed to
/// `event:plan-creator:PlanCreated` during the step.
const PI_DISPATCH: &str = "#!/usr/bin/env bash\n\
     cat > /dev/null\n\
     echo \"run @producer\" >> \"__RUNS_LOG__\"\n\
     echo \"Plan work complete.\"\n\
     echo\n\
     echo '```markdown'\n\
     echo \"---\"\n\
     echo \"event: PlanCreated\"\n\
     echo \"plan: PLAN-001\"\n\
     echo \"description: Feature plan created\"\n\
     echo \"---\"\n\
     echo\n\
     echo \"Plan created for feature.\"\n\
     echo '```'\n\
     exit 0\n";

// ── Fixture ────────────────────────────────────────────────────────────────

struct StepFixture {
    _tmp: tempfile::TempDir,
    rig_dir: PathBuf,
    pi_path: PathBuf,
    runs_log: PathBuf,
}

/// Create `<tmp>/rig` with a mock `pi` binary, the rig config, and the
/// `fast` profile. Knots and looms are added via [`write_knot_file`].
fn setup(pi_script: &str) -> StepFixture {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let runs_log = tmp.path().join("agent-runs.log");

    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script = pi_script.replace("__RUNS_LOG__", &runs_log.display().to_string());
    fs::write(&pi_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\n",
    )
    .unwrap();
    fs::write(rig_dir.join("models.yml"), "# empty registry\n")
        .unwrap();

    helpers::create_fast_profile(&rig_dir);

    StepFixture {
        _tmp: tmp,
        rig_dir,
        pi_path,
        runs_log,
    }
}

/// Write a knot definition file into `<rig>/<loom_id>/<knot_id>.md`.
fn write_knot_file(rig_dir: &Path, loom_id: &str, knot_id: &str, strand_dir: &str) {
    let loom_dir = rig_dir.join(loom_id);
    fs::create_dir_all(&loom_dir).unwrap();
    fs::write(
        loom_dir.join(format!("{knot_id}.md")),
        format!(
            "---\nname: {knot_id}\nagent-profile-ref: fast\nstrand-dir: \"{strand_dir}\"\ngit-versioned: false\n---\n\nTest knot: {knot_id}.\n"
        ),
    )
    .unwrap();
}

/// Create a strand file in the project root's `strands/` directory
/// (where `strand-dir: "./strands"` resolves).
fn create_strand(fixture: &StepFixture, name: &str, content: &str) -> PathBuf {
    let strands = fixture.rig_dir.parent().unwrap().join("strands");
    fs::create_dir_all(&strands).unwrap();
    let path = strands.join(name);
    fs::write(&path, content).unwrap();
    path
}

fn runtime_root(fixture: &StepFixture) -> PathBuf {
    knot::domain::knot_file::derive_runtime_root(&fixture.rig_dir)
}

/// Pre-queue an event by writing its queue file directly to
/// `tie-offs/<rig>/events/<id>.json` — the disk is the queue;
/// `load_persisted()` loads it at step startup. The `id` prefix
/// controls FIFO order (filename sort).
fn queue_event(
    fixture: &StepFixture,
    id: &str,
    loom_id: &str,
    knot_id: &str,
    strand_path: &Path,
) {
    let events = runtime_root(fixture).join("events");
    fs::create_dir_all(&events).unwrap();
    let json = serde_json::json!({
        "id": id,
        "kind": "Created",
        "loom_id": loom_id,
        "knot_id": knot_id,
        "strand_path": strand_path.to_string_lossy(),
        "queued_at": "2026-01-01T00:00:00",
    });
    fs::write(events.join(format!("{id}.json")), json.to_string())
        .unwrap();
}

/// List the event files currently on disk in the queue.
fn event_files(fixture: &StepFixture) -> Vec<PathBuf> {
    let events = runtime_root(fixture).join("events");
    fs::read_dir(&events)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Parse a queued event file from disk.
fn read_event_file(_fixture: &StepFixture, path: &Path) -> serde_json::Value {
    let content = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("failed to read event file {}: {e}", path.display())
    });
    serde_json::from_str(&content).unwrap_or_else(|e| {
        panic!("failed to parse event file {}: {e}", path.display())
    })
}

/// Count the agent invocations recorded by the mock `pi`.
fn agent_runs(fixture: &StepFixture) -> Vec<String> {
    fs::read_to_string(&fixture.runs_log)
        .map(|c| c.lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

// ── Lib-level runner ───────────────────────────────────────────────────────

/// Run `knot::step_knot` on a dedicated thread with its own tokio
/// runtime (mirrors `helpers::start_knot_with_config`, but `step_knot`
/// is finite — the thread always joins). Returns `(result, elapsed)`.
fn run_step(
    config: knot::AppConfig,
    event_spec: Option<&str>,
) -> (Result<(), std::io::Error>, Duration) {
    // Fast debounce timing — process-global env vars read by the
    // event pipeline at startup (all tests use the same values).
    unsafe {
        std::env::set_var("KNOT_TEST_DEBOUNCE_MS", "20");
        std::env::set_var("KNOT_TEST_CHECK_MS", "2");
    }

    let spec = event_spec.map(|s| s.to_string());
    let (tx, rx) = mpsc::channel::<Result<(), std::io::Error>>();
    let started = Instant::now();
    let handle = thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("should create tokio runtime");
        let result = rt.block_on(knot::step_knot(config, spec));
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(Duration::from_secs(120))
        .unwrap_or_else(|_| panic!("step_knot did not return within 120s (hang?)"));
    handle.join().expect("step thread panicked");
    (result, started.elapsed())
}

/// The `strand_path` values of the tie-off completion sections for a
/// loom's knots (plan 083 replacement for the retired loom-log
/// `KnotCompleted` events). Tie-offs persist and accumulate across
/// `knot step` runs, so the sections are the durable per-run record
/// for in-process tests.
fn completed_strands(fixture: &StepFixture, loom_id: &str) -> Vec<String> {
    let dir = runtime_root(fixture).join(loom_id);
    let mut strands: Vec<String> = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return strands;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("tie-off-") || !name.ends_with(".md") {
            continue;
        }
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        // Section header: `## {knot} triggered by {event} {strand}`.
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("## ") {
                if let Some(after) = rest.split(" triggered by ").nth(1) {
                    if let Some(strand) = after.rsplit_once(' ') {
                        strands.push(strand.1.to_string());
                    }
                }
            }
        }
    }
    strands
}

// ── Lib-level tests ────────────────────────────────────────────────────────

/// Two queued events → exactly one agent run; the head event file is
/// removed (late removal), the second event is still in `events/`, and
/// the second event produced no tie-off and no completion.
#[test]
fn step_processes_head_and_leaves_rest() {
    let f = setup(PI_RUNS);
    write_knot_file(&f.rig_dir, "review-loom", "review", "./strands");
    let a = create_strand(&f, "a.md", "strand a");
    let b = create_strand(&f, "b.md", "strand b");
    queue_event(&f, "1000000000001-aaaa", "review-loom", "review", &a);
    queue_event(&f, "1000000000002-bbbb", "review-loom", "review", &b);

    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    let (result, _elapsed) = run_step(config, None);
    assert!(result.is_ok(), "step should succeed: {:?}", result.err());

    // Exactly one agent run — the head only.
    let runs = agent_runs(&f);
    assert_eq!(runs.len(), 1, "exactly one agent run expected, got: {runs:?}");
    assert!(runs[0].contains("a.md"), "head strand must have run: {runs:?}");

    // Head event file removed (late removal), second event intact.
    let events = event_files(&f);
    assert_eq!(
        events.len(),
        1,
        "second event must remain queued; events: {events:?}"
    );
    assert_eq!(
        events[0]
            .file_name()
            .and_then(|n| n.to_str()),
        Some("1000000000002-bbbb.json"),
        "the remaining event must be the second one: {events:?}"
    );
    let remaining = read_event_file(&f, &events[0]);
    assert_eq!(
        remaining.get("strand_path").and_then(|v| v.as_str()),
        Some(b.to_string_lossy().as_ref()),
        "remaining event must target the second strand"
    );

    // One completion, for the head strand only — no tie-off for the
    // second event.
    let completed = completed_strands(&f, "review-loom");
    assert_eq!(
        completed,
        vec![a.to_string_lossy().into_owned()],
        "exactly one KnotCompleted (head strand): {completed:?}"
    );
    let tie_off = runtime_root(&f).join("review-loom").join("tie-off-review.md");
    let content = fs::read_to_string(&tie_off).expect("producer tie-off must exist");
    assert!(content.contains("a.md"), "tie-off must cover the head strand");
    assert!(
        !content.contains("b.md"),
        "no tie-off content for the unprocessed second strand: {content}"
    );
}

/// `--event` naming the *second* (non-head) event processes only that
/// one; the head remains queued. Also pins the `.json`-optional exact-id
/// match.
#[test]
fn step_event_targets_specific() {
    let f = setup(PI_RUNS);
    write_knot_file(&f.rig_dir, "review-loom", "review", "./strands");
    let a = create_strand(&f, "a.md", "strand a");
    let b = create_strand(&f, "b.md", "strand b");
    queue_event(&f, "1000000000001-aaaa", "review-loom", "review", &a);
    queue_event(&f, "1000000000002-bbbb", "review-loom", "review", &b);

    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    // Exact id with the optional `.json` suffix, naming the second event.
    let (result, _elapsed) = run_step(config, Some("1000000000002-bbbb.json"));
    assert!(result.is_ok(), "step should succeed: {:?}", result.err());

    // Only the second event ran.
    let runs = agent_runs(&f);
    assert_eq!(runs.len(), 1, "exactly one agent run expected, got: {runs:?}");
    assert!(runs[0].contains("b.md"), "second strand must have run: {runs:?}");

    // The second event file is gone; the head remains queued.
    let events = event_files(&f);
    assert_eq!(events.len(), 1, "head event must remain queued: {events:?}");
    assert_eq!(
        events[0]
            .file_name()
            .and_then(|n| n.to_str()),
        Some("1000000000001-aaaa.json"),
        "the remaining event must be the head: {events:?}"
    );

    // One completion, for the second strand only.
    let completed = completed_strands(&f, "review-loom");
    assert_eq!(
        completed,
        vec![b.to_string_lossy().into_owned()],
        "exactly one KnotCompleted (second strand): {completed:?}"
    );
}

/// The producer knot dispatches an event to a consumer *during* the
/// step: after the step, the consumer's event file is in `events/`
/// (watcher → debounce buffer → flushed at shutdown) and the consumer's
/// tie-off was **not** written (captured, not executed).
#[test]
fn step_captures_dispatched_events_without_executing() {
    let f = setup(PI_DISPATCH);
    write_knot_file(&f.rig_dir, "planning-loom", "plan-creator", "./strands");
    write_knot_file(
        &f.rig_dir,
        "validation-loom",
        "gap-assessor",
        "event:plan-creator:PlanCreated",
    );
    let plan = create_strand(&f, "plan.md", "feature plan");
    queue_event(
        &f,
        "1000000000001-aaaa",
        "planning-loom",
        "plan-creator",
        &plan,
    );

    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    let (result, _elapsed) = run_step(config, None);
    assert!(result.is_ok(), "step should succeed: {:?}", result.err());

    // Exactly one agent run — the producer. The consumer never ran.
    let runs = agent_runs(&f);
    assert_eq!(runs.len(), 1, "producer ran exactly once, got: {runs:?}");

    // The producer's head event was removed (late removal).
    let producer_event = runtime_root(&f)
        .join("events")
        .join("1000000000001-aaaa.json");
    assert!(
        !producer_event.exists(),
        "producer event file must be removed after processing"
    );

    // The dispatched consumer event is captured in the queue — flushed
    // to disk at shutdown (the debounce buffer held it until the channel
    // closed).
    let events = event_files(&f);
    assert_eq!(
        events.len(),
        1,
        "the dispatched consumer event must be queued; events: {events:?}"
    );
    let ev = read_event_file(&f, &events[0]);
    assert_eq!(
        ev.get("loom_id").and_then(|v| v.as_str()),
        Some("validation-loom"),
        "queued event must target the consumer loom: {ev:?}"
    );
    assert_eq!(
        ev.get("knot_id").and_then(|v| v.as_str()),
        Some("gap-assessor"),
        "queued event must target the consumer knot: {ev:?}"
    );
    // The dispatcher creates the event file atomically (temp file +
    // rename), so the watcher sees Create *and* Modify for the final
    // path; the debounce window coalesces them to the last one —
    // either kind is a valid captured dispatch.
    let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        kind == "Created" || kind == "Modified",
        "dispatched event kind must be Created or Modified, got: {kind}"
    );
    let strand_path = ev
        .get("strand_path")
        .and_then(|v| v.as_str())
        .expect("queued event must carry the dispatched strand path");
    assert!(
        strand_path.contains("validation-loom/PlanCreated/event-"),
        "strand path must be the dispatched event file: {strand_path}"
    );

    // The consumer was captured, NOT executed: no tie-off, no
    // completion (the consumer's tie-off absence is the plan-083
    // observable of "no KnotProcessing / KnotCompleted" — the in-run
    // event lines are process-private).
    let consumer_tie_off = runtime_root(&f)
        .join("validation-loom")
        .join("tie-off-gap-assessor.md");
    assert!(
        !consumer_tie_off.exists(),
        "consumer tie-off must NOT be written (captured, not executed)"
    );

    // The producer completed and its tie-off carries the event block.
    let producer_tie_off = runtime_root(&f)
        .join("planning-loom")
        .join("tie-off-plan-creator.md");
    let content =
        fs::read_to_string(&producer_tie_off).expect("producer tie-off must exist");
    assert!(
        content.contains("event: PlanCreated"),
        "producer tie-off must contain the dispatched event block: {content}"
    );
}

/// `state.json` reflects the post-step queue: the state writer ran
/// during the step, so the snapshot lists the remaining (unprocessed)
/// event and not the processed head.
#[test]
fn step_writes_state_json() {
    let f = setup(PI_RUNS);
    write_knot_file(&f.rig_dir, "review-loom", "review", "./strands");
    let a = create_strand(&f, "a.md", "strand a");
    let b = create_strand(&f, "b.md", "strand b");
    queue_event(&f, "1000000000001-aaaa", "review-loom", "review", &a);
    queue_event(&f, "1000000000002-bbbb", "review-loom", "review", &b);

    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    let (result, _elapsed) = run_step(config, None);
    assert!(result.is_ok(), "step should succeed: {:?}", result.err());

    let state_path = runtime_root(&f).join("state.json");
    assert!(
        state_path.exists(),
        "state.json must exist after the step"
    );
    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_path).unwrap())
            .expect("state.json must parse");

    // The loom and its knot are in the snapshot.
    let looms = state
        .get("looms")
        .and_then(|v| v.as_array())
        .expect("state must list looms");
    let loom = looms
        .iter()
        .find(|l| l.get("id").and_then(|v| v.as_str()) == Some("review-loom"))
        .expect("review-loom must be in state");
    let knots = loom.get("knots").and_then(|v| v.as_array()).unwrap();
    assert!(
        knots.iter().any(|k| k.get("id").and_then(|v| v.as_str()) == Some("review")),
        "knot must be in state: {knots:?}"
    );

    // The queue snapshot reflects the post-step state: the remaining
    // event (b.md) is listed, the processed head (a.md) is not.
    let queue = state
        .get("strand_queue")
        .and_then(|v| v.as_array())
        .expect("state must carry the strand queue snapshot");
    let paths: Vec<&str> = queue
        .iter()
        .filter_map(|e| e.get("strand_path").and_then(|v| v.as_str()))
        .collect();
    assert!(
        paths.iter().any(|p| p.ends_with("b.md")),
        "the remaining event must be in the state queue: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.ends_with("a.md")),
        "the processed head must NOT be in the state queue: {paths:?}"
    );
}

/// Step mode does not clear the logs: two sequential steps — the second
/// step's full startup (with `clear_logs: false`) leaves the first
/// step's `KnotCompleted` in the loom-log.
#[test]
fn step_does_not_clear_logs() {
    let f = setup(PI_RUNS);
    write_knot_file(&f.rig_dir, "review-loom", "review", "./strands");
    let a = create_strand(&f, "a.md", "strand a");
    queue_event(&f, "1000000000001-aaaa", "review-loom", "review", &a);

    // Step 1 — processes a.md.
    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    let (result, _elapsed) = run_step(config, None);
    assert!(result.is_ok(), "step 1 should succeed: {:?}", result.err());
    assert_eq!(
        completed_strands(&f, "review-loom"),
        vec![a.to_string_lossy().into_owned()],
        "step 1 must complete the head strand"
    );

    // Step 2 — a fresh step_knot (full startup again) processes b.md.
    let b = create_strand(&f, "b.md", "strand b");
    queue_event(&f, "1000000000002-bbbb", "review-loom", "review", &b);
    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    let (result, _elapsed) = run_step(config, None);
    assert!(result.is_ok(), "step 2 should succeed: {:?}", result.err());

    // Both completions survive — the second step's startup did not
    // clear the loom-log.
    let completed = completed_strands(&f, "review-loom");
    assert!(
        completed.iter().any(|p| p.ends_with("a.md")),
        "step 1's KnotCompleted must survive step 2's startup: {completed:?}"
    );
    assert!(
        completed.iter().any(|p| p.ends_with("b.md")),
        "step 2's KnotCompleted must be present: {completed:?}"
    );
    assert_eq!(
        completed.len(),
        2,
        "exactly two completions across the two steps: {completed:?}"
    );

    // Each step is a self-contained run: the per-step brackets
    // (LoomStarted / LoomStopped) are in-memory per process now
    // (plan 083) — the durable per-run evidence is the accumulated
    // tie-off sections asserted above (one section per step's
    // completion; the binary-level test in tests/consolidated_log.rs
    // pins the per-run `[EVENT]` bracket on captured stderr).
}

// ── Binary-level tests ─────────────────────────────────────────────────────

/// Resolve the path to the compiled `knot` binary.
fn binary_path() -> String {
    std::env::var("CARGO_BIN_EXE_knot").unwrap_or_else(|_| {
        format!(
            "{}/target/debug/knot",
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string())
        )
    })
}

/// Run the `knot` binary in `cwd` with fast debounce timing and return
/// `(output, elapsed)`. `step` always terminates (no service loop).
fn run_knot(cwd: &Path, args: &[&str]) -> (Output, Duration) {
    let started = Instant::now();
    let output = Command::new(binary_path())
        .current_dir(cwd)
        .args(args)
        .env("KNOT_TEST_DEBOUNCE_MS", "20")
        .env("KNOT_TEST_CHECK_MS", "2")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("should execute knot binary");
    (output, started.elapsed())
}

/// Create a minimal rig directory (one knot, the fast profile, a mock
/// `pi`) at `cwd/<name>`.
fn create_minimal_rig(cwd: &Path, name: &str) -> PathBuf {
    let rig = cwd.join(name);
    fs::create_dir_all(&rig).unwrap();
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
    let bin = rig.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let pi = bin.join("pi");
    fs::write(
        &pi,
        "#!/usr/bin/env bash\ncat > /dev/null\necho \"ok\"\nexit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi, fs::Permissions::from_mode(0o755)).unwrap();
    }
    rig
}

/// `--event` naming an unknown event: exit 1, and stderr lists the
/// queued event ids (the user-facing listing of what *is* queued).
#[test]
fn step_event_unknown_lists_queue_and_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    // The rig must match the `*-rig` discovery pattern (a bare `rig/`
    // is the service default only — step has no default fallback).
    let rig = create_minimal_rig(cwd, "dev-rig");

    // One queued event (strand file created for realism).
    let strands = cwd.join("strands");
    fs::create_dir_all(&strands).unwrap();
    let strand = strands.join("a.md");
    fs::write(&strand, "strand a").unwrap();
    let events = knot::domain::knot_file::derive_runtime_root(&rig).join("events");
    fs::create_dir_all(&events).unwrap();
    let json = serde_json::json!({
        "id": "1000000000001-aaaa",
        "kind": "Created",
        "loom_id": "review-loom",
        "knot_id": "review",
        "strand_path": strand.to_string_lossy(),
        "queued_at": "2026-01-01T00:00:00",
    });
    fs::write(events.join("1000000000001-aaaa.json"), json.to_string()).unwrap();

    let (output, _elapsed) = run_knot(cwd, &["step", "--event", "no-such-event"]);
    assert!(
        !output.status.success(),
        "unknown --event must exit 1; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no queued event matches"),
        "stderr should explain the no-match: {stderr}"
    );
    assert!(
        stderr.contains("1000000000001-aaaa"),
        "stderr must list the queued event id: {stderr}"
    );

    // The unknown-event failure must not have consumed the queued
    // event (it was never processed).
    assert!(
        events.join("1000000000001-aaaa.json").exists(),
        "the queued event must remain on disk after a no-match failure"
    );
}

/// Empty queue: "queue empty" on stdout, exit 0, no agent run, and a
/// prompt exit (the step does not enter the service loop).
///
/// Plan 083: the run's events are asserted on the captured stderr
/// (`[KNOT][EVENT]` lines) — no loom-log file exists any more.
#[test]
fn step_empty_queue_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    let rig = create_minimal_rig(cwd, "dev-rig");

    let (output, elapsed) = run_knot(cwd, &["step"]);
    assert!(
        output.status.success(),
        "empty queue must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("queue empty"),
        "stdout should announce the empty queue: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    // No agent run — the event lines show discovery + shutdown only.
    assert!(
        !stderr.contains("KnotProcessing") && !stderr.contains("KnotCompleted"),
        "no processing must have happened; stderr: {stderr}"
    );
    assert!(
        stderr.contains("[KNOT][EVENT] LoomStopped loom=review-loom"),
        "LoomStopped must be written on the graceful shutdown; stderr: {stderr}"
    );
    // Baseline state write: `initial snapshot` line, no deltas.
    assert!(
        stderr.contains("[KNOT][STATE] initial snapshot"),
        "the baseline state write must be logged; stderr: {stderr}"
    );

    // Plan 083: no log files under the runtime root.
    let runtime_root = knot::domain::knot_file::derive_runtime_root(&rig);
    assert!(
        !runtime_root.join("review-loom").join(".loom-log").exists(),
        ".loom-log must not be created"
    );
    assert!(
        !runtime_root.join(".rig-log").exists(),
        ".rig-log must not be created"
    );

    // Prompt exit — the step returns once the shutdown cascade is done
    // (5-second drain safety net dominates; a hang would be much longer).
    assert!(
        elapsed < Duration::from_secs(30),
        "step must exit promptly, took {elapsed:?}"
    );
}

/// `--rig` targets the named rig among several — no multiple-rigs
/// error, and only the named rig is started (its runtime root gets
/// `state.json`; the other rig is untouched).
#[test]
fn step_rig_flag_targets_named_rig() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    create_minimal_rig(cwd, "dev-rig");
    create_minimal_rig(cwd, "other-rig");

    let (output, _elapsed) = run_knot(cwd, &["step", "--rig", "dev-rig"]);
    assert!(
        output.status.success(),
        "step --rig dev-rig must succeed (empty queue is exit 0); stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let dev_state = cwd.join("tie-offs").join("dev-rig").join("state.json");
    assert!(
        dev_state.exists(),
        "the named rig must be started (state.json written)"
    );
    assert!(
        !cwd.join("tie-offs").join("other-rig").exists(),
        "the other rig must not be started"
    );
}

/// Multiple rigs in the cwd and no `--rig`: step resolution is an
/// error (stricter than the service — no default fallthrough).
#[test]
fn step_multiple_rigs_without_flag_is_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();
    create_minimal_rig(cwd, "dev-rig");
    create_minimal_rig(cwd, "review-rig");

    let (output, _elapsed) = run_knot(cwd, &["step"]);
    assert!(
        !output.status.success(),
        "multiple rigs without --rig must exit 1"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("dev-rig"), "stderr lists dev-rig: {stderr}");
    assert!(
        stderr.contains("review-rig"),
        "stderr lists review-rig: {stderr}"
    );
    assert!(
        stderr.contains("--rig"),
        "stderr should hint at --rig: {stderr}"
    );
}

/// Zero rigs in the cwd: step resolution is an error and **no implicit
/// `rig/` is created** (unlike the service, which defaults to `rig/`).
#[test]
fn step_zero_rigs_is_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    let (output, _elapsed) = run_knot(cwd, &["step"]);
    assert!(
        !output.status.success(),
        "zero rigs must exit 1"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no rigs found"),
        "stderr should explain the empty discovery: {stderr}"
    );
    assert!(
        !cwd.join("rig").exists(),
        "step must NOT create an implicit rig/"
    );
}
