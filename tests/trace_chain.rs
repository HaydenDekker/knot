//! Plan 099 (Phase 5) acceptance: the **trace chain**.
//!
//! One run's identity threads through the three durable surfaces:
//!
//! ```text
//! service log  →  git commit trailers  →  tie-offs.log section
//! ```
//!
//! and the `session:` field links the commit back to the `pi` session
//! (`pi --session-id`). The test drives a producer run that emits a
//! `PlanCreated` delivery, then the consumer run that receives it, and
//! asserts:
//!
//! - the consumer's `tie-offs.log` section header carries the full
//!   trace (`event-id=`, `source=`, `session=`, `original-strand=`);
//! - the consumer's git commit carries the matching trailer block
//!   (`event:`, `source:`, `session:`, `original-strand:`);
//! - the session id is **the same** on both surfaces (the
//!   `pi --session-id` link);
//! - the event payload is **not duplicated** into the log — the
//!   `tie-offs.log` section carries the slim audit line, not the full
//!   event block (the full content lives in the commit body).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary_path() -> String {
    std::env::var("CARGO_BIN_EXE_knot").unwrap_or_else(|_| {
        format!(
            "{}/target/debug/knot",
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string())
        )
    })
}

/// A mock `pi` (JSON-L): reports a session id, then an `agent_end`
/// carrying a tie-off with one `PlanCreated` event block.
const MOCK_PI: &str = r#"#!/usr/bin/env bash
cat > /dev/null
echo '{"type":"session","id":"sess-trace-001"}'
printf '{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"Work complete.\\n\\n```markdown\\n---\\nevent: PlanCreated\\noccurred: true\\ndescription: PLAN-001 created\\n---\\n```\\n"}]}]}'
exit 0
"#;

/// A git-initialized project with a two-knot rig (producer + consumer),
/// both git-versioned.
struct TraceFixture {
    _tmp: tempfile::TempDir,
    project: PathBuf,
    rig_dir: PathBuf,
    pi_path: PathBuf,
}

fn setup() -> TraceFixture {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    fs::create_dir_all(&project).unwrap();

    // Git repo with an initial commit and a configured identity.
    run_git(&project, &["init", "-q"]);
    run_git(&project, &["config", "user.email", "trace@test.local"]);
    run_git(&project, &["config", "user.name", "Trace Test"]);
    fs::write(project.join("README.md"), "# trace fixture\n").unwrap();
    run_git(&project, &["add", "-A"]);
    run_git(&project, &["commit", "-q", "-m", "initial"]);

    let rig_dir = project.join("rig");
    fs::create_dir_all(&rig_dir).unwrap();
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-json\n",
    )
    .unwrap();
    fs::write(rig_dir.join("models.yml"), "# empty registry\n").unwrap();

    // Profiles.
    let profiles = rig_dir.join("profiles");
    fs::create_dir_all(&profiles).unwrap();
    fs::write(
        profiles.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\n---\n\nYou are a worker.\n",
    )
    .unwrap();

    // Producer loom + knot (git-versioned).
    let producer_loom = rig_dir.join("plan-loom");
    fs::create_dir_all(&producer_loom).unwrap();
    fs::write(
        producer_loom.join("plan-creator.md"),
        "---\nname: plan-creator\nagent-profile-ref: fast\nstrand-dir: \"./strands\"\ngit-versioned: true\n---\n\nCreate the plan.\n",
    )
    .unwrap();

    // Consumer loom + knot (git-versioned, subscribes to the delivery).
    let consumer_loom = rig_dir.join("watch-loom");
    fs::create_dir_all(&consumer_loom).unwrap();
    fs::write(
        consumer_loom.join("plan-watcher.md"),
        "---\nname: plan-watcher\nagent-profile-ref: fast\nstrand-dir: \"event:plan-creator:PlanCreated\"\ngit-versioned: true\n---\n\nWatch for created plans.\n",
    )
    .unwrap();

    // Mock pi.
    let bin = rig_dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let pi_path = bin.join("pi");
    fs::write(&pi_path, MOCK_PI).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    TraceFixture {
        _tmp: tmp,
        project,
        rig_dir,
        pi_path,
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} failed: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn runtime_root(f: &TraceFixture) -> PathBuf {
    knot::domain::knot_file::derive_runtime_root(&f.rig_dir)
}

/// Pre-queue a Created event for the producer strand.
fn queue_producer_created(f: &TraceFixture, strand: &Path) {
    let events = runtime_root(f).join("events");
    fs::create_dir_all(&events).unwrap();
    let json = serde_json::json!({
        "id": "1000000000001-prod",
        "kind": "Created",
        "loom_id": "plan-loom",
        "knot_id": "plan-creator",
        "strand_path": strand.to_string_lossy(),
        "queued_at": "2026-01-01T00:00:00",
    });
    fs::write(events.join("1000000000001-prod.json"), json.to_string())
        .unwrap();
}

