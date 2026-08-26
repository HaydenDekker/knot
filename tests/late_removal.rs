//! Late-removal (at-least-once) queue semantics — Phase 2 of plan 073.
//!
//! **Part A — use-case level (mocked ports).** Pins the invariant:
//! *every* return from `execute_with_pending` removes the queued event
//! exactly once, and on success the removal precedes the git commit.
//! The queue-less `execute()` entry point never touches the queue.
//!
//! **Part B — full composition (real runtime + mock `pi`).** Pins the
//! front-based service loop: it drains a multi-event queue, the event
//! file survives while the agent is running (crash window — a restart
//! re-queues instead of losing the event), and a failing knot's event
//! is consumed exactly once (no poison-pill re-fail loop).

mod helpers;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use helpers::ProcessStrandBuilder;
use knot::application::in_memory_event_queue::InMemoryEventQueue;
use knot::application::ports::{
    AgentOutput, GitVersioningPort, LoomLogPort, PortError, StrandEventQueue,
};
use knot::application::store::LoomStore;
use knot::application::usecases::test_fixtures::*;
use knot::application::usecases::ProcessStrand;
use knot::domain::entities::{Knot, KnotId, Loom, LoomId, StrandPath};
use knot::domain::events::{LoomEvent, StrandEvent, StrandQueueAccessor};
use knot::domain::pending_event::{PendingEvent, PendingEventId};
use knot::domain::value_objects::RigAgentConfig;

// ── Shared fixtures (mirror tests/pipeline.rs) ───────────────────────────

fn build_knot(id: &str) -> Knot {
    build_knot_with_profile(id, "fast")
}

fn build_loom(id: &str, knots: Vec<Knot>) -> Loom {
    Loom {
        id: LoomId(id.to_string()),
        knots,
    }
}

fn success_runner(output: &str) -> Arc<MockAgentRunner> {
    Arc::new(MockAgentRunner::new(Ok(AgentOutput {
        stdout: output.to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    })))
}

fn failure_runner(message: &str) -> Arc<MockAgentRunner> {
    Arc::new(MockAgentRunner::new(Err(PortError::AgentExecutionFailed {
        message: message.to_string(),
        session_id: None,
    })))
}

fn create_strand_file(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, "content").unwrap();
    path
}

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Build a `PendingEvent` with a unique ID for the given loom/knot/strand.
fn make_pending(loom_id: &str, knot_id: &str, strand_path: &Path) -> PendingEvent {
    PendingEvent {
        id: PendingEventId(format!(
            "1000000000000-{:04x}",
            EVENT_COUNTER.fetch_add(1, Ordering::SeqCst)
        )),
        kind: "Created".to_string(),
        loom_id: loom_id.to_string(),
        knot_id: knot_id.to_string(),
        strand_path: strand_path.to_string_lossy().into_owned(),
        queued_at: "2026-01-01T00:00:00".to_string(),
    }
}

// ── Part A: use-case level (mocked ports) ────────────────────────────────

/// On success the queued event is removed exactly once — the queue is
/// empty after `execute_with_pending` returns `Ok`.
#[test]
fn success_removes_event_file() {
    let dir = tempfile::tempdir().unwrap();
    let strand = create_strand_file(&dir, "feature.md");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let queue = Arc::new(InMemoryEventQueue::new());

    let helpers::ProcessStrandResult { strand: use_case, .. } =
        ProcessStrandBuilder::new(loom, success_runner("review output"))
            .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
            .build();

    let pending = make_pending("review-loom", "review", &strand);
    queue.push(pending.clone());

    let result = use_case.execute_with_pending(&pending);
    assert!(result.is_ok(), "success path should return Ok: {:?}", result);
    assert!(
        queue.is_empty(),
        "event must be removed on success (late removal)"
    );
}

