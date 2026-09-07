//! Queue entry identity self-heal — Phase 2 of plan 075: incident-
//! reproduction integration tests (full composition).
//!
//! Part-B style, mirroring `tests/late_removal.rs`: real
//! `DiskBackedEventQueue`, real `spawn_process_strand_loop` (service
//! mode) / `step_knot` (step mode), mock `pi` script, `wait_until`
//! helper. Queue events are pre-seeded by direct file writes to
//! `tie-offs/rig/events/` — the watcher-free pre-seeding technique
//! (plan 73 phase 4): the disk **is** the queue, and
//! `load_persisted()` picks the files up at startup with
//! deterministic FIFO order (no watcher/debounce wait).
//!
//! 1. `backdated_head_processes_first_and_drains` — the 2026-08-25
//!    incident repro: a head event whose queue file was renamed to an
//!    earlier timestamp (the 14:52 operator backdate — JSON id
//!    untouched). The pipeline must process the backdated head
//!    **first**, remove its queue file with no orphan, process the
//!    tail, drain, and idle cleanly (pre-fix: `QueueIdle` with the
//!    queue still full — a phantom head).
//! 2. `restart_over_duplicate_key_files_collapses_to_one` — the
//!    post-incident state: a renamed file plus a file carrying the
//!    same JSON id and the same dedup key. `load_persisted` must
//!    collapse the pair to one entry; it is processed once; no orphan
//!    file remains.
//! 3. `step_mode_renamed_head_does_not_panic` — the `step_knot` head
//!    path over a renamed head: the event is processed, exit 0, file
//!    removed (pins the `front`/`pop` failure path fixed in Phase 1).

mod helpers;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

// ── Mock `pi` script ─────────────────────────────────────────────────────

/// Mock `pi`: records one line per invocation (tagged with the
/// `@<path>` strand argument) and echoes a completion message naming
/// the strand, so both the runs log and the tie-off record which
/// strand ran.
const PI_RUNS: &str = "#!/usr/bin/env bash\n\
     cat > /dev/null\n\
     file=\"\"\n\
     for a in \"$@\"; do\n\
       case \"$a\" in @*) file=\"${a#@}\" ;; esac\n\
     done\n\
     echo \"run $file\" >> \"__RUNS_LOG__\"\n\
     echo \"review complete for $file\"\n\
     exit 0\n";

// ── Fixture ──────────────────────────────────────────────────────────────

struct RigFixture {
    _tmp: tempfile::TempDir,
    project_root: PathBuf,
    rig_dir: PathBuf,
    pi_path: PathBuf,
    runs_log: PathBuf,
}

/// Create `<tmp>/rig` (project root `<tmp>`) with one loom
/// (`review-loom`) and one knot (`review`, strand-dir `./strands`), a
/// mock `pi` binary, the rig config, and the `fast` profile.
///
/// `git_versioned` selects the knot's `git-versioned` frontmatter and,
/// when true, initialises a git repository at the project root (local
/// user config, signing off) so processing produces real commits.
fn setup_rig(git_versioned: bool) -> RigFixture {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path().to_path_buf();
    let rig_dir = project_root.join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    if git_versioned {
        init_git_repo(&project_root);
    }

    let runs_log = project_root.join("agent-runs.log");

    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script =
        PI_RUNS.replace("__RUNS_LOG__", &runs_log.display().to_string());
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
    fs::write(rig_dir.join("models.yml"), "# empty registry\n").unwrap();

    let loom_dir = rig_dir.join("review-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let git_flag = git_versioned.to_string();
    fs::write(
        loom_dir.join("review.md"),
        format!(
            "---\n\
             name: review\n\
             agent-profile-ref: fast\n\
             strand-dir: \"./strands\"\n\
             git-versioned: {git_flag}\n\
             ---\n\
             \n\
             Test knot: review.\n\
             \n"
        ),
    )
    .unwrap();

    helpers::create_fast_profile(&rig_dir);

    RigFixture {
        _tmp: tmp,
        project_root,
        rig_dir,
        pi_path,
        runs_log,
    }
}

// ── Git helpers ──────────────────────────────────────────────────────────

/// Run a git command in `dir`, panicking with stderr on failure.
fn run_git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git should be available on test system");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Initialise a git repository at `project_root` with local config so
/// commits work regardless of the machine's global git configuration.
fn init_git_repo(project_root: &Path) {
    run_git(project_root, &["init", "-b", "main"]);
    run_git(project_root, &["config", "user.email", "test@knot.test"]);
    run_git(project_root, &["config", "user.name", "Knot Test"]);
    run_git(project_root, &["config", "commit.gpgsign", "false"]);
}

/// The commit subjects of the project repo, newest first.
fn git_log_subjects(project_root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["log", "--format=%s"])
        .current_dir(project_root)
        .output()
        .expect("git should be available on test system");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect()
}

