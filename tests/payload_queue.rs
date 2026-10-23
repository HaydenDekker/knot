//! Plan 099 Phase 1 — payload-embedded queue entries.
//!
//! The queue entry for a delivered agent-event emission **is** the
//! message: the payload rides in the entry body, one entry per
//! subscriber, removed at completion or failure, and redelivered after a
//! crash by `load_persisted`. No dispatched `.md` file is involved —
//! these tests never create one, and assert the entry alone carries what
//! the consumer needs.
//!
//! The pane for Phase 1: fan-out, removal, redelivery, and the direct
//! queue write of the new `Event` kind.

mod helpers;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use helpers::ProcessStrandBuilder;
use knot::adapters::outbound::DiskBackedEventQueue;
use knot::application::ports::{AgentOutput, PortError, StrandEventQueue};
use knot::application::usecases::test_fixtures::*;
use knot::domain::entities::{KnotId, Loom, LoomId, StrandPath};
use knot::domain::events::{StrandEvent, StrandQueueAccessor};
use knot::domain::pending_event::{
    event_strand_descriptor, is_event_descriptor, EventDelivery, PendingEvent,
    PendingEventId, EVENT_ENTRY_KIND,
};

// ── Fixtures ─────────────────────────────────────────────────────────────

fn delivery() -> EventDelivery {
    let mut payload = BTreeMap::new();
    payload.insert("plan".to_string(), "PLAN-001".to_string());
    payload.insert("phase".to_string(), "3".to_string());
    EventDelivery {
        event_id: "PlanCreated".to_string(),
        producer_knot: "plan-creator".to_string(),
        producer_loom: "plan-loom".to_string(),
        original_strand: Some("project/plans/099.md".to_string()),
        body: Some("The plan covers three phases.".to_string()),
        payload,
    }
}

fn loom_with(id: &str, knot: &str) -> Loom {
    Loom {
        id: LoomId(id.to_string()),
        knots: vec![build_knot_with_profile(knot, "fast")],
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

// ── The entry body carries the payload ───────────────────────────────────

#[test]
fn event_entry_is_typed_and_described() {
    let entry = PendingEvent::from_delivery(
        delivery(),
        &LoomId("review-loom".to_string()),
        &KnotId("plan-reviewer".to_string()),
    );

    assert_eq!(entry.kind, EVENT_ENTRY_KIND);
    assert!(entry.is_event_delivery());
    // The strand is the synthetic descriptor, not a file path.
    assert_eq!(entry.strand_path, "event:plan-creator/PlanCreated");
    assert!(is_event_descriptor(&entry.strand_path));
    // The message is in the entry.
    let d = entry.delivery.expect("delivery present");
    assert_eq!(d.event_id, "PlanCreated");
    assert_eq!(d.producer_knot, "plan-creator");
    assert_eq!(d.producer_loom, "plan-loom");
    assert_eq!(d.original_strand.as_deref(), Some("project/plans/099.md"));
    assert_eq!(d.body.as_deref(), Some("The plan covers three phases."));
    assert_eq!(d.payload.get("plan").map(String::as_str), Some("PLAN-001"));
}

#[test]
fn descriptor_helper_matches_the_entry() {
    assert_eq!(
        event_strand_descriptor("plan-creator", "PlanCreated"),
        "event:plan-creator/PlanCreated"
    );
    assert!(!is_event_descriptor("/project/file.md"));
    assert!(is_event_descriptor("event:knot/Evt"));
}

// ── Fan-out: one entry per subscriber ────────────────────────────────────

#[test]
fn fan_out_gives_each_subscriber_its_own_entry_with_the_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("events");
    let queue = DiskBackedEventQueue::new(dir.clone());

    let subscribers = [
        ("review-loom", "plan-reviewer"),
        ("build-loom", "build-runner"),
        ("audit-loom", "auditor"),
    ];
    let mut ids = Vec::new();
    for (loom, knot) in subscribers {
        let entry = PendingEvent::from_delivery(
            delivery(),
            &LoomId(loom.to_string()),
            &KnotId(knot.to_string()),
        );
        ids.push(queue.push(entry));
    }

    assert_eq!(queue.len(), 3, "one entry per subscriber");

    let snapshot = queue.snapshot();
    assert_eq!(snapshot.len(), 3);
    let mut got: Vec<(String, String)> = snapshot
        .iter()
        .map(|e| (e.loom_id.clone(), e.knot_id.clone()))
        .collect();
    got.sort();
    let mut want: Vec<(String, String)> = subscribers
        .iter()
        .map(|(l, k)| (l.to_string(), k.to_string()))
        .collect();
    want.sort();
    assert_eq!(got, want, "one entry per subscriber, each targeted");
    for entry in &snapshot {
        assert_eq!(entry.kind, EVENT_ENTRY_KIND);
        let d = entry.delivery.as_ref().expect("payload in every entry");
        assert_eq!(d.event_id, "PlanCreated");
        assert_eq!(d.payload.get("phase").map(String::as_str), Some("3"));
    }

    // Distinct ids — removal is per-delivery, never shared.
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 3);

    // The payload is in the persisted JSON, not in a side file.
    let raw = fs::read_to_string(dir.join(format!("{}.json", ids[0].0))).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed["kind"], EVENT_ENTRY_KIND);
    assert_eq!(parsed["delivery"]["event_id"], "PlanCreated");
    assert_eq!(parsed["delivery"]["producer_knot"], "plan-creator");
    assert_eq!(parsed["delivery"]["payload"]["plan"], "PLAN-001");
    assert_eq!(
        parsed["delivery"]["body"],
        "The plan covers three phases."
    );
    assert_eq!(parsed["strand_path"], "event:plan-creator/PlanCreated");
}