/// On agent failure the event is consumed at the point of failure —
/// no poison-pill retry loop.
#[test]
fn agent_failure_removes_event_file() {
    let dir = tempfile::tempdir().unwrap();
    let strand = create_strand_file(&dir, "feature.md");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let queue = Arc::new(InMemoryEventQueue::new());

    let helpers::ProcessStrandResult {
        strand: use_case,
        log_events,
        ..
    } = ProcessStrandBuilder::new(loom, failure_runner("agent crash"))
        .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
        .build();

    let pending = make_pending("review-loom", "review", &strand);
    queue.push(pending.clone());

    // Agent failure is handled internally (KnotFailed) — returns Ok.
    let result = use_case.execute_with_pending(&pending);
    assert!(result.is_ok());
    assert!(
        queue.is_empty(),
        "event must be consumed on agent failure"
    );

    let events = log_events.lock().unwrap();
    assert!(
        events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. })),
        "KnotFailed should be logged on agent failure"
    );
}

/// Profile-not-found is an early error — the event is still consumed.
#[test]
fn profile_not_found_removes_event_file() {
    let dir = tempfile::tempdir().unwrap();
    let strand = create_strand_file(&dir, "feature.md");

    // Knot references a profile that does not exist in the repository
    // (the builder only registers "fast").
    let loom = build_loom(
        "review-loom",
        vec![build_knot_with_profile("review", "ghost")],
    );
    let queue = Arc::new(InMemoryEventQueue::new());

    let helpers::ProcessStrandResult { strand: use_case, .. } =
        ProcessStrandBuilder::new(loom, success_runner("unused"))
            .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
            .build();

    let pending = make_pending("review-loom", "review", &strand);
    queue.push(pending.clone());

    let result = use_case.execute_with_pending(&pending);
    assert!(result.is_err(), "profile-not-found should return Err");
    assert!(
        queue.is_empty(),
        "event must be consumed on profile-not-found"
    );
}

/// Loom-not-found is an early error — the event is still consumed.
#[test]
fn loom_not_found_removes_event_file() {
    let dir = tempfile::tempdir().unwrap();
    let strand = create_strand_file(&dir, "feature.md");

    // Loom "review-loom" is registered; the event targets "ghost-loom".
    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let queue = Arc::new(InMemoryEventQueue::new());

    let helpers::ProcessStrandResult { strand: use_case, .. } =
        ProcessStrandBuilder::new(loom, success_runner("unused"))
            .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
            .build();

    let pending = make_pending("ghost-loom", "review", &strand);
    queue.push(pending.clone());

    let result = use_case.execute_with_pending(&pending);
    assert!(
        matches!(result, Err(PortError::LoomNotFound(_))),
        "should be LoomNotFound, got: {:?}",
        result
    );
    assert!(
        queue.is_empty(),
        "event must be consumed on loom-not-found"
    );
}

/// Binary-file skip consumes the event and never invokes the agent.
#[test]
fn binary_skip_removes_event_file() {
    let dir = tempfile::tempdir().unwrap();
    let strand = create_strand_file(&dir, "data.bin");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let queue = Arc::new(InMemoryEventQueue::new());

    let result = ProcessStrandBuilder::new(loom, success_runner("unused"))
        .with_tracking_file_checker()
        .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
        .build();
    let file_checker = result.file_checker.as_ref().expect("file_checker");
    file_checker.mark_binary(&strand);
    let helpers::ProcessStrandResult {
        strand: use_case,
        agent_runner: captured,
        ..
    } = result;

    let pending = make_pending("review-loom", "review", &strand);
    queue.push(pending.clone());

    let r = use_case.execute_with_pending(&pending);
    assert!(r.is_ok(), "skip path returns Ok");
    assert!(
        queue.is_empty(),
        "event must be consumed on binary skip"
    );
    assert!(
        captured.get_captured_contexts().is_empty(),
        "agent must not run for a skipped (binary) strand"
    );
}