/// Run `knot step` (FIFO head) with fast timing.
fn step(f: &TraceFixture) {
    unsafe {
        std::env::set_var("KNOT_TEST_DEBOUNCE_MS", "20");
        std::env::set_var("KNOT_TEST_CHECK_MS", "2");
    }
    let out = Command::new(binary_path())
        .current_dir(&f.project)
        .args(["step", "--rig", "rig"])
        .env("KNOT_TEST_CLI_PATH", &f.pi_path)
        .env("KNOT_TEST_DEBOUNCE_MS", "20")
        .env("KNOT_TEST_CHECK_MS", "2")
        .output()
        .expect("run knot step");
    assert!(
        out.status.success(),
        "step failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The latest commit message in the project repo.
fn latest_commit_message(f: &TraceFixture) -> String {
    let out = Command::new("git")
        .args(["log", "-1", "--pretty=%B"])
        .current_dir(&f.project)
        .output()
        .expect("git log");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The tie-offs.log content.
fn tie_offs_log(f: &TraceFixture) -> String {
    let log = runtime_root(f).join("tie-offs.log");
    fs::read_to_string(&log).unwrap_or_default()
}

#[test]
fn trace_chain_links_service_log_commit_and_tie_off_log() {
    let f = setup();

    // The producer strand file.
    let strands = f.project.join("strands");
    fs::create_dir_all(&strands).unwrap();
    let plan = strands.join("plan.md");
    fs::write(&plan, "write PLAN-001").unwrap();

    // Step 1 — the producer runs, emits PlanCreated, commits, and the
    // delivery is queued to the consumer.
    queue_producer_created(&f, &plan);
    step(&f);

    // Step 2 — the consumer runs the queued delivery and commits with
    // the full trace.
    step(&f);

    let log = tie_offs_log(&f);
    let commit = latest_commit_message(&f);

    // ── tie-offs.log: the consumer section header carries the trace ──
    assert!(
        log.contains("## knot=plan-watcher"),
        "the consumer section must be in the unified log:\n{log}"
    );
    assert!(
        log.contains("event-id=PlanCreated"),
        "the header must carry the triggering event id:\n{log}"
    );
    assert!(
        log.contains("source=plan-creator"),
        "the header must name the producing knot:\n{log}"
    );
    assert!(
        log.contains("session=sess-trace-001"),
        "the header must carry the session id:\n{log}"
    );
    assert!(
        log.contains("original-strand=") && log.contains("plan.md"),
        "the header must carry the producer's original strand:\n{log}"
    );

    // ── git commit: the trailer block carries the matching trace ──
    assert!(
        commit.contains("event: PlanCreated"),
        "the commit trailer must carry the event id:\n{commit}"
    );
    assert!(
        commit.contains("source: plan-creator"),
        "the commit trailer must name the producing knot:\n{commit}"
    );
    assert!(
        commit.contains("session: sess-trace-001"),
        "the commit trailer must carry the session id:\n{commit}"
    );
    assert!(
        commit.contains("original-strand:") && commit.contains("plan.md"),
        "the commit trailer must carry the original strand:\n{commit}"
    );

    // ── the session: link — the same id on both surfaces ──
    // (the commit's `session:` == the log's `session=` == the pi
    // session, resumable with `pi --session-id sess-trace-001`)
    let log_session = extract_field(&log, "session=");
    let commit_session = extract_field(&commit, "session: ");
    assert_eq!(
        log_session.as_deref(),
        Some("sess-trace-001"),
        "log session must be the pi session id"
    );
    assert_eq!(
        commit_session.as_deref(),
        Some("sess-trace-001"),
        "commit session must be the same pi session id (the \
         `pi --session-id` link)"
    );

    // ── no persistent payload duplication ──
    // The tie-offs.log section carries the SLIM audit line, not the
    // full event block. The full event content lives in the commit
    // body; the log keeps only the audit.
    assert!(
        log.contains("- event-audit id=PlanCreated occurred=true reason=PLAN-001 created"),
        "the log must carry the slim event-audit line:\n{log}"
    );
    assert!(
        !log.contains("```markdown"),
        "the log must NOT carry the full event block (slimmed):\n{log}"
    );
    // The commit body keeps the producer's full tie-off content.
    assert!(
        commit.contains("PLAN-001 created"),
        "the commit body must keep the full event content:\n{commit}"
    );
}

/// Extract the value of `key<value>` up to the next whitespace.
fn extract_field(haystack: &str, key: &str) -> Option<String> {
    let pos = haystack.find(key)?;
    let rest = &haystack[pos + key.len()..];
    let val = rest.split_whitespace().next()?;
    Some(val.to_string())
}
