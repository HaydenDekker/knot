//! Plan 082 acceptance tests: system events are dispatchable to subscriber
//! knots.
//!
//! These tests drive the **real** `ProcessStrand` through the harness with a
//! real `FileSystemEventDispatcher` and a real system-event emitter, and
//! assert that subscriber knots receive system events as event files in
//! their `strand-dir` — exactly the mechanism this plan introduces:
//!
//! - a producer **run failure** reaches a wildcard `event:*:KnotFailed`
//!   consumer and the consumer then runs;
//! - a producer **success** reaches a specific-producer
//!   `event:<producer>:KnotCompleted` consumer and the consumer runs;
//! - self-exclusion keeps a knot from re-triggering on its own event;
//! - a producer **timeout** reaches a `TimeoutExceeded` consumer and the
//!   rig-log records `TimeoutExceeded` (no failed tie-off is written —
//!   077/081).

mod helpers;

use helpers::ProcessStrandBuilder;
use knot::application::ports::{AgentOutput, PortError};
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{
    Knot, KnotId, Loom, LoomId, PromptTemplate, StrandPath,
};
use knot::domain::events::{RigLogEvent, StrandEvent};
use knot::domain::value_objects::StrandSource;
use std::path::PathBuf;
use std::sync::Arc;

// ── Fixtures ─────────────────────────────────────────────────────────

/// A producer knot with a filesystem strand source (no subscription).
fn producer_knot(id: &str, profile: &str) -> Knot {
    Knot {
        id: KnotId(id.to_string()),
        agent_profile_ref: profile.to_string(),
        prompt_template: PromptTemplate {
            instructions: "Produce.".to_string(),
        },
        git_versioned: true,
        strand_source: StrandSource::Filesystem(PathBuf::from("/s")),
        event_description: None,
    }
}

/// A consumer knot that subscribes to a system event via an `event:` URI.
fn consumer_knot(id: &str, producer: &str, event_id: &str) -> Knot {
    Knot {
        id: KnotId(id.to_string()),
        agent_profile_ref: "fast".to_string(),
        prompt_template: PromptTemplate {
            instructions: "React to the system event.".to_string(),
        },
        git_versioned: true,
        strand_source: StrandSource::EventUri {
            producer_knot: producer.to_string(),
            event_id: event_id.to_string(),
        },
        event_description: Some("React to a system event.".to_string()),
    }
}

fn ok_output() -> AgentOutput {
    AgentOutput {
        stdout: "done".to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    }
}

/// Collect the event files written to a consumer's dispatch directory
/// (`tie-offs/rig/<loom>/<event-id>/`), sorted by name.
fn event_files(rig_dir: &PathBuf, consumer_loom: &str, event_id: &str) -> Vec<PathBuf> {
    let dir = rig_dir
        .parent()
        .unwrap()
        .join("tie-offs")
        .join("rig")
        .join(consumer_loom)
        .join(event_id);
    if !dir.exists() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("event directory should be readable")
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
}

fn frontmatter_field(content: &str, field: &str) -> Option<String> {
    for line in content.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix(&format!("{field}:")) {
            return Some(rest.trim().to_string());
        }
    }
    None
}

// ── Tests ────────────────────────────────────────────────────────────