// ── Pre-seeding and observation helpers ─────────────────────────────────

/// Create a strand file in the project root's `strands/` directory
/// (where `strand-dir: "./strands"` resolves).
fn create_strand(fixture: &RigFixture, name: &str, content: &str) -> PathBuf {
    let strands = fixture.project_root.join("strands");
    fs::create_dir_all(&strands).unwrap();
    let path = strands.join(name);
    fs::write(&path, content).unwrap();
    path
}

/// The rig's runtime root (`tie-offs/rig/` under the project root).
fn runtime_root(fixture: &RigFixture) -> PathBuf {
    knot::domain::knot_file::derive_runtime_root(&fixture.rig_dir)
}

/// The queue directory (`tie-offs/rig/events/`).
fn events_dir(fixture: &RigFixture) -> PathBuf {
    runtime_root(fixture).join("events")
}

/// Pre-queue an event by writing its queue file directly to
/// `tie-offs/rig/events/{id}.json` — the disk is the queue, and
/// `load_persisted()` loads it at startup (deterministic FIFO, no
/// watcher/debounce wait). The `id` prefix controls FIFO order
/// (filename sort).
fn queue_event(
    fixture: &RigFixture,
    id: &str,
    loom_id: &str,
    knot_id: &str,
    strand_path: &Path,
) {
    let events = events_dir(fixture);
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

/// Poll `condition` every 50 ms until it returns true or the timeout
/// elapses.
fn wait_until<F>(condition: F, timeout_ms: u64, what: &str)
where
    F: Fn() -> bool,
{
    let deadline =
        std::time::Instant::now() + Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("timeout waiting for {what}");
}

/// Count the `.json` files currently on disk in the queue.
fn count_event_files(fixture: &RigFixture) -> usize {
    let events = events_dir(fixture);
    fs::read_dir(&events)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
                .count()
        })
        .unwrap_or(0)
}