/// An event with an unknown kind can never be processed — it is
/// consumed (poison-pill avoidance) and the error propagates.
#[test]
fn unknown_kind_consumes_event() {
    let dir = tempfile::tempdir().unwrap();
    let strand = create_strand_file(&dir, "feature.md");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let queue = Arc::new(InMemoryEventQueue::new());

    let helpers::ProcessStrandResult { strand: use_case, .. } =
        ProcessStrandBuilder::new(loom, success_runner("unused"))
            .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
            .build();

    let mut pending = make_pending("review-loom", "review", &strand);
    pending.kind = "Unknown".to_string();
    queue.push(pending.clone());

    let result = use_case.execute_with_pending(&pending);
    assert!(result.is_err(), "unknown kind should return Err");
    assert!(
        queue.is_empty(),
        "unconvertible event must be consumed (no poison-pill loop)"
    );
}

/// The queue-less `execute()` entry point (no event ID) never touches
/// the queue — existing callers are unaffected by late removal.
#[test]
fn execute_without_pending_id_leaves_queue_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let strand = create_strand_file(&dir, "feature.md");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let queue = Arc::new(InMemoryEventQueue::new());

    let helpers::ProcessStrandResult { strand: use_case, .. } =
        ProcessStrandBuilder::new(loom, success_runner("review output"))
            .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
            .build();

    let pending = make_pending("review-loom", "review", &strand);
    queue.push(pending);
    let event = StrandEvent::Created {
        loom_id: LoomId("review-loom".to_string()),
        knot_id: KnotId("review".to_string()),
        strand_path: StrandPath(strand),
    };

    let result = use_case.execute(event);
    assert!(result.is_ok());
    assert_eq!(
        queue.len(),
        1,
        "execute() with no event ID must not remove the queued event"
    );
}

// ── Part A: ordering (delete before commit) ──────────────────────────────

/// Records the kind of every loom-log append into a shared ordering log.
#[derive(Clone)]
struct OrderingLoomLog {
    order: Arc<Mutex<Vec<String>>>,
}

fn loom_event_name(e: &LoomEvent) -> &'static str {
    match e {
        LoomEvent::KnotRegistered { .. } => "KnotRegistered",
        LoomEvent::LoomStarted { .. } => "LoomStarted",
        LoomEvent::LoomStopped { .. } => "LoomStopped",
        LoomEvent::StrandProcessed { .. } => "StrandProcessed",
        LoomEvent::KnotProcessing { .. } => "KnotProcessing",
        LoomEvent::KnotCompleted { .. } => "KnotCompleted",
        LoomEvent::KnotFailed { .. } => "KnotFailed",
        LoomEvent::KnotDeregistered { .. } => "KnotDeregistered",
        LoomEvent::KnotParseWarning { .. } => "KnotParseWarning",
        LoomEvent::DirectoryCreated { .. } => "DirectoryCreated",
        LoomEvent::StrandIgnored { .. } => "StrandIgnored",
        LoomEvent::StrandSkipped { .. } => "StrandSkipped",
        LoomEvent::SessionResumed { .. } => "SessionResumed",
        LoomEvent::KnotEmptyResponse { .. } => "KnotEmptyResponse",
        LoomEvent::EventsDispatched { .. } => "EventsDispatched",
        LoomEvent::KnotEventsMissing { .. } => "KnotEventsMissing",
        LoomEvent::ContextCompacted { .. } => "ContextCompacted",
    }
}

impl LoomLogPort for OrderingLoomLog {
    fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
        Ok(())
    }

    fn append(&self, event: LoomEvent) -> Result<(), PortError> {
        self.order
            .lock()
            .unwrap()
            .push(format!("log:{}", loom_event_name(&event)));
        Ok(())
    }

    fn read_all(&self, _loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
        Ok(Vec::new())
    }

    fn clear_all(&self) -> Result<(), PortError> {
        Ok(())
    }
}

/// Records the git commit into the shared ordering log.
struct OrderingGit {
    order: Arc<Mutex<Vec<String>>>,
}

impl GitVersioningPort for OrderingGit {
    fn ensure_rig_repo(&self, _rig_dir: &Path) -> Result<(), PortError> {
        Ok(())
    }