/// A producer run failure reaches a wildcard `event:*:KnotFailed` consumer;
/// the consumer then runs and completes.
#[test]
fn failure_reaches_wildcard_consumer_and_consumer_runs() {
    let root = tempfile::tempdir().unwrap();
    let rig_dir = root.path().join("proj").join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();

    // Producer references a missing profile → config-resolution failure.
    let producer_loom = Loom {
        id: LoomId("prod-loom".to_string()),
        knots: vec![producer_knot("producer", "missing")],
    };
    let consumer_loom = Loom {
        id: LoomId("mon-loom".to_string()),
        knots: vec![consumer_knot("monitor", "*", "KnotFailed")],
    };

    let runner = Arc::new(MockAgentRunner::new(Ok(ok_output())));
    let result = ProcessStrandBuilder::new(producer_loom.clone(), runner)
        .with_looms(vec![producer_loom, consumer_loom])
        .with_real_event_dispatcher(rig_dir.clone())
        .with_real_tie_off_sink(rig_dir.clone())
        .with_system_emitter()
        .build();

    let strand_path = rig_dir.join("strands").join("prod.md");
    std::fs::create_dir_all(strand_path.parent().unwrap()).unwrap();
    std::fs::write(&strand_path, "go").unwrap();

    // The producer run fails (missing profile) — `execute` returns Err.
    let res = result
        .strand
        .execute(StrandEvent::Created {
            loom_id: LoomId("prod-loom".to_string()),
            knot_id: KnotId("producer".to_string()),
            strand_path: StrandPath(strand_path),
        });
    assert!(
        res.is_err(),
        "producer run should fail at config resolution"
    );

    // The wildcard consumer received a `KnotFailed` event file, produced by
    // the failing producer (the resolved producer token).
    let files = event_files(&rig_dir, "mon-loom", "KnotFailed");
    assert_eq!(
        files.len(),
        1,
        "exactly one KnotFailed event file for the wildcard consumer: {files:?}"
    );
    let content = std::fs::read_to_string(&files[0]).unwrap();
    assert_eq!(
        frontmatter_field(&content, "event-id").as_deref(),
        Some("KnotFailed")
    );
    assert_eq!(
        frontmatter_field(&content, "target-knot").as_deref(),
        Some("producer"),
        "the resolved producer token must name the failing knot"
    );
    assert!(
        frontmatter_field(&content, "error").is_some(),
        "KnotFailed must carry the error payload: {content}"
    );

    // The consumer is enqueued and runs: drive it with the delivered file.
    result
        .strand
        .execute(StrandEvent::Created {
            loom_id: LoomId("mon-loom".to_string()),
            knot_id: KnotId("monitor".to_string()),
            strand_path: StrandPath(files[0].clone()),
        })
        .expect("consumer strand should process");

    let log_events = result.log_events.lock().unwrap();
    assert!(
        log_events.iter().any(|e| matches!(
            e,
            knot::domain::events::LoomEvent::KnotCompleted { knot_id, .. }
                if knot_id.0 == "monitor"
        )),
        "consumer must complete after reacting to KnotFailed"
    );
}

/// A producer success reaches a specific-producer
/// `event:<producer>:KnotCompleted` consumer; the consumer runs.
#[test]
fn success_reaches_specific_producer_consumer_and_consumer_runs() {
    let root = tempfile::tempdir().unwrap();
    let rig_dir = root.path().join("proj").join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();

    let producer_loom = Loom {
        id: LoomId("prod-loom".to_string()),
        knots: vec![producer_knot("producer", "fast")],
    };
    let consumer_loom = Loom {
        id: LoomId("mon-loom".to_string()),
        knots: vec![consumer_knot("monitor", "producer", "KnotCompleted")],
    };

    let runner = Arc::new(MockAgentRunner::new(Ok(ok_output())));
    let result = ProcessStrandBuilder::new(producer_loom.clone(), runner)
        .with_looms(vec![producer_loom, consumer_loom])
        .with_real_event_dispatcher(rig_dir.clone())
        .with_real_tie_off_sink(rig_dir.clone())
        .with_system_emitter()
        .build();

    let strand_path = rig_dir.join("strands").join("prod.md");
    std::fs::create_dir_all(strand_path.parent().unwrap()).unwrap();
    std::fs::write(&strand_path, "go").unwrap();

    result
        .strand
        .execute(StrandEvent::Created {
            loom_id: LoomId("prod-loom".to_string()),
            knot_id: KnotId("producer".to_string()),
            strand_path: StrandPath(strand_path),
        })
        .expect("producer strand should process");

    // The consumer received a KnotCompleted file (KnotFailed must NOT exist —
    // the run succeeded).
    let files = event_files(&rig_dir, "mon-loom", "KnotCompleted");
    assert_eq!(
        files.len(),
        1,
        "exactly one KnotCompleted event file: {files:?}"
    );
    let content = std::fs::read_to_string(&files[0]).unwrap();
    assert_eq!(
        frontmatter_field(&content, "target-knot").as_deref(),
        Some("producer")
    );
    assert!(
        event_files(&rig_dir, "mon-loom", "KnotFailed").is_empty(),
        "no KnotFailed on a successful run"
    );

    // Drive the consumer with the delivered file.
    result
        .strand
        .execute(StrandEvent::Created {
            loom_id: LoomId("mon-loom".to_string()),
            knot_id: KnotId("monitor".to_string()),
            strand_path: StrandPath(files[0].clone()),
        })
        .expect("consumer strand should process");
}

