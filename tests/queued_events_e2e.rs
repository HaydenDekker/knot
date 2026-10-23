//! Plan 097 phase 5 — end-to-end acceptance for the queue-backed
//! pending-event visibility (the `## Queued Events` prompt section):
//! producer emits → the entry lands in the queue → producer re-enters
//! (same strand, `Modified`) → the **queued** entry (not a directory
//! file) appears in the prompt with the dedup norm, and the acking
//! re-entry dispatches nothing. Plus the cap assertion (a backlog
//! beyond `PENDING_INDEX_CAP` shows count + newest N, not all N) and
//! the omit-when-empty assertion.
//!
//! Per AGENTS.md: mock runner only — no live service, no live rig run.

mod helpers;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use helpers::ProcessStrandBuilder;
use knot::application::in_memory_event_queue::InMemoryEventQueue;
use knot::application::ports::{AgentOutput, StrandEventQueue};
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{
    Knot, KnotId, Loom, LoomId, PromptTemplate, StrandPath,
};
use knot::domain::events::{LoomEvent, StrandEvent, StrandQueueAccessor};
use knot::domain::pending_event::{PendingEvent, PendingEventId};
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
            instructions: "Review the plan.".to_string(),
        },
        git_versioned: true,
        strand_source: StrandSource::EventUri {
            producer_knot: producer.to_string(),
            event_id: event_id.to_string(),
        },
        event_description: Some("When a plan is created.".to_string()),
    }
}

fn build_loom(id: &str, knots: Vec<Knot>) -> Loom {
    Loom {
        id: LoomId(id.to_string()),
        knots,
    }
}

fn output(s: &str) -> AgentOutput {
    AgentOutput {
        stdout: s.to_string(),
        stderr: String::new(),
        exit_code: 0,
        metadata: None,
    }
}

/// The producer loom + a consumer loom listening for the producer's
/// `PlanCreated` (so the producer prompt carries the emission
/// instructions and the subscriber-event requirement).
fn producer_consumer_looms() -> (Loom, Loom) {
    (
        build_loom("plan-loom", vec![build_producer_knot("plan-creator")]),
        build_loom(
            "review-loom",
            vec![build_consumer_knot(
                "plan-reviewer",
                "plan-creator",
                "PlanCreated",
            )],
        ),
    )
}

/// Runtime root for a rig dir under `root` (tie-offs/rig — a sibling of
/// the rig dir's parent, per `derive_runtime_root`).
fn runtime_root(root: &std::path::Path) -> PathBuf {
    root.join("proj").join("tie-offs").join("rig")
}

/// Write a dispatched-shape event file (frontmatter `target-knot` — the
/// producer identity the queued index reads) under the consumer's
/// dispatch directory.
fn write_event_file(
    root: &std::path::Path,
    event_id: &str,
    name: &str,
    target_knot: &str,
    timestamp: &str,
) -> PathBuf {
    let dir = runtime_root(root).join("review-loom").join(event_id);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!(
            "---\nevent-id: {event_id}\ntarget-knot: {target_knot}\ntimestamp: {timestamp}\n---\n## Event: {event_id}\n"
        ),
    )
    .unwrap();
    path
}

/// Push one queued entry (the watcher's job in production).
fn push_entry(
    queue: &InMemoryEventQueue,
    n: u32,
    path: &Path,
    queued_at: &str,
) {
    queue.push(PendingEvent {
        id: PendingEventId(format!("q-{n}")),
        kind: "Created".to_string(),
        loom_id: "review-loom".to_string(),
        knot_id: "plan-reviewer".to_string(),
        strand_path: path.display().to_string(),
        queued_at: queued_at.to_string(),
        delivery: None,
    });
}

// ── Test 1: the queued entry (not a directory file) reaches the ─────
// ── re-entry prompt, with the norm; the ack re-dispatches nothing ──