/// The `strand_path` values of the tie-off completion sections for a
/// loom's knots, in processing order (plan 083 replacement for the
/// retired loom-log `KnotCompleted` events).
fn completed_strands(fixture: &RigFixture, loom_id: &str) -> Vec<String> {
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

/// The agent invocations recorded by the mock `pi` (one line per run,
/// tagged with the strand path).
fn agent_runs(fixture: &RigFixture) -> Vec<String> {
    fs::read_to_string(&fixture.runs_log)
        .map(|c| c.lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

/// The queue is drained: no event files on disk and an empty
/// `strand_queue` in `state.json` — the observable form of the
/// `QueueIdle` rig event (now in-memory only; the `[EVENT]` line is
/// asserted at the binary level in `tests/consolidated_log.rs`).
fn queue_drained(fixture: &RigFixture) -> bool {
    if count_event_files(fixture) != 0 {
        return false;
    }
    match helpers::read_state_file(&fixture.rig_dir) {
        Ok(state) => state["strand_queue"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(false),
        Err(_) => false,
    }
}

// ── Step-mode runner ─────────────────────────────────────────────────────

/// Run `knot::step_knot` (head path) on a dedicated thread with its
/// own tokio runtime — `step_knot` is finite, so the thread always
/// joins. Returns `(result, elapsed)`.
fn run_step(config: knot::AppConfig) -> (Result<(), std::io::Error>, Duration) {
    // Fast debounce timing — process-global env vars read by the
    // event pipeline at startup.
    unsafe {
        std::env::set_var("KNOT_TEST_DEBOUNCE_MS", "20");
        std::env::set_var("KNOT_TEST_CHECK_MS", "2");
    }

    let (tx, rx) = mpsc::channel::<Result<(), std::io::Error>>();
    let started = Instant::now();
    let handle = thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new()
            .expect("should create tokio runtime");
        let result = rt.block_on(knot::step_knot(config, None));
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(Duration::from_secs(120))
        .unwrap_or_else(|_| panic!("step_knot did not return within 120s (hang?)"));
    handle.join().expect("step thread panicked");
    (result, started.elapsed())
}

// ── Test 1: the incident repro ───────────────────────────────────────────

/// The 2026-08-25 incident repro: a head event whose queue file was
/// renamed to an earlier timestamp (the 14:52 operator backdate — the
/// JSON id left untouched) plus a queued tail. The pipeline must
/// process the backdated head **first**, remove its queue file with no
/// orphan left, process the tail, drain, and idle cleanly.
///
/// Pre-fix behaviour: `front()` resolves the head by the JSON id, the
/// renamed file's id no longer exists on disk, the failure is
/// swallowed, and the loop idles forever with the queue still full.
#[test]
fn backdated_head_processes_first_and_drains() {
    let f = setup_rig(true);
    let tail = create_strand(&f, "tail.md", "strand tail");
    let rectify = create_strand(&f, "rectify.md", "manual rectify strand");

    // E1 (tail): queued normally — filename == JSON id.
    queue_event(&f, "1000000000001-aaaa", "review-loom", "review", &tail);
    // E2 (the backdated head): queued later, then the operator
    // renames its file to an earlier timestamp, leaving the JSON id
    // untouched (the 14:52 rename from the incident).
    queue_event(&f, "1000000000002-bbbb", "review-loom", "review", &rectify);
    let events = events_dir(&f);
    fs::rename(
        events.join("1000000000002-bbbb.json"),
        events.join("1000000000000-rectify.json"),
    )
    .unwrap();

    let config = knot::AppConfig::with_rig_dir(f.rig_dir.clone())
        .with_cli_path(f.pi_path.clone());
    let handle = helpers::start_knot_with_config(config);
    helpers::wait_for_loom_in_state(&f.rig_dir, "review-loom", 1);

    // Both events processed and the queue drained (all event files
    // removed by late removal).
    wait_until(
        || completed_strands(&f, "review-loom").len() == 2
            && count_event_files(&f) == 0,
        30_000,
        "front loop to process the healed head and the tail",
    );

    // The backdated head was processed **first**, the tail second —
    // each exactly once.
    let completed = completed_strands(&f, "review-loom");
    assert_eq!(
        completed,
        vec![
            rectify.to_string_lossy().into_owned(),
            tail.to_string_lossy().into_owned(),
        ],
        "the backdated head must be processed first, then the tail: {completed:?}"
    );

    // E2's queue file is removed with **no orphan left**: neither the
    // renamed file (the incident's dangling-id orphan) nor a file
    // under the original JSON id survives.
    assert!(
        !events.join("1000000000000-rectify.json").exists(),
        "the renamed head file must be removed (no orphan)"
    );
    assert!(
        !events.join("1000000000002-bbbb.json").exists(),
        "no duplicate file under the original JSON id may remain"
    );
    assert_eq!(
        count_event_files(&f),
        0,
        "the events dir must be empty after the drain"
    );

    // The tie-off records the runs in the same order — the head run's
    // section was appended before the tail run's.
    let tie_off = runtime_root(&f)
        .join("review-loom")
        .join("tie-off-review.md");
    let content = fs::read_to_string(&tie_off).expect("tie-off must exist");
    let head_pos =
        content.find("rectify.md").expect("tie-off must cover the head");
    let tail_pos =
        content.find("tail.md").expect("tie-off must cover the tail");
    assert!(
        head_pos < tail_pos,
        "the head run's tie-off section must precede the tail run's:\n{content}"
    );

    // Git: one commit per run — the head's commit is the older one
    // (git log lists newest first, so the tail's commit is listed
    // first).
    let subjects = git_log_subjects(&f.project_root);
    assert_eq!(
        subjects.len(),
        2,
        "one commit per processed event: {subjects:?}"
    );
    assert!(
        subjects[0].contains("tail.md"),
        "the newest commit must be the tail run: {subjects:?}"
    );
    assert!(
        subjects[1].contains("rectify.md"),
        "the older commit must be the head run: {subjects:?}"
    );

    // The loop drained and went idle **cleanly**: QueueIdle in the
    // rig-log with the queue empty. (The pre-fix behaviour is
    // QueueIdle with the queue still full — a phantom head.)
    wait_until(
        || queue_drained(&f),
        10_000,
        "queue drained cleanly",
    );
    assert_eq!(
        count_event_files(&f),
        0,
        "the queue must be empty when the loop idles (not a phantom-head wedge)"
    );

    handle.abort();
}

// ── Test 2: restart over duplicate-key files ─────────────────────────────

/// Restart over the post-incident duplicate state: a renamed file
/// (name `1000000000000-rectify`, JSON id `1000000000002-bbbb`) plus
/// a file carrying the same JSON id and the same dedup key — the
/// exact state the incident's restart produced (dedup removal was a
/// no-op on the renamed file, and `write_event` wrote under the JSON
/// id). The pipeline's `load_persisted` must collapse the pair to one
/// entry for that key; it is processed once; no orphan file remains.
#[test]
fn restart_over_duplicate_key_files_collapses_to_one() {
    let f = setup_rig(false);
    let rectify = create_strand(&f, "rectify.md", "manual rectify strand");

    // The post-incident state: two files, one logical event. The
    // renamed file carries the backdated name; the restored file
    // carries the original id — same JSON id, same dedup key.
    let events = events_dir(&f);
    fs::create_dir_all(&events).unwrap();
    let json = serde_json::json!({
        "id": "1000000000002-bbbb",
        "kind": "Created",
        "loom_id": "review-loom",
        "knot_id": "review",
        "strand_path": rectify.to_string_lossy(),
        "queued_at": "2026-01-01T00:00:00",
    });
    fs::write(
        events.join("1000000000000-rectify.json"),
        json.to_string(),
    )
    .unwrap();
    fs::write(events.join("1000000000002-bbbb.json"), json.to_string())
        .unwrap();

    // Start the pipeline — `start_event_pipeline` runs
    // `load_persisted`, which must collapse the duplicate-key pair to
    // one entry (latest position wins).
    let config = knot::AppConfig::with_rig_dir(f.rig_dir.clone())
        .with_cli_path(f.pi_path.clone());
    let handle = helpers::start_knot_with_config(config);
    helpers::wait_for_loom_in_state(&f.rig_dir, "review-loom", 1);

    wait_until(
        || completed_strands(&f, "review-loom").len() == 1
            && count_event_files(&f) == 0,
        30_000,
        "the collapsed duplicate to be processed once and drained",
    );

    // Processed exactly once — the pair is one entry, not two.
    let completed = completed_strands(&f, "review-loom");
    assert_eq!(
        completed,
        vec![rectify.to_string_lossy().into_owned()],
        "the collapsed event must be processed exactly once: {completed:?}"
    );
    assert_eq!(
        agent_runs(&f).len(),
        1,
        "exactly one agent run for the collapsed event"
    );

    // No orphan file remains: neither the renamed file nor the file
    // under the original JSON id survives.
    assert!(
        !events.join("1000000000000-rectify.json").exists(),
        "no orphan may remain under the renamed file"
    );
    assert!(
        !events.join("1000000000002-bbbb.json").exists(),
        "no orphan may remain under the original JSON id"
    );
    assert_eq!(
        count_event_files(&f),
        0,
        "the events dir must be empty after the drain"
    );

    wait_until(
        || queue_drained(&f),
        10_000,
        "queue drained",
    );

    handle.abort();
}

// ── Test 3: step mode over a renamed head ────────────────────────────────

/// `step_knot`'s head path over a renamed head (filename ≠ JSON id):
/// the event is processed, the step exits 0, and the event file is
/// removed with no orphan. Pins the `front`/`pop` failure path fixed
/// in Phase 1 — pre-fix, `front()` wedged on the renamed head (the
/// step would declare the queue "empty" and exit 0 without
/// processing), and `pop()` panicked.
#[test]
fn step_mode_renamed_head_does_not_panic() {
    let f = setup_rig(false);
    let rectify = create_strand(&f, "rectify.md", "manual rectify strand");

    // One queued event, then the operator backdate: rename the file
    // to an earlier timestamp, leaving the JSON id untouched.
    queue_event(&f, "1000000000002-bbbb", "review-loom", "review", &rectify);
    let events = events_dir(&f);
    fs::rename(
        events.join("1000000000002-bbbb.json"),
        events.join("1000000000000-rectify.json"),
    )
    .unwrap();

    let config = knot::AppConfig::with_rig_dir(f.rig_dir.clone())
        .with_cli_path(f.pi_path.clone());
    let (result, _elapsed) = run_step(config);
    assert!(
        result.is_ok(),
        "step must succeed over a renamed head: {:?}",
        result.err()
    );

    // The event was processed (not skipped as "queue empty"): exactly
    // one agent run, for the renamed head's strand.
    let runs = agent_runs(&f);
    assert_eq!(
        runs.len(),
        1,
        "exactly one agent run expected, got: {runs:?}"
    );
    assert!(
        runs[0].contains("rectify.md"),
        "the renamed head's strand must have run: {runs:?}"
    );
    assert_eq!(
        completed_strands(&f, "review-loom"),
        vec![rectify.to_string_lossy().into_owned()],
        "one KnotCompleted for the head strand"
    );

    // The event file is removed (late removal); no orphan remains.
    assert!(
        !events.join("1000000000000-rectify.json").exists(),
        "the renamed head file must be removed"
    );
    assert!(
        !events.join("1000000000002-bbbb.json").exists(),
        "no file under the original JSON id may remain"
    );
    assert_eq!(
        count_event_files(&f),
        0,
        "the events dir must be empty after the step"
    );
}