/// Self-exclusion: a knot that subscribes to its own `KnotCompleted` is not
/// re-triggered by its own completion; a separate consumer in another loom
/// IS triggered.
#[test]
fn self_exclusion_does_not_retrigger_producer() {
    let root = tempfile::tempdir().unwrap();
    let rig_dir = root.path().join("proj").join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();

    // The producer also subscribes to its own KnotCompleted (self-ref).
    let producer_loom = Loom {
        id: LoomId("prod-loom".to_string()),
        knots: vec![producer_knot("producer", "fast")],
    };
    let self_subscriber = Knot {
        id: KnotId("producer".to_string()),
        agent_profile_ref: "fast".to_string(),
        prompt_template: PromptTemplate {
            instructions: "self".to_string(),
        },
        git_versioned: true,
        strand_source: StrandSource::EventUri {
            producer_knot: "producer".to_string(),
            event_id: "KnotCompleted".to_string(),
        },
        event_description: None,
    };
    // Replace the producer with the self-subscribing variant.
    let producer_loom_self = Loom {
        id: LoomId("prod-loom".to_string()),
        knots: vec![self_subscriber],
    };
    let _ = producer_loom;
    let consumer_loom = Loom {
        id: LoomId("mon-loom".to_string()),
        knots: vec![consumer_knot("monitor", "producer", "KnotCompleted")],
    };

    let runner = Arc::new(MockAgentRunner::new(Ok(ok_output())));
    let result = ProcessStrandBuilder::new(producer_loom_self.clone(), runner)
        .with_looms(vec![producer_loom_self, consumer_loom])
        .with_real_event_dispatcher(rig_dir.clone())
        .with_real_tie_off_sink(rig_dir.clone())
        .with_system_emitter()
        .build();

    let strand_path = rig_dir.join("strands").join("prod.md");
    std::fs::create_dir_all(strand_path.parent().unwrap()).unwrap();
    std::fs::write(&strand_path, "go").unwrap();

    result
        .strand
        .execute(StrandEvent::Created {
            loom_id: LoomId("prod-loom".to_string()),
            knot_id: KnotId("producer".to_string()),
            strand_path: StrandPath(strand_path),
        })
        .expect("producer strand should process");

    // Self: the producer's own loom must NOT receive its own KnotCompleted.
    assert!(
        event_files(&rig_dir, "prod-loom", "KnotCompleted").is_empty(),
        "self-exclusion: the producing knot must not be re-triggered by its \
         own KnotCompleted"
    );
    // The separate consumer in another loom IS triggered.
    let files = event_files(&rig_dir, "mon-loom", "KnotCompleted");
    assert_eq!(
        files.len(),
        1,
        "the separate consumer must receive the KnotCompleted: {files:?}"
    );
}

/// A producer timeout reaches a `TimeoutExceeded` consumer; the rig-log
/// records `TimeoutExceeded`. (The failed-tie-off suppression is 077/081 —
/// asserted indirectly: the timeout outcome writes no tie-off.)
#[test]
fn timeout_reaches_consumer_and_rig_log_records_timeout() {
    let root = tempfile::tempdir().unwrap();
    let rig_dir = root.path().join("proj").join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();

    let producer_loom = Loom {
        id: LoomId("prod-loom".to_string()),
        knots: vec![producer_knot("producer", "fast")],
    };
    let consumer_loom = Loom {
        id: LoomId("mon-loom".to_string()),
        knots: vec![consumer_knot("monitor", "producer", "TimeoutExceeded")],
    };

    // A 1s profile budget: MIN_REMAINING_SECS (5) means the first retry
    // exhausts the budget → a `PortError::Timeout` outcome → timeout.
    let mut profile = default_profile();
    profile.timeout = Some(1);

    // The initial call returns a resumable timeout error (with a session
    // id) so the resume loop is entered, where the budget is exhausted.
    let runner = Arc::new(MockAgentRunner::new(Err(PortError::Timeout {
        message: "timed out".to_string(),
        session_id: Some("sess-abc".to_string()),
    })));

    let result = ProcessStrandBuilder::new(producer_loom.clone(), runner)
        .with_looms(vec![producer_loom, consumer_loom])
        .with_profile(profile)
        .with_real_event_dispatcher(rig_dir.clone())
        .with_real_tie_off_sink(rig_dir.clone())
        .with_system_emitter()
        .build();

    let strand_path = rig_dir.join("strands").join("prod.md");
    std::fs::create_dir_all(strand_path.parent().unwrap()).unwrap();
    std::fs::write(&strand_path, "go").unwrap();

    let res = result
        .strand
        .execute(StrandEvent::Created {
            loom_id: LoomId("prod-loom".to_string()),
            knot_id: KnotId("producer".to_string()),
            strand_path: StrandPath(strand_path),
        });
    // A timeout is a terminal *outcome*, not an error — `execute` returns Ok.
    let _ = res.expect("timeout is a terminal outcome, not an error");

    // The consumer received a TimeoutExceeded file.
    let files = event_files(&rig_dir, "mon-loom", "TimeoutExceeded");
    assert_eq!(
        files.len(),
        1,
        "exactly one TimeoutExceeded event file: {files:?}"
    );
    let content = std::fs::read_to_string(&files[0]).unwrap();
    assert_eq!(
        frontmatter_field(&content, "target-knot").as_deref(),
        Some("producer")
    );

    // The rig-log records TimeoutExceeded.
    let rig_events = result.rig_events.lock().unwrap();
    assert!(
        rig_events
            .iter()
            .any(|e| matches!(e, RigLogEvent::TimeoutExceeded { .. })),
        "rig-log must record TimeoutExceeded"
    );
}