#[test]
fn queued_entry_appears_in_reentry_prompt_and_ack_dispatches_nothing() {
    let root = tempfile::tempdir().unwrap();
    let rig_dir = root.path().join("proj").join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();

    let (producer_loom, consumer_loom) = producer_consumer_looms();

    // Session 1: the producer emits PlanCreated (occurred: true).
    // Session 2: the re-entry sees the queued event and acks with
    // occurred: false (the dedup norm — the queued event covers the
    // same facts), instead of queueing a second signal.
    let session_1 = concat!(
        "Plan created.\n\n",
        "```markdown\n---\n",
        "event: PlanCreated\n",
        "occurred: true\n",
        "plan: PLAN-001\n",
        "---\n```",
    );
    let session_2 = concat!(
        "Re-check: nothing new.\n\n",
        "```markdown\n---\n",
        "event: PlanCreated\n",
        "occurred: false\n",
        "description: PLAN-001 is already queued — the queued event \
         covers these facts; not queueing a duplicate.\n",
        "---\n```",
    );
    let runner = Arc::new(MockAgentRunner::new_sequence(vec![
        Ok(output(session_1)),
        Ok(output(session_2)),
    ]));

    let result = ProcessStrandBuilder::new(producer_loom.clone(), runner.clone())
        .with_looms(vec![producer_loom, consumer_loom])
        .with_real_event_dispatcher(rig_dir.clone())
        .build();
    let queue = result
        .event_queue
        .as_ref()
        .expect("the builder wired the real queue");

    let strand_path = rig_dir.join("strands").join("plan.md");
    std::fs::create_dir_all(strand_path.parent().unwrap()).unwrap();
    std::fs::write(&strand_path, "write PLAN-001").unwrap();

    // 1. Session 1 — the producer emits; the emission is queued
    //    directly as a payload-bearing delivery (plan 099: no
    //    dispatched file).
    result
        .strand
        .execute(StrandEvent::Created {
            loom_id: LoomId("plan-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        })
        .expect("session 1 should process");

    let queued_entries: Vec<_> = queue
        .snapshot()
        .into_iter()
        .filter(|e| e.kind == knot::domain::pending_event::EVENT_ENTRY_KIND)
        .filter(|e| e.loom_id == "review-loom")
        .collect();
    assert_eq!(
        queued_entries.len(),
        1,
        "exactly one delivery queued: {:?}",
        queue.snapshot().len()
    );
    let delivery = queued_entries[0]
        .delivery
        .as_ref()
        .expect("the entry carries the delivery");
    assert_eq!(delivery.event_id, "PlanCreated");
    assert_eq!(
        delivery.producer_knot, "plan-creator",
        "the delivery carries the producer identity"
    );

    // 2. (No manual push — the producer's emission landed in the
    //    queue directly; the entry is the queued event.)

    // 3. Session 2 — the producer re-enters the same strand (Modified).
    result
        .strand
        .execute(StrandEvent::Modified {
            loom_id: LoomId("plan-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path),
        })
        .expect("session 2 should process");

    // 4. The re-entry prompt carried the **queued** entry — the queue's
    //    path with the queue's own timestamp — plus the dedup norm.
    let ctxs = runner.get_captured_contexts();
    assert_eq!(ctxs.len(), 2, "two sessions were run");
    let prompt = &ctxs[1].prompt;
    assert!(
        prompt.contains("## Queued Events"),
        "the re-entry prompt must carry the queued-events section:\n{prompt}"
    );
    assert!(
        prompt.contains("event:plan-creator/PlanCreated"),
        "the QUEUED entry (descriptor) must appear in the prompt:\n{prompt}"
    );
    assert!(
        prompt.contains("do not queue a duplicate"),
        "the section's dedup pointer must be in the prompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Deduplication"),
        "the dedup norm (content section, same prompt) must be present:\n{prompt}"
    );

    // 5. The acking re-entry dispatched nothing (occurred: false is
    //    never dispatched) — exactly one EventsDispatched in the log.
    let events = result.log_events.lock().unwrap();
    let dispatched_count = events
        .iter()
        .filter(|e| matches!(e, LoomEvent::EventsDispatched { .. }))
        .count();
    assert_eq!(
        dispatched_count, 1,
        "the acking re-entry must not dispatch a second event"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, LoomEvent::KnotEventsMissing { .. })),
        "the occurred: false acknowledgement counts for enforcement"
    );
}

// ── Test 2: the cap end-to-end ──────────────────────────────────────

#[test]
fn backlog_beyond_cap_shows_count_and_newest_cap_entries() {
    let root = tempfile::tempdir().unwrap();
    let rig_dir = root.path().join("proj").join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();

    let (producer_loom, consumer_loom) = producer_consumer_looms();

    // A single re-entry session — it acks (occurred: false).
    let reentry = concat!(
        "Re-check: nothing new.\n\n",
        "```markdown\n---\n",
        "event: PlanCreated\n",
        "occurred: false\n",
        "description: backlog already queued — nothing new.\n",
        "---\n```",
    );
    let runner = Arc::new(MockAgentRunner::new(Ok(output(reentry))));

    // A 15-entry backlog (PENDING_INDEX_CAP is 10): files + queue
    // entries, staggered queued_at so "newest" is unambiguous.
    let queue = Arc::new(InMemoryEventQueue::new());
    let mut paths = Vec::new();
    for i in 1..=15u32 {
        let ts = format!("2026-10-18T09:00:{i:02}");
        let p = write_event_file(
            root.path(),
            "PlanCreated",
            &format!("event-{i:02}.md"),
            "plan-creator",
            &ts,
        );
        push_entry(&queue, i, &p, &ts);
        paths.push(p);
    }
    let queue_accessor: Arc<dyn StrandQueueAccessor> = queue.clone();
    let result = ProcessStrandBuilder::new(producer_loom.clone(), runner.clone())
        .with_looms(vec![producer_loom, consumer_loom])
        .with_real_event_dispatcher(rig_dir.clone())
        .with_strand_queue(queue_accessor)
        .build();

    let strand_path = rig_dir.join("strands").join("plan.md");
    std::fs::create_dir_all(strand_path.parent().unwrap()).unwrap();
    std::fs::write(&strand_path, "write PLAN-001").unwrap();
    result
        .strand
        .execute(StrandEvent::Modified {
            loom_id: LoomId("plan-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path),
        })
        .expect("re-entry should process");

    let prompt = &runner.get_captured_contexts()[0].prompt;
    // Count 15, newest 10 shown — not 15 prompt entries.
    assert!(
        prompt.contains("`PlanCreated` — 15 queued (newest 10 shown):"),
        "the capped header must show the full count + the cap:\n{prompt}"
    );
    // The 10 newest entries (09:00:06 … 09:00:15) are listed…
    for i in 6..=15u32 {
        assert!(
            prompt.contains(&format!("event-{i:02}.md (queued: 2026-10-18T09:00:{i:02})")),
            "the {i:02} (newest ten) entry must be listed:\n{prompt}"
        );
    }
    // …and the 5 oldest (09:00:01 … 09:00:05) are not.
    for i in 1..=5u32 {
        assert!(
            !prompt.contains(&format!("event-{i:02}.md")),
            "the {i:02} (oldest) entry must be capped out:\n{prompt}"
        );
    }
}

// ── Test 3: omit-when-empty end-to-end ──────────────────────────────

#[test]
fn no_queued_entry_of_emittable_types_omits_the_section() {
    let root = tempfile::tempdir().unwrap();
    let rig_dir = root.path().join("proj").join("rig");
    std::fs::create_dir_all(&rig_dir).unwrap();

    let (producer_loom, consumer_loom) = producer_consumer_looms();

    let reentry = concat!(
        "Re-check: nothing new.\n\n",
        "```markdown\n---\n",
        "event: PlanCreated\n",
        "occurred: false\n",
        "description: nothing queued for me — nothing new.\n",
        "---\n```",
    );
    let runner = Arc::new(MockAgentRunner::new(Ok(output(reentry))));

    // Queue holds entries that must NOT surface for this producer:
    // (a) a type this knot cannot emit (SpecDone), (b) a PlanCreated
    //    from another producer (target-knot: other-creator).
    let queue = Arc::new(InMemoryEventQueue::new());
    let other_type = write_event_file(
        root.path(),
        "SpecDone",
        "event-01.md",
        "plan-creator",
        "2026-10-18T09:00:01",
    );
    push_entry(&queue, 1, &other_type, "2026-10-18T09:00:01");
    let other_producer = write_event_file(
        root.path(),
        "PlanCreated",
        "event-02.md",
        "other-creator",
        "2026-10-18T09:00:02",
    );
    push_entry(&queue, 2, &other_producer, "2026-10-18T09:00:02");

    let queue_accessor: Arc<dyn StrandQueueAccessor> = queue.clone();
    let result = ProcessStrandBuilder::new(producer_loom.clone(), runner.clone())
        .with_looms(vec![producer_loom, consumer_loom])
        .with_real_event_dispatcher(rig_dir.clone())
        .with_strand_queue(queue_accessor)
        .build();

    let strand_path = rig_dir.join("strands").join("plan.md");
    std::fs::create_dir_all(strand_path.parent().unwrap()).unwrap();
    std::fs::write(&strand_path, "write PLAN-001").unwrap();
    result
        .strand
        .execute(StrandEvent::Modified {
            loom_id: LoomId("plan-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path),
        })
        .expect("re-entry should process");

    let prompt = &runner.get_captured_contexts()[0].prompt;
    assert!(
        !prompt.contains("## Queued Events"),
        "no queued entry of the knot's emittable types from this \
         producer → the section is omitted:\n{prompt}"
    );
}