    fn commit(
        &self,
        _loom_id: &LoomId,
        _knot_id: &KnotId,
        strand_path: &StrandPath,
        _event_type: &str,
        _tie_off_content: &str,
    ) -> Result<(), PortError> {
        self.order
            .lock()
            .unwrap()
            .push(format!("commit:{}", strand_path.0.display()));
        Ok(())
    }
}

/// Records queue deletions into the shared ordering log.
struct OrderingQueue {
    order: Arc<Mutex<Vec<String>>>,
}

impl StrandQueueAccessor for OrderingQueue {
    fn pending_strand_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn delete(&self, id: &PendingEventId) -> bool {
        self.order
            .lock()
            .unwrap()
            .push(format!("delete:{}", id.0));
        true
    }
}

impl std::fmt::Debug for OrderingQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrderingQueue").finish()
    }
}

/// Ordering invariant: on success the event file is removed as the last
/// step before the git commit — after `KnotProcessing`, `KnotCompleted`,
/// and `StrandProcessed`, and before `commit`.
#[test]
fn success_removes_event_before_git_commit() {
    let dir = tempfile::tempdir().unwrap();
    let strand = create_strand_file(&dir, "feature.md");

    let loom = build_loom("review-loom", vec![build_knot("review")]);
    let store = LoomStore::new();
    store.register(loom.clone());

    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let log_port = OrderingLoomLog {
        order: order.clone(),
    };
    let git = OrderingGit {
        order: order.clone(),
    };
    let queue = OrderingQueue {
        order: order.clone(),
    };

    let profile_repo = Arc::new(MockProfileRepository::new(HashMap::from_iter([
        ("fast".to_string(), default_profile()),
    ])));

    let use_case = ProcessStrand::new(
        store,
        Arc::new(log_port),
        success_runner("review output"),
        Arc::new(MockTieOffSink::new()),
        RigAgentConfig::default_config(),
        PathBuf::from("/rig"),
        profile_repo,
        Arc::new(MockModelRegistry::default()),
        Arc::new(MockRigLogPort::default()),
        Arc::new(git),
        Arc::new(MockStrandFileChecker::new()),
        Arc::new(MockEventDispatcher::default()),
        Some(Arc::new(queue) as Arc<dyn StrandQueueAccessor>),
    );

    let pending = make_pending("review-loom", "review", &strand);
    let result = use_case.execute_with_pending(&pending);
    assert!(result.is_ok(), "success path should return Ok: {:?}", result);

    let recorded = order.lock().unwrap();
    let expected = vec![
        "log:KnotProcessing".to_string(),
        "log:KnotCompleted".to_string(),
        "log:StrandProcessed".to_string(),
        format!("delete:{}", pending.id.0),
        format!("commit:{}", strand.display()),
    ];
    assert_eq!(
        recorded.as_slice(),
        expected.as_slice(),
        "removal must happen after the loom-log writes and before the git commit"
    );
}

// ── Part B: full composition (real runtime + mock pi) ────────────────────

/// Mock `pi` that echoes a fixed response.
const PI_ECHO: &str = "#!/usr/bin/env bash\n\
     cat > /dev/null\n\
     echo \"review complete\"\n\
     exit 0\n";

/// Mock `pi` that reports whether the event queue still holds files
/// *while the agent is running* (the late-removal crash window).
///
/// The strand path arrives as an `@<abs-path>` CLI argument; the events
/// dir is derived from it (`<project>/tie-offs/rig/events`).
const PI_CHECK_EVENTS: &str = "#!/usr/bin/env bash\n\
     cat > /dev/null\n\
     file=\"\"\n\
     for a in \"$@\"; do\n\
       case \"$a\" in @*) file=\"${a#@}\" ;; esac\n\
     done\n\
     if [ -n \"$file\" ]; then\n\
       project_root=$(dirname \"$(dirname \"$file\")\")\n\
       events_dir=\"$project_root/tie-offs/rig/events\"\n\
       if ls \"$events_dir\"/*.json >/dev/null 2>&1; then\n\
         echo \"EVENT_FILE_PRESENT_DURING_RUN\"\n\
       else\n\
         echo \"EVENT_FILE_ABSENT_DURING_RUN\"\n\
       fi\n\
     fi\n\
     echo \"review complete\"\n\
     exit 0\n";

