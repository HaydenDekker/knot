//! Queue-backed event delivery — plan 099.
//!
//! The delivery mechanism that replaces the dispatched `.md` file. A
//! producer's emission is written **straight into the consumer's queue**
//! as a payload-bearing [`PendingEvent`]: one entry per subscriber, the
//! message in the entry body, removed when the consumer finishes.
//!
//! What this retires:
//!
//! - the per-emission dispatched `event-*.md` file (the payload was
//!   stored twice — in the producer's tie-off block and in every
//!   subscriber's dispatch directory);
//! - the watcher + debounce hop an event had to take to become a queue
//!   entry (delivery is now a direct write, not a filesystem round-trip);
//! - the per-`EventId` dispatch directories under the runtime root.
//!
//! The consumer receives the payload from the queue entry (rendered into
//! its prompt by the processing pipeline) and never reads the producer's
//! tie-off to find out what arrived.

use std::path::PathBuf;

use crate::application::ports::{EventDispatcherPort, PortError, StrandEventQueue};
use crate::domain::entities::{Knot, LoomId, StrandPath};
use crate::domain::events::AgentEvent;
use crate::domain::pending_event::{EventDelivery, PendingEvent};

/// Delivers events by pushing payload-bearing queue entries.
///
/// Holds the queue it writes into. The returned "delivery reference" is
/// the entry's `event:<producer>/<event-id>` descriptor — the same
/// string the entry carries as its `strand_path`, so the service-log
/// `dispatch=` field names the delivery rather than a file that no
/// longer exists.
pub struct QueueEventDispatcher {
    queue: std::sync::Arc<dyn StrandEventQueue>,
}

impl QueueEventDispatcher {
    /// Wrap the queue that deliveries are pushed into.
    pub fn new(queue: std::sync::Arc<dyn StrandEventQueue>) -> Self {
        Self { queue }
    }

    /// Build the delivery payload for one emission.
    ///
    /// `producer_loom` is taken from the event payload's `producer-loom`
    /// field when the emitter supplied one, else the producer knot's own
    /// loom is unknown to this layer and recorded as the producer token
    /// (system-event scopes carry their loom in the token).
    pub fn build_delivery(
        event: &AgentEvent,
        producer_knot: &str,
        producer_loom: &str,
        original_strand: Option<&StrandPath>,
    ) -> EventDelivery {
        EventDelivery {
            event_id: event.event_id.clone(),
            producer_knot: producer_knot.to_string(),
            producer_loom: producer_loom.to_string(),
            original_strand: original_strand
                .map(|s| s.0.to_string_lossy().into_owned()),
            body: event.body.clone(),
            payload: event
                .payload
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}

impl EventDispatcherPort for QueueEventDispatcher {
    fn dispatch(
        &self,
        event: &AgentEvent,
        consumer_knot: &Knot,
        producer_knot: &str,
        consumer_loom_id: &LoomId,
        _rig_dir: &std::path::Path,
        _seq: u32,
        original_strand: Option<&StrandPath>,
    ) -> Result<PathBuf, PortError> {
        // The producer's loom is carried in the payload by callers that
        // know it (`producer-loom`); otherwise the producer token is
        // the best available identity.
        let producer_loom = event
            .payload
            .get("producer-loom")
            .cloned()
            .unwrap_or_else(|| producer_knot.to_string());

        let delivery = Self::build_delivery(
            event,
            producer_knot,
            &producer_loom,
            original_strand,
        );
        let entry = PendingEvent::from_delivery(
            delivery,
            consumer_loom_id,
            &consumer_knot.id,
        );
        let reference = entry.strand_path.clone();
        self.queue.push(entry);
        Ok(PathBuf::from(reference))
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::in_memory_event_queue::InMemoryEventQueue;
    use crate::application::usecases::test_fixtures::KnotBuilder;
    use std::collections::HashMap;

    fn agent_event() -> AgentEvent {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-001".to_string());
        AgentEvent {
            event_id: "PlanCreated".to_string(),
            occurred: true,
            payload,
            body: Some("Three phases.".to_string()),
        }
    }

    fn consumer() -> Knot {
        KnotBuilder::new("reviewer").build()
    }

    #[test]
    fn dispatch_pushes_a_payload_bearing_entry() {
        let queue = std::sync::Arc::new(InMemoryEventQueue::new());
        let dispatcher = QueueEventDispatcher::new(queue.clone());

        let reference = dispatcher
            .dispatch(
                &agent_event(),
                &consumer(),
                "plan-creator",
                &LoomId("review-loom".to_string()),
                std::path::Path::new("/p/rig"),
                0,
                Some(&StrandPath(PathBuf::from("project/plans/099.md"))),
            )
            .unwrap();

        assert_eq!(reference.display().to_string(), "event:plan-creator/PlanCreated");
        assert_eq!(queue.len(), 1);
        let entry = queue.snapshot()[0].clone();
        assert_eq!(entry.kind, "Event");
        assert_eq!(entry.loom_id, "review-loom");
        assert_eq!(entry.knot_id, "reviewer");
        let d = entry.delivery.expect("payload in the entry");
        assert_eq!(d.producer_knot, "plan-creator");
        assert_eq!(
            d.original_strand.as_deref(),
            Some("project/plans/099.md")
        );
        assert_eq!(d.body.as_deref(), Some("Three phases."));
        assert_eq!(d.payload.get("plan").map(String::as_str), Some("PLAN-001"));
    }

    #[test]
    fn dispatch_writes_no_file_anywhere() {
        // The delivery is a queue write only — nothing under the rig dir
        // is created.
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let queue = std::sync::Arc::new(InMemoryEventQueue::new());
        let dispatcher = QueueEventDispatcher::new(queue.clone());
        dispatcher
            .dispatch(
                &agent_event(),
                &consumer(),
                "plan-creator",
                &LoomId("review-loom".to_string()),
                &rig_dir,
                0,
                None,
            )
            .unwrap();

        assert_eq!(std::fs::read_dir(&rig_dir).unwrap().count(), 0);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn fan_out_pushes_one_entry_per_consumer() {
        let queue = std::sync::Arc::new(InMemoryEventQueue::new());
        let dispatcher = QueueEventDispatcher::new(queue.clone());
        let event = agent_event();

        for knot in ["reviewer", "auditor"] {
            dispatcher
                .dispatch(
                    &event,
                    &KnotBuilder::new(knot).build(),
                    "plan-creator",
                    &LoomId("review-loom".to_string()),
                    std::path::Path::new("/p/rig"),
                    0,
                    None,
                )
                .unwrap();
        }

        assert_eq!(queue.len(), 2);
        let knots: Vec<String> = queue
            .snapshot()
            .iter()
            .map(|e| e.knot_id.clone())
            .collect();
        assert_eq!(knots, vec!["reviewer".to_string(), "auditor".to_string()]);
    }

    #[test]
    fn producer_loom_is_taken_from_the_payload_when_supplied() {
        let queue = std::sync::Arc::new(InMemoryEventQueue::new());
        let dispatcher = QueueEventDispatcher::new(queue.clone());
        let mut event = agent_event();
        event
            .payload
            .insert("producer-loom".to_string(), "plan-loom".to_string());

        dispatcher
            .dispatch(
                &event,
                &consumer(),
                "plan-creator",
                &LoomId("review-loom".to_string()),
                std::path::Path::new("/p/rig"),
                0,
                None,
            )
            .unwrap();

        let snapshot = queue.snapshot();
        let d = snapshot[0].delivery.as_ref().unwrap();
        assert_eq!(d.producer_loom, "plan-loom");
    }
}
