use serde::{Deserialize, Serialize};

use crate::domain::entities::{
    Knot, KnotId, LoomId, StrandPath, TieOffPath,
};

// ── Agent Events (Intent-Based Routing) ────────────────────────────────────

use std::collections::HashMap;



/// A structured agent-to-agent event emitted in a tie-off.
///
/// When a target knot is instructed to emit an event (via intent-based routing
/// context injection), it writes a structured block in its tie-off body. The
/// `event:` key signals that the block contains event data. All other keys
/// (except `target-knot`, which is derived from context) become the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvent {
    /// Unique event identifier (e.g. `PlanCreated`).
    pub event_id: String,
    /// Arbitrary key-value pairs carrying event data.
    /// Includes fields like `plan`, `description`, `source`, etc.
    #[serde(default)]
    pub payload: HashMap<String, String>,
    /// Freeform narrative context attached to the event.
    ///
    /// When agents emit events inside ```markdown code blocks with
    /// YAML-style frontmatter, the text after the closing `---` is
    /// captured here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}



// ── Context Injection ──────────────────────────────────────────────────────

/// Build the listener context block to inject at the start of a target
/// knot's prompt.
///
/// Scans all knots' `strand_source` entries for `EventUri` subscriptions
/// where the current knot is the producer. Groups by `event-id` — if
/// multiple consumers listen for the same event from the same knot,
/// they are merged into one event block (not duplicated).
///
/// Uses the `event_description` from the first consumer knot declaring
/// each event. When `event_description` is absent, a generic message
/// is injected.
///
/// Returns an empty string when no consumers are listening (no injection
/// needed).
///
/// The returned markdown is designed to be prepended to the knot's
/// instructions before execution.
///
/// ## Multi-event format
///
/// A producer may emit **multiple events** in a single tie-off (one
/// indented block per event type). Each event block is independently
/// parsed and dispatched. If no events occurred, emit `event: None`.
///
pub fn build_listener_context(knot: &Knot, all_knots: &[Knot]) -> String {
    use crate::domain::value_objects::StrandSource;

    // Collect all event subscriptions where this knot is the producer.
    let mut matching_knots: Vec<&Knot> = Vec::new();
    for other in all_knots {
        if let StrandSource::EventUri {
            producer_knot,
            ..
        } = &other.strand_source
        {
            if producer_knot == &knot.id.0 {
                matching_knots.push(other);
            }
        }
    }

    // No listeners — no injection needed.
    if matching_knots.is_empty() {
        return String::new();
    }

    // Group by event-id, preserving insertion order (first seen wins for
    // the description). Only keep the first consumer knot for each event.
    let mut seen_ids: std::collections::HashMap<String, &Knot> =
        std::collections::HashMap::new();
    for consumer in &matching_knots {
        if let StrandSource::EventUri {
            producer_knot: _,
            event_id,
        } = &consumer.strand_source
        {
            if !seen_ids.contains_key(event_id) {
                seen_ids.insert(event_id.clone(), consumer);
            }
        }
    }

    let mut output = String::from(
        "## Agent Events\n\n\
         Other processors are listening for events you may emit. If an event occurs\n\
         during your work, include an explicit event block in your final response using\n\
         the format shown.\n\n\
         You may emit **multiple events** in one final response — one indented block\n\
         per event type.\n\n\
         Events you may emit:\n",
    );

    for (event_id, consumer) in &seen_ids {
        let description = if let Some(desc) = &consumer.event_description {
            desc.as_str()
        } else {
            "If this event occurs, emit a structured event block in your final response."
        };

        output.push_str(&format!(
            "- `{}` — {}\n",
            event_id, description
        ));
    }

    output.push_str("\nIf events occurred, emit one indented block per event in your final response:\n");
    output.push_str("```\n");
    output.push_str("event: <EventId>\n");
    output.push_str("description: <short summary of what happened>\n");
    output.push_str("<additional fields as relevant>\n");
    output.push_str("```\n");

    output.push_str("\nIf no events occurred, emit:\n");
    output.push_str("```\n");
    output.push_str("event: None\n");
    output.push_str("```\n");

    output
}

// ── Domain Events ──────────────────────────────────────────────────────────

/// An event that describes the lifecycle of a Strand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrandEvent {
    /// A new strand (input file) was detected.
    Created {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
    },
    /// An existing strand was modified.
    Modified {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
    },
    /// A strand was removed from the source.
    Deleted {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
    },
}

/// A TieOff (output file) was successfully produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TieOffProduced {
    pub knot_id: KnotId,
    pub strand_path: StrandPath,
    pub tie_off_path: TieOffPath,
}

