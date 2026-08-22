//! Acceptance test: producer fan-out to consumer processing.
//!
//! Replays the 2026-08-22 incident end-to-end with the **real**
//! `FileSystemEventDispatcher`: a producer knot emits four
//! `ValidationFail` event blocks (distinct CI payloads, one shared
//! event id) in a single tie-off; all four are dispatched to one
//! consumer loom within the same wall-clock second.
//!
//! Before per-batch sequence suffixes + atomic creation, all four
//! writes targeted one path and only the last survived. After the fix:
//! four distinct event files with four distinct payloads, an
//! `EventsDispatched` entry listing all four dispatches, and the
//! consumer processes all four strands.

mod helpers;

use std::path::PathBuf;
use std::sync::Arc;

use helpers::ProcessStrandBuilder;
use knot::application::ports::AgentOutput;
use knot::application::usecases::test_fixtures::*;
use knot::application::usecases::extract_event_metadata;
use knot::domain::entities::{
    Knot, KnotId, Loom, LoomId, PromptTemplate, StrandPath,
};
use knot::domain::events::{LoomEvent, StrandEvent};
use knot::domain::value_objects::StrandSource;

// ── Fixtures ─────────────────────────────────────────────────────────

fn build_producer_knot(id: &str) -> Knot {
    build_knot_with_profile(id, "fast")
}

fn build_consumer_knot(id: &str, producer: &str, event_id: &str) -> Knot {
    Knot {
        id: KnotId(id.to_string()),
        agent_profile_ref: "fast".to_string(),
        prompt_template: PromptTemplate {
            instructions: "Assess the validation gap.".to_string(),
        },
        git_versioned: true,
        strand_source: StrandSource::EventUri {
            producer_knot: producer.to_string(),
            event_id: event_id.to_string(),
        },
        event_description: Some("When validation fails.".to_string()),
    }
}

fn build_loom(id: &str, knots: Vec<Knot>) -> Loom {
    Loom {
        id: LoomId(id.to_string()),
        knots,
    }
}

/// The producer's tie-off: four `ValidationFail` blocks — same event
/// id, distinct CI payloads (the incident: frontend, tauri-commands,
/// tauri-desktop, tauri-android).
fn four_validation_fail_tie_off() -> String {
    let mut blocks = String::new();
    for ci in [
        "frontend",
        "tauri-commands",
        "tauri-desktop",
        "tauri-android",
    ] {
        blocks.push_str(&format!(
            "```markdown\n---\nevent: ValidationFail\nci: {ci}\ndescription: {ci} validation failed\n---\n```\n"
        ));
    }
    format!("Validation complete — four failures.\n{blocks}")
}

// ── Acceptance test ──────────────────────────────────────────────────

#[test]
fn fan_out_four_events_same_second_all_delivered_and_processed() {
    let root = tempfile::tempdir().unwrap();
    let rig_dir = root.path().join("proj").join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();

    let producer_loom = build_loom(
        "retest-loom",
        vec![build_producer_knot("retest-validator")],
    );
    let consumer_loom = build_loom(
        "uat-gap-assessment-loom",
        vec![build_consumer_knot(
            "gap-assessor",
            "retest-validator",
            "ValidationFail",
        )],
    );

    let runner = Arc::new(MockAgentRunner::new(Ok(AgentOutput {
        stdout: four_validation_fail_tie_off(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    })));

    let result = ProcessStrandBuilder::new(producer_loom.clone(), runner)
        .with_looms(vec![producer_loom, consumer_loom])
        .with_real_event_dispatcher(rig_dir.clone())
        .build();

    // 1. Execute the producer strand — the mock agent returns the
    //    four-block tie-off.
    let strand_path = rig_dir.join("strands").join("retest.md");
    std::fs::create_dir_all(strand_path.parent().unwrap()).unwrap();
    std::fs::write(&strand_path, "retest all CIs").unwrap();

    result
        .strand
        .execute(StrandEvent::Created {
            loom_id: LoomId("retest-loom".to_string()),
            knot_id: KnotId("retest-validator".to_string()),
            strand_path: StrandPath(strand_path),
        })
        .expect("producer strand should process");

    // 2. Four event files in the consumer's dispatch directory — all
    //    four survive (the incident regression: before the fix, one).
    let event_dir = root
        .path()
        .join("proj")
        .join("tie-offs")
        .join("rig")
        .join("uat-gap-assessment-loom")
        .join("ValidationFail");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&event_dir)
        .expect("consumer event directory should exist")
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();

    assert_eq!(
        files.len(),
        4,
        "all four event files must exist (none lost to a same-second \
         collision): {files:?}"
    );

    // 4 distinct names, all in the `event-*.md` shape
    let mut names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(
        names.iter().all(|n| n.starts_with("event-") && n.ends_with(".md")),
        "all names must be event-*.md: {names:?}"
    );
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 4, "names must be distinct: {names:?}");

    // 4 distinct payloads — each file contains exactly its own CI
    let contents: Vec<String> =
        files.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
    let mut seen_cis: Vec<&str> = Vec::new();
    for content in &contents {
        let ci = ["frontend", "tauri-commands", "tauri-desktop", "tauri-android"]
            .into_iter()
            .find(|ci| content.contains(&format!("ci: {ci}")))
            .expect("each file must contain one of the four CI payloads");
        assert!(
            !seen_cis.contains(&ci),
            "two files carry the same payload {ci:?}: {names:?}"
        );
        seen_cis.push(ci);
    }
    assert_eq!(seen_cis.len(), 4, "all four payloads delivered");

    // 3. The `EventsDispatched` loom-log entry lists all four dispatches.
    let log_events = result.log_events.lock().unwrap();
    let dispatch_log = log_events
        .iter()
        .find(|e| matches!(e, LoomEvent::EventsDispatched { .. }))
        .expect("producer loom-log must contain EventsDispatched");
    if let LoomEvent::EventsDispatched {
        loom_id,
        knot_id,
        dispatches,
        ..
    } = dispatch_log
    {
        assert_eq!(loom_id.0, "retest-loom");
        assert_eq!(knot_id.0, "retest-validator");
        assert_eq!(
            dispatches.len(),
            4,
            "all four dispatches must be logged"
        );
    }
    drop(log_events);

    // 4. Drive the consumer side: one `StrandEvent::Created` per event
    //    file (the watcher reports per path, so N files → N strands).
    for file in &files {
        result
            .strand
            .execute(StrandEvent::Created {
                loom_id: LoomId("uat-gap-assessment-loom".to_string()),
                knot_id: KnotId("gap-assessor".to_string()),
                strand_path: StrandPath(file.clone()),
            })
            .expect("consumer strand should process");
    }

    // The consumer processed all four strands.
    let log_events = result.log_events.lock().unwrap();
    let completions = log_events
        .iter()
        .filter(|e| {
            matches!(e, LoomEvent::KnotCompleted { knot_id, .. }
                if knot_id.0 == "gap-assessor")
        })
        .count();
    assert_eq!(
        completions, 4,
        "consumer must process all four event strands, got {completions}"
    );

    // Event metadata is traceable on every event file (a2a
    // traceability: event-id + producing knot).
    for file in &files {
        let meta = extract_event_metadata(&StrandPath(file.clone()))
            .expect("event metadata must be extractable");
        assert_eq!(
            meta.event_id.as_deref(),
            Some("ValidationFail"),
            "event-id must round-trip through the frontmatter"
        );
        assert_eq!(
            meta.source_knot.as_deref(),
            Some("retest-validator"),
            "target-knot must round-trip through the frontmatter"
        );
    }
}
