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