/// Processing of a strand failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessingFailed {
    pub knot_id: KnotId,
    pub strand_path: StrandPath,
    pub error_message: String,
}

/// An event that describes the lifecycle of a Loom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoomEvent {
    /// A new Knot was registered with the Loom.
    KnotRegistered {
        loom_id: LoomId,
        knot_id: KnotId,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// The Loom began processing its strands.
    LoomStarted {
        loom_id: LoomId,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// The Loom stopped processing.
    LoomStopped {
        loom_id: LoomId,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A strand was processed (either produced output or failed).
    StrandProcessed {
        loom_id: LoomId,
        strand_path: StrandPath,
        /// Error message if processing failed. `None` on success.
        error: Option<String>,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot started processing a strand.
    KnotProcessing {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot completed processing a strand successfully.
    KnotCompleted {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        tie_off_path: TieOffPath,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot failed while processing a strand.
    KnotFailed {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        error: String,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot was deregistered from the loom.
    KnotDeregistered {
        loom_id: LoomId,
        knot_id: KnotId,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A knot file contained unknown YAML properties (accepted but not used).
    KnotParseWarning {
        loom_id: LoomId,
        knot_file_name: String,
        message: String,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// The strand directory for a knot was auto-created.
    DirectoryCreated {
        loom_id: LoomId,
        knot_id: KnotId,
        /// Absolute path of the directory that was created.
        directory: String,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A strand file was ignored (not a text file).
    ///
    /// Binary or non-text files in a strand directory are silently
    /// skipped. A warning is written to the loom-log and stderr.
    StrandIgnored {
        loom_id: LoomId,
        knot_id: KnotId,
        /// Path to the file that was ignored.
        strand_path: StrandPath,
        /// Reason the file was ignored (e.g. "binary file").
        reason: String,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A strand file was skipped because it could not be found on disk.
    ///
    /// Unlike [`StrandIgnored`] (binary files), this records a file that
    /// existed at event time but disappeared before processing — typically a
    /// short-lived temp file from editors like `sed -i`. Known temp-file
    /// patterns are silently dropped elsewhere; this variant logs the
    /// remaining unknown-missing-file cases so the user can investigate.
    StrandSkipped {
        loom_id: LoomId,
        knot_id: KnotId,
        /// Path to the file that was skipped.
        strand_path: StrandPath,
        /// Reason the file was skipped (e.g. "missing file (unknown pattern)").
        reason: String,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// A failed agent invocation was resumed using the same Pi session.
    ///
    /// Recorded when a resumable error (timeout, mid-stream failure) is
    /// detected and Knot retries the invocation with `--session-id <id>`
    /// to continue the Pi session from where it left off.
    SessionResumed {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        session_id: String,
        attempt: u32,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// The agent completed within its timeout but produced no response.
    ///
    /// Recorded each time an invocation returns exit-code 0 with empty
    /// stdout — the agent session ended early (e.g. provider returned
    /// immediately) without generating any output. Logged per-attempt
    /// so the user can see repeated empty responses during retries.
    KnotEmptyResponse {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// Number of the attempt that produced the empty response
        /// (1 = first attempt, 2 = first retry, etc.).
        attempt: u32,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// One or more agent events were dispatched to consumer knots.
    ///
    /// Recorded after a knot completes successfully and structured agent
    /// events are extracted from its tie-off. Lists which event-ids were
    /// dispatched and to which consumer looms.
    EventsDispatched {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        /// List of (event-id, consumer-loom-id) pairs dispatched.
        dispatches: Vec<(String, String)>,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
}

/// A Knot was registered with a Loom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnotRegistered {
    pub loom_id: LoomId,
    pub knot_id: KnotId,
}

// ── Rig-Log Events ─────────────────────────────────────────────────────────

/// An operational event written to the rig-log (`rig/.rig-log`).
///
/// The rig-log is an append-only JSONL file that records serious operational
/// events so the user or an external watcher can monitor and react.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RigLogEvent {
    /// An agent session exceeded its timeout deadline.
    TimeoutExceeded {
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: StrandPath,
        error: String,
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
    /// All pending events have been processed and the queue is idle.
    QueueIdle {
        /// ISO 8601 UTC timestamp.
        timestamp: String,
    },
}

// ── Configuration Events ───────────────────────────────────────────────────

/// An event that describes configuration changes to looms and knots.
///
/// Unlike [`StrandEvent`] which tracks input file lifecycle, config events
/// track changes to the loom/knot definition files themselves (the `.md` knot
/// files and `*-loom` directories).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigEvent {
    /// A new loom directory was detected (ends in `-loom`).
    LoomAdded {
        loom_id: LoomId,
        /// Absolute path to the loom directory (e.g. `/project/rig/new-loom`).
        /// Used by `ConfigEventHandler` to scan only this directory instead of
        /// re-scanning the full rig.
        loom_dir: String,
    },
    /// A new knot `.md` file was created in a loom directory.
    KnotAdded {
        loom_id: LoomId,
        knot: Knot,
    },
    /// An existing knot `.md` file was modified in a loom directory.
    KnotModified {
        loom_id: LoomId,
        knot: Knot,
    },
    /// A knot `.md` file was deleted from a loom directory.
    KnotDeleted {
        loom_id: LoomId,
        knot_id: KnotId,
    },
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value_objects::StrandSource;
    use std::path::PathBuf;

    use crate::application::usecases::test_fixtures::KnotBuilder;

    // ── build_listener_context Tests (Phase 2) ────────────────────────

    fn make_test_knot(id: &str) -> Knot {
        KnotBuilder::new(id)
            .with_instructions("test")
            .build()
    }

    fn make_event_knot(
        id: &str,
        producer_knot: &str,
        event_id: &str,
        event_description: Option<String>,
    ) -> Knot {
        KnotBuilder::new(id)
            .with_instructions("test")
            .with_strand_source(StrandSource::EventUri {
                producer_knot: producer_knot.to_string(),
                event_id: event_id.to_string(),
            })
            .with_event_description(event_description)
            .build()
    }

    /// No consumers listening for events — returns empty string.
    #[test]
    fn build_listener_context_no_consumers_returns_empty() {
        let producer = make_test_knot("plan-creator");
        let context = build_listener_context(&producer, &[]);
        assert!(
            context.is_empty(),
            "no consumers should produce empty context: '{}'",
            context
        );
    }

    /// No consumers listening — only filesystem knots — returns empty.
    #[test]
    fn build_listener_context_only_filesystem_knots_returns_empty() {
        let producer = make_test_knot("plan-creator");
        let filesystem_knot = make_test_knot("reviewer");
        let context = build_listener_context(&producer, &[filesystem_knot]);
        assert!(
            context.is_empty(),
            "only filesystem knots should produce empty context: '{}'",
            context
        );
    }

    /// Output starts with the `## Agent Events` heading.
    #[test]
    fn build_listener_context_output_has_heading() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &[consumer]);
        assert!(
            context.starts_with("## Agent Events\n"),
            "context should start with heading: {}",
            context
        );
    }

    /// Output contains the event description from the consumer knot.
    #[test]
    fn build_listener_context_output_contains_event_description() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created for the first time".to_string()),
        );
        let context = build_listener_context(&producer, &[consumer]);
        assert!(
            context.contains("When a plan is created for the first time"),
            "context should contain event description: {}",
            context
        );
    }

    /// Output does NOT contain consumer knot names in the event list.
    /// Only the event ID and description are visible to the producer.
    #[test]
    fn build_listener_context_output_does_not_contain_consumer_knot_names() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "secret-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &[consumer]);
        // The consumer knot ID should NOT appear in the output
        assert!(
            !context.contains("secret-validator"),
            "context should NOT contain consumer knot name: {}",
            context
        );
    }

    /// Output contains instructions for emitting `event: None`.
    #[test]
    fn build_listener_context_output_instructs_event_none() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &[consumer]);
        assert!(
            context.contains("event: None"),
            "context should instruct to emit 'event: None': {}",
            context
        );
    }

    /// Output instructs the producer to include a `description` field.
    #[test]
    fn build_listener_context_output_requires_description_field() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &[consumer]);
        assert!(
            context.contains("description:")
                && context.contains("<short summary"),
            "context should require description field: {}",
            context
        );
    }

    /// Single consumer triggers context with its event description.
    #[test]
    fn build_listener_context_single_consumer_triggers_context() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let context = build_listener_context(&producer, &[consumer]);
        assert!(!context.is_empty());
        assert!(context.contains("## Agent Events"));
        assert!(context.contains("PlanCreated"));
        assert!(context.contains("When a plan is created"));
        assert!(context.contains("event: None"));
        assert!(context.contains("description:"));
    }

    /// Multiple consumers listening for the same event deduplicate
    /// — only one entry appears in the output.
    #[test]
    fn build_listener_context_multiple_consumers_same_event_deduplicates() {
        let producer = make_test_knot("plan-creator");
        let consumer1 = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let consumer2 = make_event_knot(
            "plan-auditor",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created for audit".to_string()),
        );
        let context = build_listener_context(&producer, &[consumer1, consumer2]);
        // Count occurrences of "PlanCreated" in the event list (should appear
        // only once as a bullet point)
        let count = context.matches("- `PlanCreated`").count();
        assert_eq!(count, 1, "same event from multiple consumers should deduplicate: {}", context);
    }

    /// Multiple different events from the same producer each appear.
    #[test]
    fn build_listener_context_multiple_different_events() {
        let producer = make_test_knot("plan-creator");
        let consumer1 = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let consumer2 = make_event_knot(
            "plan-fixer",
            "plan-creator",
            "ValidationFailed",
            Some("When validation fails".to_string()),
        );
        let context = build_listener_context(&producer, &[consumer1, consumer2]);
        assert!(context.contains("PlanCreated"));
        assert!(context.contains("ValidationFailed"));
        assert!(context.contains("When a plan is created"));
        assert!(context.contains("When validation fails"));
    }

    /// When event-description is None, a generic message is used.
    #[test]
    fn build_listener_context_generic_message_when_event_description_none() {
        let producer = make_test_knot("plan-creator");
        let consumer = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            None, // no event-description
        );
        let context = build_listener_context(&producer, &[consumer]);
        assert!(
            context.contains("If this event occurs, emit a structured event block in your final response."),
            "should use generic message when event-description is None: {}",
            context
        );
        // Consumer knot name should not appear even in generic message
        assert!(
            !context.contains("plan-validator"),
            "generic message should not contain consumer knot name: {}",
            context
        );
    }

    /// Consumers for a different producer knot do not affect output.
    #[test]
    fn build_listener_context_other_producer_no_effect() {
        let producer = make_test_knot("plan-creator");
        let other_consumer = make_event_knot(
            "other-validator",
            "other-producer", // different producer
            "OtherEvent",
            Some("Some event".to_string()),
        );
        let context = build_listener_context(&producer, &[other_consumer]);
        assert!(context.is_empty());
    }

    /// Mixed: one consumer for this producer, one for another.
    /// Only the matching consumer appears.
    #[test]
    fn build_listener_context_mixed_consumers_only_matching() {
        let producer = make_test_knot("plan-creator");
        let matching = make_event_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let non_matching = make_event_knot(
            "other-validator",
            "other-producer",
            "OtherEvent",
            Some("Some event".to_string()),
        );
        let context = build_listener_context(&producer, &[matching, non_matching]);
        assert!(!context.is_empty());
        assert!(context.contains("PlanCreated"));
        assert!(context.contains("When a plan is created"));
        assert!(!context.contains("OtherEvent"));
        assert!(!context.contains("other-validator"));
    }

    // ── AgentEvent Tests ─────────────────────────────────────────

    #[test]
    fn agent_event_construction() {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-001".to_string());
        payload.insert(
            "description".to_string(),
            "Implementation plan".to_string(),
        );

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            payload,
            body: None,
        };

        assert_eq!(event.event_id, "PlanCreated");
        assert_eq!(event.payload.len(), 2);
        assert_eq!(
            event.payload.get("plan"),
            Some(&"PLAN-001".to_string())
        );
        assert_eq!(event.body, None);
    }

    #[test]
    fn agent_event_serialisation_roundtrip() {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-007".to_string());

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            payload,
            body: None,
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn agent_event_empty_payload_defaults() {
        let event = AgentEvent {
            event_id: "Something".to_string(),
            payload: HashMap::new(),
            body: None,
        };

        // Serialize and deserialize — empty payload should survive
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        assert!(deserialized.payload.is_empty());
    }

    #[test]
    fn agent_event_missing_payload_in_json_defaults_to_empty() {
        // JSON without a payload field should deserialize with empty HashMap
        let json = r#"{"event_id":"Test"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_id, "Test");
        assert!(event.payload.is_empty());
    }

    #[test]
    fn agent_event_with_body_roundtrips_through_json() {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-010".to_string());

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            payload,
            body: Some(
                "The plan covers three phases: planning, review, and approval.".to_string(),
            ),
        };

        let json = serde_json::to_string(&event).unwrap();
        // Verify body appears in JSON
        assert!(
            json.contains("body"),
            "JSON should contain body field: {}",
            json
        );
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        assert_eq!(
            deserialized.body.as_deref(),
            Some("The plan covers three phases: planning, review, and approval.")
        );
    }

    #[test]
    fn agent_event_with_none_body_survives_serialisation() {
        let event = AgentEvent {
            event_id: "NoBodyEvent".to_string(),
            payload: HashMap::new(),
            body: None,
        };

        let json = serde_json::to_string(&event).unwrap();
        // body is skip_serializing_if = is_none, so it should not appear
        assert!(
            !json.contains("body"),
            "JSON should not contain body when None: {}",
            json
        );
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        assert_eq!(deserialized.body, None);
    }

    #[test]
    fn agent_event_missing_body_in_json_defaults_to_none() {
        // JSON without a body field should deserialize with None
        let json = r#"{"event_id":"Test","payload":{"key":"val"}}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_id, "Test");
        assert_eq!(event.body, None);
    }

    #[test]
    fn agent_event_with_empty_string_body_preserved() {
        let event = AgentEvent {
            event_id: "EmptyBody".to_string(),
            payload: HashMap::new(),
            body: Some(String::new()),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(
            json.contains("body"),
            "JSON should contain body even when empty string: {}",
            json
        );
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
        assert_eq!(deserialized.body.as_deref(), Some(""));
    }

    #[test]
    fn strand_event_types() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));

        let created = StrandEvent::Created {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
        };
        let modified = StrandEvent::Modified {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
        };
        let deleted = StrandEvent::Deleted {
            loom_id,
            knot_id,
            strand_path,
        };

        // Verify all three variants exist and carry correct data
        match created {
            StrandEvent::Created {
                loom_id: ref lid,
                knot_id: ref kid,
                strand_path: ref sp,
            } => {
                assert_eq!(*lid, LoomId("prds".to_string()));
                assert_eq!(*kid, KnotId("review".to_string()));
                assert_eq!(sp.0, PathBuf::from("project/prds/my-prd.md"));
            }
            _ => panic!("Expected Created variant"),
        }

        match modified {
            StrandEvent::Modified {
                loom_id: ref lid,
                knot_id: ref kid,
                strand_path: ref sp,
            } => {
                assert_eq!(*lid, LoomId("prds".to_string()));
                assert_eq!(*kid, KnotId("review".to_string()));
                assert_eq!(sp.0, PathBuf::from("project/prds/my-prd.md"));
            }
            _ => panic!("Expected Modified variant"),
        }

        match deleted {
            StrandEvent::Deleted {
                loom_id: ref lid,
                knot_id: ref kid,
                strand_path: ref sp,
            } => {
                assert_eq!(*lid, LoomId("prds".to_string()));
                assert_eq!(*kid, KnotId("review".to_string()));
                assert_eq!(sp.0, PathBuf::from("project/prds/my-prd.md"));
            }
            _ => panic!("Expected Deleted variant"),
        }
    }

    #[test]
    fn tieoff_produced_event() {
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let tie_off_path = TieOffPath(PathBuf::from("output/review.md"));

        let event = TieOffProduced {
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            tie_off_path: tie_off_path.clone(),
        };

        assert_eq!(event.knot_id, knot_id);
        assert_eq!(event.strand_path, strand_path);
        assert_eq!(event.tie_off_path, tie_off_path);

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: TieOffProduced = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn processing_failed_event() {
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let error_message = "Agent returned non-zero exit code".to_string();

        let event = ProcessingFailed {
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            error_message: error_message.clone(),
        };

        assert_eq!(event.knot_id, knot_id);
        assert_eq!(event.strand_path, strand_path);
        assert_eq!(event.error_message, error_message);

        // Verify error details are preserved through serialisation
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ProcessingFailed = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.error_message, error_message);
        assert_eq!(deserialized, event);
    }

    #[test]
    fn loom_event_types() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));

        let ts = "2026-06-10T12:00:00Z".to_string();
        let knot_registered = LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            timestamp: ts.clone(),
        };
        let loom_started = LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
            timestamp: ts.clone(),
        };
        let loom_stopped = LoomEvent::LoomStopped {
            loom_id: loom_id.clone(),
            timestamp: ts.clone(),
        };
        let strand_processed = LoomEvent::StrandProcessed {
            loom_id: loom_id.clone(),
            strand_path: strand_path.clone(),
            error: None,
            timestamp: ts.clone(),
        };

        // Verify KnotRegistered
        match knot_registered {
            LoomEvent::KnotRegistered {
                loom_id: ref lid,
                knot_id: ref kid,
                timestamp: ref ts,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*ts, "2026-06-10T12:00:00Z");
            }
            _ => panic!("Expected KnotRegistered variant"),
        }

        // Verify LoomStarted
        match loom_started {
            LoomEvent::LoomStarted {
                loom_id: ref lid,
                timestamp: ref ts,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*ts, "2026-06-10T12:00:00Z");
            }
            _ => panic!("Expected LoomStarted variant"),
        }

        // Verify LoomStopped
        match loom_stopped {
            LoomEvent::LoomStopped {
                loom_id: ref lid,
                timestamp: ref ts,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*ts, "2026-06-10T12:00:00Z");
            }
            _ => panic!("Expected LoomStopped variant"),
        }

        // Verify StrandProcessed
        match strand_processed {
            LoomEvent::StrandProcessed {
                loom_id: ref lid,
                strand_path: ref sp,
                error,
                timestamp: ref ts,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*sp, strand_path);
                assert!(error.is_none());
                assert_eq!(*ts, "2026-06-10T12:00:00Z");
            }
            _ => panic!("Expected StrandProcessed variant"),
        }
    }

    #[test]
    fn knot_registered_event() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());

        let event = KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
        };

        assert_eq!(event.loom_id, loom_id);
        assert_eq!(event.knot_id, knot_id);

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: KnotRegistered = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn strand_event_serialisation() {
        let created = StrandEvent::Created {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
        };

        let json = serde_json::to_string(&created).unwrap();
        let deserialized: StrandEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, created);

        // Also verify Modified and Deleted round-trip
        let modified = StrandEvent::Modified {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
        };
        let json = serde_json::to_string(&modified).unwrap();
        let deserialized: StrandEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, modified);

        let deleted = StrandEvent::Deleted {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
        };
        let json = serde_json::to_string(&deleted).unwrap();
        let deserialized: StrandEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, deleted);
    }

    #[test]
    fn loom_event_serialisation() {
        let ts = "2026-06-10T12:00:00Z".to_string();
        let knot_registered = LoomEvent::KnotRegistered {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            timestamp: ts.clone(),
        };
        let json = serde_json::to_string(&knot_registered).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, knot_registered);

        let loom_started = LoomEvent::LoomStarted {
            loom_id: LoomId("prds".to_string()),
            timestamp: ts.clone(),
        };
        let json = serde_json::to_string(&loom_started).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, loom_started);

        let loom_stopped = LoomEvent::LoomStopped {
            loom_id: LoomId("prds".to_string()),
            timestamp: ts.clone(),
        };
        let json = serde_json::to_string(&loom_stopped).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, loom_stopped);

        let strand_processed = LoomEvent::StrandProcessed {
            loom_id: LoomId("prds".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
            error: None,
            timestamp: ts.clone(),
        };
        let json = serde_json::to_string(&strand_processed).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, strand_processed);
    }

    #[test]
    fn loom_event_strand_processed_with_error() {
        let event = LoomEvent::StrandProcessed {
            loom_id: LoomId("prds".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
            error: Some("agent crashed".to_string()),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        };

        // Verify error field is present
        match &event {
            LoomEvent::StrandProcessed { error, .. } => {
                assert_eq!(error.as_deref(), Some("agent crashed"));
            }
            _ => panic!("Expected StrandProcessed"),
        }

        // Verify error survives serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn loom_event_knot_processing() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let ts = "2026-06-10T12:00:00Z".to_string();

        let event = LoomEvent::KnotProcessing {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotProcessing {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotProcessing variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn loom_event_knot_completed() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let tie_off_path = TieOffPath(PathBuf::from("output/review.md"));
        let ts = "2026-06-10T12:00:00Z".to_string();

        let event = LoomEvent::KnotCompleted {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            tie_off_path: tie_off_path.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotCompleted {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                tie_off_path: tp,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(*tp, tie_off_path);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotCompleted variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn loom_event_knot_failed() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let error = "Agent returned non-zero exit code".to_string();
        let ts = "2026-06-10T12:00:00Z".to_string();

        let event = LoomEvent::KnotFailed {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            error: error.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotFailed {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                error: msg,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(msg.as_str(), error);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotFailed variant"),
        }

        // Verify error survives serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    fn make_knot(id: &str) -> Knot {
        KnotBuilder::new(id)
            .with_instructions("Test instructions.")
            .build()
    }

    /// `ConfigEvent::LoomAdded` carries both `loom_id` and `loom_dir`.
    /// Verifies the variant shape and JSON round-trip serialisation.
    #[test]
    fn config_event_loom_added_has_path() {
        let loom_id = LoomId("my-loom".to_string());
        let loom_dir = "/project/rig/my-loom".to_string();

        let event = ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: loom_dir.clone(),
        };

        // Verify both fields are present
        match &event {
            ConfigEvent::LoomAdded {
                loom_id: lid,
                loom_dir: dir,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(dir, &loom_dir);
            }
            _ => panic!("Expected LoomAdded variant"),
        }

        // Verify JSON serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: ConfigEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn config_event_types() {
        let loom_id = LoomId("prds".to_string());
        let knot = make_knot("review");
        let knot_id = KnotId("review".to_string());

        // Build all four variants
        let loom_added = ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: "/project/rig/prds-loom".to_string(),
        };
        let knot_added = ConfigEvent::KnotAdded {
            loom_id: loom_id.clone(),
            knot: knot.clone(),
        };
        let knot_modified = ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: knot.clone(),
        };
        let knot_deleted = ConfigEvent::KnotDeleted {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
        };

        // Verify LoomAdded carries correct data
        match &loom_added {
            ConfigEvent::LoomAdded {
                loom_id: lid,
                loom_dir,
            } => {
                assert_eq!(*lid, LoomId("prds".to_string()));
                assert_eq!(loom_dir, &"/project/rig/prds-loom".to_string());
            }
            _ => panic!("Expected LoomAdded variant"),
        }

        // Verify KnotAdded carries correct data
        match &knot_added {
            ConfigEvent::KnotAdded {
                loom_id: lid,
                knot: k,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(k.id, KnotId("review".to_string()));
            }
            _ => panic!("Expected KnotAdded variant"),
        }

        // Verify KnotModified carries correct data
        match &knot_modified {
            ConfigEvent::KnotModified {
                loom_id: lid,
                knot: k,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(k.id, KnotId("review".to_string()));
            }
            _ => panic!("Expected KnotModified variant"),
        }

        // Verify KnotDeleted carries correct data
        match &knot_deleted {
            ConfigEvent::KnotDeleted {
                loom_id: lid,
                knot_id: kid,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
            }
            _ => panic!("Expected KnotDeleted variant"),
        }

        // Verify serialisation round-trip for all variants
        let events: Vec<ConfigEvent> =
            vec![loom_added, knot_added, knot_modified, knot_deleted];
        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let deserialized: ConfigEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(
                deserialized, *event,
                "round-trip failed for variant"
            );
        }
    }

    #[test]
    fn loom_event_serialisation_all_variants() {
        // Verify all 9 variants round-trip through JSON
        let loom_id = LoomId("all".to_string());
        let knot_id = KnotId("k1".to_string());
        let strand_path = StrandPath(PathBuf::from("in.md"));
        let tie_off_path = TieOffPath(PathBuf::from("out.md"));
        let ts = "2026-06-10T12:00:00Z".to_string();

        let events: Vec<LoomEvent> = vec![
            LoomEvent::KnotRegistered {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::LoomStarted {
                loom_id: loom_id.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::LoomStopped {
                loom_id: loom_id.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::StrandProcessed {
                loom_id: loom_id.clone(),
                strand_path: strand_path.clone(),
                error: None,
                timestamp: ts.clone(),
            },
            LoomEvent::KnotProcessing {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::KnotCompleted {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                tie_off_path: tie_off_path.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::KnotFailed {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                error: "boom".to_string(),
                timestamp: ts.clone(),
            },
            LoomEvent::KnotDeregistered {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                timestamp: ts.clone(),
            },
            LoomEvent::KnotParseWarning {
                loom_id: loom_id.clone(),
                knot_file_name: "legacy.md".to_string(),
                message: "unknown property 'tie-off-dir'".to_string(),
                timestamp: ts.clone(),
            },
            LoomEvent::DirectoryCreated {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                directory: "/project/rig/prds-loom/strands".to_string(),
                timestamp: ts.clone(),
            },
            LoomEvent::StrandIgnored {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                reason: "binary file".to_string(),
                timestamp: ts.clone(),
            },
            LoomEvent::StrandSkipped {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                reason: "missing file (unknown pattern)".to_string(),
                timestamp: ts.clone(),
            },
            LoomEvent::SessionResumed {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                session_id: "sess-abc123".to_string(),
                attempt: 2,
                timestamp: ts.clone(),
            },
            LoomEvent::KnotEmptyResponse {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                attempt: 3,
                timestamp: ts.clone(),
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, *event, "round-trip failed for variant");
        }
    }

    /// `DirectoryCreated` carries `loom_id`, `knot_id`, `directory`, and
    /// `timestamp`. Verifies the variant shape and JSON round-trip.
    #[test]
    fn loom_event_directory_created_serialisation() {
        let loom_id = LoomId("auto-strand-dir-loom".to_string());
        let knot_id = KnotId("codegen".to_string());
        let directory = "/project/rig/auto-strand-dir-loom/strands".to_string();
        let ts = "2026-06-17T09:00:00Z".to_string();

        let event = LoomEvent::DirectoryCreated {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            directory: directory.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::DirectoryCreated {
                loom_id: lid,
                knot_id: kid,
                directory: dir,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(dir, &directory);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected DirectoryCreated variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn loom_event_knot_parse_warning() {
        let loom_id = LoomId("prds".to_string());
        let ts = "2026-06-10T12:00:00Z".to_string();

        let event = LoomEvent::KnotParseWarning {
            loom_id: loom_id.clone(),
            knot_file_name: "legacy-knot.md".to_string(),
            message: "unknown property 'tie-off-dir' in knot frontmatter (not used)".to_string(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotParseWarning {
                loom_id: lid,
                knot_file_name,
                message,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*knot_file_name, "legacy-knot.md");
                assert!(message.contains("tie-off-dir"));
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotParseWarning variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn riglog_event_timeout_exceeded() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path = StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let error = "Agent session exceeded 60s deadline".to_string();
        let ts = "2026-06-14T10:00:00Z".to_string();

        let event = RigLogEvent::TimeoutExceeded {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            error: error.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            RigLogEvent::TimeoutExceeded {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                error: msg,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(msg.as_str(), error);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected TimeoutExceeded variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: RigLogEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn riglog_event_queue_idle() {
        let ts = "2026-06-14T10:05:00Z".to_string();

        let event = RigLogEvent::QueueIdle {
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            RigLogEvent::QueueIdle { timestamp: t } => {
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected QueueIdle variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: RigLogEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn riglog_event_serialisation_all_variants() {
        let loom_id = LoomId("ops".to_string());
        let knot_id = KnotId("slow-review".to_string());
        let strand_path = StrandPath(PathBuf::from("input/data.md"));
        let ts = "2026-06-14T12:00:00Z".to_string();

        let events: Vec<RigLogEvent> = vec![
            RigLogEvent::TimeoutExceeded {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                error: "deadline exceeded after 600s".to_string(),
                timestamp: ts.clone(),
            },
            RigLogEvent::QueueIdle {
                timestamp: ts.clone(),
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let deserialized: RigLogEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, *event, "round-trip failed for variant");
        }
    }

    /// `LoomEvent::StrandIgnored` carries `loom_id`, `knot_id`,
    /// `strand_path`, `reason`, and `timestamp`. Verifies the variant
    /// shape and JSON round-trip serialisation.
    #[test]
    fn loom_event_strand_ignored() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path =
            StrandPath(PathBuf::from("project/prds/image.png"));
        let reason = "binary file".to_string();
        let ts = "2026-06-19T10:00:00Z".to_string();

        let event = LoomEvent::StrandIgnored {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            reason: reason.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::StrandIgnored {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                reason: r,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(r, &reason);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected StrandIgnored variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    /// `LoomEvent::StrandSkipped` carries `loom_id`, `knot_id`,
    /// `strand_path`, `reason`, and `timestamp`. Verifies the variant
    /// shape and JSON round-trip serialisation.
    #[test]
    fn loom_event_strand_skipped() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path =
            StrandPath(PathBuf::from("project/prds/sedABC123"));
        let reason = "missing file (unknown pattern)".to_string();
        let ts = "2026-06-24T10:00:00Z".to_string();

        let event = LoomEvent::StrandSkipped {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            reason: reason.clone(),
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::StrandSkipped {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                reason: r,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(r, &reason);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected StrandSkipped variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    /// `LoomEvent::SessionResumed` carries `loom_id`, `knot_id`,
    /// `strand_path`, `session_id`, `attempt`, and `timestamp`.
    /// Verifies the variant shape and JSON round-trip serialisation.
    #[test]
    fn session_resumed_event_serialisation() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path =
            StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let session_id = "sess-resume-42".to_string();
        let attempt: u32 = 3;
        let ts = "2026-06-28T14:00:00Z".to_string();

        let event = LoomEvent::SessionResumed {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            session_id: session_id.clone(),
            attempt,
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::SessionResumed {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                session_id: sid,
                attempt: a,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(sid, &session_id);
                assert_eq!(*a, attempt);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected SessionResumed variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    /// `LoomEvent::KnotEmptyResponse` carries `loom_id`, `knot_id`,
    /// `strand_path`, `attempt`, and `timestamp`. Verifies the variant
    /// shape and JSON round-trip serialisation.
    #[test]
    fn loom_event_knot_empty_response() {
        let loom_id = LoomId("prds".to_string());
        let knot_id = KnotId("review".to_string());
        let strand_path =
            StrandPath(PathBuf::from("project/prds/my-prd.md"));
        let attempt: u32 = 2;
        let ts = "2026-06-28T15:00:00Z".to_string();

        let event = LoomEvent::KnotEmptyResponse {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            attempt,
            timestamp: ts.clone(),
        };

        // Verify fields via pattern matching
        match &event {
            LoomEvent::KnotEmptyResponse {
                loom_id: lid,
                knot_id: kid,
                strand_path: sp,
                attempt: a,
                timestamp: t,
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
                assert_eq!(*sp, strand_path);
                assert_eq!(*a, attempt);
                assert_eq!(t, &ts);
            }
            _ => panic!("Expected KnotEmptyResponse variant"),
        }

        // Verify serialisation round-trip
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LoomEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }
}