/// Mock `pi` that fails (exit 1) when its session title names the
/// "fail" knot, succeeds otherwise.
const PI_FAIL_KNOT: &str = "#!/usr/bin/env bash\n\
     cat > /dev/null\n\
     name=\"\"\n\
     prev=\"\"\n\
     for a in \"$@\"; do\n\
       if [ \"$prev\" = \"--name\" ]; then name=\"$a\"; fi\n\
       prev=\"$a\"\n\
     done\n\
     case \"$name\" in\n\
       fail*) echo \"this agent always fails\" >&2; exit 1 ;;\n\
       *) echo \"ok output\" ;;\n\
     esac\n";

struct RigFixture {
    _tmp: tempfile::TempDir,
    rig_dir: PathBuf,
    pi_path: PathBuf,
}

/// Create a project with `rig/`, a mock `pi` binary, and the given
/// loom/knot definitions (frontmatter written to `<loom>-loom/`).
fn setup_rig(pi_script: &str, looms: &[(&str, &str, &str)]) -> RigFixture {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path().to_path_buf();
    let rig_dir = project_root.join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    fs::write(&pi_path, pi_script).unwrap();
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

    for (loom_id, knot_id, strand_dir) in looms {
        // Loom IDs carry the `-loom` suffix (e.g. "review-loom") and
        // match the directory name.
        let loom_dir = rig_dir.join(loom_id);
        fs::create_dir_all(&loom_dir).unwrap();
        fs::write(
            loom_dir.join(format!("{knot_id}.md")),
            format!(
                "---\n\
                 name: {knot_id}\n\
                 agent-profile-ref: fast\n\
                 strand-dir: \"{strand_dir}\"\n\
                 git-versioned: false\n\
                 ---\n\
                 \n\
                 Test knot.\n\
                 \n"
            ),
        )
        .unwrap();
    }

    helpers::create_fast_profile(&rig_dir);
    RigFixture {
        _tmp: tmp,
        rig_dir,
        pi_path,
    }
}

/// Poll `condition` every 50 ms until it returns true or the timeout
/// elapses.
fn wait_until<F>(condition: F, timeout_ms: u64, what: &str)
where
    F: Fn() -> bool,
{
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("timeout waiting for {what}");
}

fn events_dir(rig_dir: &Path) -> PathBuf {
    knot::domain::knot_file::derive_runtime_root(rig_dir).join("events")
}