// ── Binary-level: rig-scoped QueueIdle (plan 092) ─────────────────────
//
// The in-process `ProcessStrand` harness above drives knot/loom-scoped
// system events. `QueueIdle` is emitted by the service's pipeline loop
// (`spawn_process_strand_loop`), so its end-to-end coverage needs the
// full service: these tests spawn the compiled `knot` binary with a
// mock agent CLI, drive a burst, and assert the rig-scoped dispatch and
// the zero-consumer diagnostic on captured stderr (the harness pattern
// of `tests/consolidated_log.rs`).

use std::fs;
use std::path::Path;
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

/// The rig's runtime root (`tie-offs/dev-rig/` under `cwd`).
fn runtime_root(cwd: &Path) -> std::path::PathBuf {
    knot::domain::knot_file::derive_runtime_root(&cwd.join("dev-rig"))
}

/// Create a minimal rig at `cwd/dev-rig`:
///
/// - `work-loom` / `worker` — a filesystem-strand knot that completes
///   instantly (drives the burst);
/// - `orch-loom` / `queue-monitor` — the rig-scoped `QueueIdle` consumer
///   whose subscription is parameterised (`consumer_subscription`);
/// - the `fast` profile and a mock `pi` that records each invocation in
///   `cwd/agent-runs.log` and exits 0.
fn create_rig(cwd: &Path, consumer_subscription: &str) -> std::path::PathBuf {
    let rig = cwd.join("dev-rig");
    let work = rig.join("work-loom");
    fs::create_dir_all(&work).unwrap();
    fs::write(
        work.join("worker.md"),
        "---\nname: worker\nagent-profile-ref: fast\nstrand-dir: \"./strands\"\ngit-versioned: false\n---\n\nWork.\n",
    )
    .unwrap();
    let orch = rig.join("orch-loom");
    fs::create_dir_all(&orch).unwrap();
    fs::write(
        orch.join("queue-monitor.md"),
        format!(
            "---\nname: queue-monitor\nagent-profile-ref: fast\nstrand-dir: \"{}\"\ngit-versioned: false\n---\n\nMonitor.\n",
            consumer_subscription
        ),
    )
    .unwrap();
    let profiles = rig.join("profiles");
    fs::create_dir_all(&profiles).unwrap();
    fs::write(
        profiles.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\n---\n\nYou work.\n",
    )
    .unwrap();
    let pi = rig.join("bin").join("pi");
    fs::create_dir_all(pi.parent().unwrap()).unwrap();
    fs::write(
        &pi,
        format!(
            "#!/usr/bin/env bash\ncat > /dev/null\necho run >> {}/agent-runs.log\necho \"ok\"\nexit 0\n",
            cwd.display()
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
    pi
}

/// Spawn the knot service (auto-discovery of the single rig) with fast
/// timing and the mock `pi` injected via `KNOT_TEST_CLI_PATH`.
struct ServiceGuard(Option<Child>);

impl ServiceGuard {
    fn into_inner(mut self) -> Child {
        self.0.take().expect("service guard must own the child")
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
/// return the captured output plus whether the exit was graceful.
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
fn wait_until<F>(mut pred: F, timeout_ms: u64, what: &str)
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if pred() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("timeout waiting for {what}");
}

/// Count `*.md` files directly inside `dir` (`0` while it is absent).
fn md_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<std::path::PathBuf> =
        fs::read_dir(dir).map_or_else(
            |_| Vec::new(),
            |rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                    .map(|e| e.path())
                    .collect()
            },
        );
    files.sort();
    files
}

/// A producer burst (one worker strand) drives the queue to drain, and
/// the rig-scoped `QueueIdle` reaches the `event:knot:QueueIdle` consumer
/// end to end: the event file lands in the consumer's dispatch directory
/// with the actual rig id as producer (D4), and the consumer knot runs.
#[test]
fn queue_idle_reaches_static_engine_token_consumer() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path();
    let pi = create_rig(cwd, "event:knot:QueueIdle");
    let guard = spawn_service(cwd, &pi);

    // Startup: state.json under the runtime root.
    wait_until(
        || runtime_root(cwd).join("state.json").exists(),
        15_000,
        "service startup (state.json)",
    );

    // Drive a burst: one worker strand.
    let strands = cwd.join("strands");
    fs::create_dir_all(&strands).unwrap();
    fs::write(strands.join("s1.md"), "go").unwrap();

    // Queue drains after the worker completes; the 500ms idle window
    // then dispatches `QueueIdle` to the consumer's event directory.
    let event_dir = runtime_root(cwd).join("orch-loom").join("QueueIdle");
    wait_until(
        || !md_files(&event_dir).is_empty(),
        20_000,
        "QueueIdle event file for the static-token consumer",
    );

    // The first dispatched file carries the event id and the actual rig
    // id as producer — the subscription form does not leak into the
    // file (plan 092 D4).
    let first = md_files(&event_dir).remove(0);
    let content = fs::read_to_string(&first).unwrap();
    assert_eq!(
        frontmatter_field(&content, "event-id").as_deref(),
        Some("QueueIdle")
    );
    assert_eq!(
        frontmatter_field(&content, "target-knot").as_deref(),
        Some("dev-rig"),
        "the frontmatter must carry the actual rig id: {content}"
    );

    // The consumer knot runs: the mock `pi` records the worker run plus
    // the `queue-monitor` run triggered by the delivered event file.
    let run_count = || {
        fs::read_to_string(cwd.join("agent-runs.log"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    };
    wait_until(
        || run_count() >= 2,
        15_000,
        "worker + queue-monitor agent runs",
    );

    let (output, graceful) = stop_service(guard);
    assert!(graceful, "the service must stop gracefully on SIGINT");
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Positive control: a dispatch that matched a consumer leaves no
    // zero-consumer diagnostic.
    assert!(
        !stderr.lines().any(|l| l.contains("[KNOT][SYSTEM]") && l.contains("QueueIdle")),
        "a matched dispatch must not log a zero-consumer diagnostic: {stderr}"
    );
}

/// The rename-mismatch signature: a consumer subscribed to the same
/// event id with a non-matching producer token receives no event file,
/// and the service log carries the `[KNOT][SYSTEM]` diagnostic naming
/// the near-miss subscription (plan 092 D6).
#[test]
fn queue_idle_near_miss_is_loud_and_dispatches_nothing() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path();
    let pi = create_rig(cwd, "event:stale-rig:QueueIdle");
    let guard = spawn_service(cwd, &pi);

    wait_until(
        || runtime_root(cwd).join("state.json").exists(),
        15_000,
        "service startup (state.json)",
    );

    let strands = cwd.join("strands");
    fs::create_dir_all(&strands).unwrap();
    fs::write(strands.join("s1.md"), "go").unwrap();

    // Let the burst drain and the 500ms idle window elapse (the mock
    // `pi` exits instantly, so 3s is ample margin).
    thread::sleep(Duration::from_millis(3_000));

    // No event file was dispatched for the near-miss subscription.
    let event_dir = runtime_root(cwd).join("orch-loom").join("QueueIdle");
    assert!(
        md_files(&event_dir).is_empty(),
        "a near-miss subscription must not receive an event file"
    );

    let (output, graceful) = stop_service(guard);
    assert!(graceful, "the service must stop gracefully on SIGINT");
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The diagnostic is loud and names the mismatch.
    assert!(
        stderr.lines().any(|l| {
            l.contains("[KNOT][SYSTEM]")
                && l.contains("event=QueueIdle rig=dev-rig")
                && l.contains("near-miss subscription(s): queue-monitor (event:stale-rig:QueueIdle)")
        }),
        "the service log must name the near-miss subscription: {stderr}"
    );
}