#[test]
fn direct_queue_write_creates_no_dispatched_md_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("events");
    let queue = DiskBackedEventQueue::new(dir.clone());

    queue.push(PendingEvent::from_delivery(
        delivery(),
        &LoomId("review-loom".to_string()),
        &KnotId("plan-reviewer".to_string()),
    ));

    // Nothing named `event-*.md` anywhere under the temp root — the
    // delivery never touched the filesystem as a dispatched file.
    let md = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("event-") && n.ends_with(".md")
        })
        .count();
    assert_eq!(md, 0, "no dispatched .md file for a direct entry");
    // …and the entry itself is the only artefact.
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
}

// ── Redelivery after a crash ─────────────────────────────────────────────

#[test]
fn load_persisted_redelivers_the_payload_after_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("events");

    // First process: queue the delivery, then "crash" (drop the queue
    // without processing).
    let first = DiskBackedEventQueue::new(dir.clone());
    let pushed = first.push(PendingEvent::from_delivery(
        delivery(),
        &LoomId("review-loom".to_string()),
        &KnotId("plan-reviewer".to_string()),
    ));
    drop(first);

    // Restart: a fresh queue over the same directory re-seeds from disk.
    let restarted = DiskBackedEventQueue::new(dir.clone());
    assert_eq!(restarted.load_persisted(), 1, "one entry recovered");

    let front = restarted.front().expect("entry redelivered");
    assert_eq!(front.id, pushed);
    assert_eq!(front.kind, EVENT_ENTRY_KIND);
    let d = front.delivery.expect("payload survived the restart");
    assert_eq!(d.producer_knot, "plan-creator");
    assert_eq!(d.body.as_deref(), Some("The plan covers three phases."));
    assert_eq!(d.payload.get("plan").map(String::as_str), Some("PLAN-001"));
}

// ── Removal at completion / failure ──────────────────────────────────────

#[test]
fn completion_removes_the_delivery_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let queue = Arc::new(DiskBackedEventQueue::new(tmp.path().join("events")));
    let entry = PendingEvent::from_delivery(
        delivery(),
        &LoomId("review-loom".to_string()),
        &KnotId("review".to_string()),
    );
    let id = queue.push(entry);
    let stored = queue.pending_event(&id).unwrap();

    let result = ProcessStrandBuilder::new(
        loom_with("review-loom", "review"),
        success_runner("Reviewed."),
    )
    .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
    .build();

    result
        .strand
        .execute_with_pending(&stored)
        .expect("delivery processes");

    assert_eq!(queue.len(), 0, "late removal consumed the delivery");
}

#[test]
fn failure_removes_the_delivery_entry_too() {
    let tmp = tempfile::tempdir().unwrap();
    let queue = Arc::new(DiskBackedEventQueue::new(tmp.path().join("events")));
    let entry = PendingEvent::from_delivery(
        delivery(),
        &LoomId("review-loom".to_string()),
        &KnotId("review".to_string()),
    );
    let id = queue.push(entry);
    let stored = queue.pending_event(&id).unwrap();

    let result = ProcessStrandBuilder::new(
        loom_with("review-loom", "review"),
        failure_runner("agent exploded"),
    )
    .with_strand_queue(Arc::clone(&queue) as Arc<dyn StrandQueueAccessor>)
    .build();

    let _ = result.strand.execute_with_pending(&stored);

    assert_eq!(
        queue.len(),
        0,
        "a failed delivery is consumed, not retried forever"
    );
}

// ── The consumer gets the payload from the entry ─────────────────────────