fn count_event_files(rig_dir: &Path) -> usize {
    fs::read_dir(events_dir(rig_dir))
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

fn loom_log_completed(rig_dir: &Path, loom_id: &str) -> usize {
    helpers::read_loom_log(rig_dir, loom_id)
        .iter()
        .filter(|e| helpers::loom_log_event_type(e) == Some("KnotCompleted"))
        .count()
}

fn loom_log_failed(rig_dir: &Path, loom_id: &str) -> usize {
    helpers::read_loom_log(rig_dir, loom_id)
        .iter()
        .filter(|e| helpers::loom_log_event_type(e) == Some("KnotFailed"))
        .count()
}

fn read_rig_log(rig_dir: &Path) -> Vec<serde_json::Value> {
    let log_path =
        knot::domain::knot_file::derive_runtime_root(rig_dir).join(".rig-log");
    let content = match fs::read_to_string(&log_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn rig_log_has_queue_idle(rig_dir: &Path) -> bool {
    read_rig_log(rig_dir)
        .iter()
        .any(|e| e.get("QueueIdle").is_some())
}

/// The front-based service loop drains a 2-event queue (both events
/// processed, both event files removed by late removal) and then writes
/// `QueueIdle` to the rig-log.
#[test]
fn front_loop_drains_queue_then_idles() {
    let f = setup_rig(PI_ECHO, &[("review-loom", "review", "./strands")]);
    let project_root = f.rig_dir.parent().unwrap().to_path_buf();

    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    let handle = helpers::start_knot_with_config(config);
    helpers::wait_for_loom_in_state(&f.rig_dir, "review-loom", 1);

    // Two strand files → two queued events.
    let strands = project_root.join("strands");
    fs::create_dir_all(&strands).unwrap();
    fs::write(strands.join("a.md"), "strand a").unwrap();
    fs::write(strands.join("b.md"), "strand b").unwrap();

    // Both processed (2 × KnotCompleted) and both event files removed.
    wait_until(
        || loom_log_completed(&f.rig_dir, "review-loom") >= 2
            && count_event_files(&f.rig_dir) == 0,
        30_000,
        "front loop to drain 2 events with late removal",
    );

    // Then the loop idles: QueueIdle in the rig-log.
    wait_until(
        || rig_log_has_queue_idle(&f.rig_dir),
        10_000,
        "QueueIdle after drain",
    );

    // Both tie-offs recorded for the same knot (appended entries).
    let tie_off = knot::domain::knot_file::derive_runtime_root(&f.rig_dir)
        .join("review-loom")
        .join("tie-off-review.md");
    let content = fs::read_to_string(&tie_off).unwrap();
    assert!(
        content.contains("review complete"),
        "tie-off should contain agent output. Got:\n{content}"
    );

    handle.abort();
}

/// Crash window: the event file must still exist **while the agent is
/// running** (late removal — a crash mid-processing re-queues instead
/// of losing the event), and be gone after success.
#[test]
fn event_file_present_during_processing_gone_after() {
    let f =
        setup_rig(PI_CHECK_EVENTS, &[("review-loom", "review", "./strands")]);
    let project_root = f.rig_dir.parent().unwrap().to_path_buf();

    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    let handle = helpers::start_knot_with_config(config);
    helpers::wait_for_loom_in_state(&f.rig_dir, "review-loom", 1);

    let strands = project_root.join("strands");
    fs::create_dir_all(&strands).unwrap();
    fs::write(strands.join("feature.md"), "new feature").unwrap();

    // Wait for processing to finish and the queue to be empty.
    wait_until(
        || loom_log_completed(&f.rig_dir, "review-loom") >= 1
            && count_event_files(&f.rig_dir) == 0,
        30_000,
        "processing to complete with late removal",
    );

    // The agent saw the event file while it was running.
    let tie_off = knot::domain::knot_file::derive_runtime_root(&f.rig_dir)
        .join("review-loom")
        .join("tie-off-review.md");
    let content = fs::read_to_string(&tie_off).unwrap();
    assert!(
        content.contains("EVENT_FILE_PRESENT_DURING_RUN"),
        "event file must still exist while the agent runs (late removal). \
         Tie-off:\n{content}"
    );
    assert!(
        !content.contains("EVENT_FILE_ABSENT_DURING_RUN"),
        "event file must NOT already be removed while the agent runs"
    );

    // And it is gone after success.
    assert_eq!(
        count_event_files(&f.rig_dir),
        0,
        "event file must be removed after successful processing"
    );

    handle.abort();
}

/// Poison pill: a knot that always fails has its event consumed exactly
/// once (one `KnotFailed`, no tight re-fail loop), and the loop proceeds
/// to the next event.
#[test]
fn failing_event_consumed_once_then_loop_proceeds() {
    let f = setup_rig(
        PI_FAIL_KNOT,
        &[
            ("fail-loom", "fail", "./strands-fail"),
            ("ok-loom", "ok", "./strands-ok"),
        ],
    );
    let project_root = f.rig_dir.parent().unwrap().to_path_buf();

    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    let handle = helpers::start_knot_with_config(config);
    helpers::wait_for_loom_in_state(&f.rig_dir, "fail-loom", 1);
    helpers::wait_for_loom_in_state(&f.rig_dir, "ok-loom", 1);

    let strands_fail = project_root.join("strands-fail");
    fs::create_dir_all(&strands_fail).unwrap();
    fs::write(strands_fail.join("fail-strand.md"), "will fail").unwrap();
    let strands_ok = project_root.join("strands-ok");
    fs::create_dir_all(&strands_ok).unwrap();
    fs::write(strands_ok.join("ok-strand.md"), "will succeed").unwrap();

    // Both events processed: fail → KnotFailed, ok → KnotCompleted,
    // and the queue is drained (both event files removed).
    wait_until(
        || loom_log_failed(&f.rig_dir, "fail-loom") >= 1
            && loom_log_completed(&f.rig_dir, "ok-loom") >= 1
            && count_event_files(&f.rig_dir) == 0,
        30_000,
        "fail event consumed and ok event processed",
    );

    // The failing event was consumed exactly once — if it had survived
    // at the queue head, the front loop would re-fail it in a tight
    // loop and many more KnotFailed entries would accumulate.
    let failed_count = loom_log_failed(&f.rig_dir, "fail-loom");
    assert_eq!(
        failed_count, 1,
        "failing event must be consumed exactly once (no poison-pill loop), \
         got {failed_count} KnotFailed entries"
    );

    // The ok knot completed and the loop idled afterwards.
    assert!(
        loom_log_completed(&f.rig_dir, "ok-loom") >= 1,
        "loop must proceed to the next event after a failure"
    );
    wait_until(
        || rig_log_has_queue_idle(&f.rig_dir),
        10_000,
        "QueueIdle after poison-pill drain",
    );

    handle.abort();
}

/// A push that lands while the loop is in the **blocking** (non-burst)
/// idle must wake it: the single event is processed without a second
/// push and without a restart.
///
/// The queue starts **empty**, so the loop is in the blocking wait
/// from startup — no event has ever been processed, so no burst window
/// has started and no `QueueIdle` has been written. Waiting past one
/// burst window (500 ms) plus asserting the absence of `QueueIdle`
/// pins the loop in the blocking wait before the single push. The
/// push goes through the real pipeline (watcher → debounce → queue);
/// the armed `notified()` permit (arm-before-check in `next_event`)
/// is what wakes the blocking wait.
#[test]
fn push_while_idle_wakes_loop() {
    let f = setup_rig(PI_ECHO, &[("review-loom", "review", "./strands")]);
    let project_root = f.rig_dir.parent().unwrap().to_path_buf();

    let config =
        knot::AppConfig::with_rig_dir(f.rig_dir.clone()).with_cli_path(f.pi_path.clone());
    let handle = helpers::start_knot_with_config(config);
    helpers::wait_for_loom_in_state(&f.rig_dir, "review-loom", 1);

    // Wait past one burst window: with an empty queue the loop can
    // only be in the blocking wait (a burst — and the QueueIdle that
    // ends it — requires a processed event).
    thread::sleep(Duration::from_millis(1_000));
    assert_eq!(
        count_event_files(&f.rig_dir),
        0,
        "queue must be empty while the loop idles"
    );
    assert_eq!(
        loom_log_completed(&f.rig_dir, "review-loom"),
        0,
        "no event may have been processed before the push"
    );
    assert!(
        !rig_log_has_queue_idle(&f.rig_dir),
        "no QueueIdle before any event: the loop is in the blocking wait"
    );

    // One push while idle: watcher → debounce → queue.
    let strands = project_root.join("strands");
    fs::create_dir_all(&strands).unwrap();
    fs::write(strands.join("idle.md"), "pushed while idle").unwrap();

    // The single push wakes the blocking loop: processed and the event
    // file removed by late removal — no second push, no restart.
    wait_until(
        || loom_log_completed(&f.rig_dir, "review-loom") >= 1
            && count_event_files(&f.rig_dir) == 0,
        30_000,
        "push while idle to wake the blocking loop",
    );
    assert_eq!(
        loom_log_completed(&f.rig_dir, "review-loom"),
        1,
        "the single push must be processed exactly once"
    );

    handle.abort();
}