#[test]
fn consumer_prompt_carries_the_payload_without_a_file() {
    let runner = success_runner("Reviewed.");
    let result = ProcessStrandBuilder::new(
        loom_with("review-loom", "review"),
        runner.clone(),
    )
    .build();

    let event = StrandEvent::EventDelivered {
        loom_id: LoomId("review-loom".to_string()),
        knot_id: KnotId("review".to_string()),
        strand_path: StrandPath(PathBuf::from("event:plan-creator/PlanCreated")),
        delivery: Box::new(delivery()),
    };

    result
        .strand
        .execute(event)
        .expect("event delivery runs through the pipeline");

    let contexts = runner.get_captured_contexts();
    assert_eq!(contexts.len(), 1);
    let ctx = &contexts[0];
    assert!(
        ctx.prompt.contains("PlanCreated"),
        "prompt names the event: {}",
        ctx.prompt
    );
    assert!(
        ctx.prompt.contains("plan-creator"),
        "prompt names the producer: {}",
        ctx.prompt
    );
    assert!(
        ctx.prompt.contains("PLAN-001"),
        "prompt carries the payload: {}",
        ctx.prompt
    );
    assert!(
        ctx.prompt.contains("The plan covers three phases."),
        "prompt carries the body: {}",
        ctx.prompt
    );
    // …and no file was handed to the adapter to read (`@path` arg).
    assert!(
        !ctx.agent_config
            .extra_args
            .iter()
            .any(|a| a.starts_with('@')),
        "no @file reference for a payload-embedded delivery: {:?}",
        ctx.agent_config.extra_args
    );
}

// ── Backward compatibility with pre-099 entries ──────────────────────────

#[test]
fn pre_099_entry_json_loads_without_a_delivery() {
    // Exactly the shape written before plan 099 — no `delivery` key.
    let legacy = r#"{"id":"1700000000000-abcd","kind":"Created",
        "loom_id":"review-loom","knot_id":"review",
        "strand_path":"/project/strand.md",
        "queued_at":"2026-09-11T19:08:00+08:00"}"#;

    let entry: PendingEvent = serde_json::from_str(legacy).unwrap();
    assert_eq!(entry.kind, "Created");
    assert!(entry.delivery.is_none(), "legacy entries have no payload");
    assert!(!entry.is_event_delivery());

    let se: StrandEvent = entry.try_into().unwrap();
    assert!(matches!(se, StrandEvent::Created { .. }));
}

#[test]
fn filesystem_entries_never_serialise_a_delivery_key() {
    let entry = PendingEvent {
        id: PendingEventId("1-a".to_string()),
        kind: "Modified".to_string(),
        loom_id: "l".to_string(),
        knot_id: "k".to_string(),
        strand_path: "/f.md".to_string(),
        queued_at: "2026-01-01T00:00:00Z".to_string(),
        delivery: None,
    };
    let json = serde_json::to_string(&entry).unwrap();
    assert!(
        !json.contains("delivery"),
        "absent payload is never written: {json}"
    );
}

#[test]
fn event_kind_without_a_payload_is_rejected_not_misprocessed() {
    // A malformed `Event` entry (kind set, payload missing) must fail
    // conversion loudly rather than run with an empty message.
    let entry = PendingEvent {
        id: PendingEventId("2-b".to_string()),
        kind: EVENT_ENTRY_KIND.to_string(),
        loom_id: "review-loom".to_string(),
        knot_id: "review".to_string(),
        strand_path: "event:plan-creator/PlanCreated".to_string(),
        queued_at: "2026-01-01T00:00:00Z".to_string(),
        delivery: None,
    };
    let res: Result<StrandEvent, _> = entry.try_into();
    let err = res.unwrap_err().to_string();
    assert!(err.contains("carries no delivery payload"), "{err}");
}

// ── Rendering ────────────────────────────────────────────────────────────

#[test]
fn render_for_consumer_carries_every_field_the_file_used_to() {
    let rendered = delivery().render_for_consumer();
    assert!(rendered.contains("## Event: PlanCreated from plan-creator"));
    assert!(rendered.contains("loom plan-loom"));
    assert!(rendered.contains("original_strand: project/plans/099.md"));
    assert!(rendered.contains("- **plan**: PLAN-001"));
    assert!(rendered.contains("- **phase**: 3"));
    assert!(rendered.contains("The plan covers three phases."));
}

#[test]
fn render_for_consumer_omits_absent_fields() {
    let bare = EventDelivery {
        event_id: "Ping".to_string(),
        producer_knot: "pinger".to_string(),
        producer_loom: "ping-loom".to_string(),
        original_strand: None,
        body: None,
        payload: BTreeMap::new(),
    };
    let rendered = bare.render_for_consumer();
    assert!(!rendered.contains("original_strand:"));
    assert!(!rendered.contains("payload:"));
    assert!(rendered.contains("## Event: Ping from pinger"));
}

// ── Determinism ──────────────────────────────────────────────────────────

#[test]
fn entry_json_is_deterministic_for_the_same_delivery() {
    let mk = || PendingEvent {
        id: PendingEventId("fixed-id".to_string()),
        kind: EVENT_ENTRY_KIND.to_string(),
        loom_id: "review-loom".to_string(),
        knot_id: "review".to_string(),
        strand_path: "event:plan-creator/PlanCreated".to_string(),
        queued_at: "2026-01-01T00:00:00Z".to_string(),
        delivery: Some(delivery()),
    };
    // BTreeMap payload → byte-identical JSON across runs (a HashMap would
    // order keys arbitrarily and make the persisted file unstable).
    assert_eq!(
        serde_json::to_string(&mk()).unwrap(),
        serde_json::to_string(&mk()).unwrap()
    );
}
