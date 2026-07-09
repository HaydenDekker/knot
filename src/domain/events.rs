use serde::{Deserialize, Serialize};

use crate::domain::entities::{
    Knot, KnotId, LoomId, StrandPath, TieOffPath,
};

// ── Agent Events (Intent-Based Routing) ────────────────────────────────────

use std::collections::HashMap;

/// A consumer knot's declaration of interest in a specific event.
///
/// Each intent says: "I want to hear about `event_id` when `target_knot`
/// emits it." The `event_description` tells the producer *when* to emit
/// the event and *what data* to include.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    /// Which knot may emit this event (the producer/target).
    #[serde(rename = "target-knot")]
    pub target_knot: String,
    /// Unique event identifier (e.g. `PlanCreated`).
    #[serde(rename = "event-id")]
    pub event_id: String,
    /// Human-readable description — when the event fires and what data it
    /// should contain. Injected into the target knot's prompt as instructions.
    #[serde(rename = "event-description")]
    pub event_description: String,
}

/// A structured agent-to-agent event emitted in a tie-off.
///
/// When a target knot is instructed to emit an event (via intent-based routing
/// context injection), it writes a structured block in its tie-off body. The
/// `event:` key signals that the block contains event data. The `target-knot`
/// field identifies which knot emitted it. All other keys become the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvent {
    /// Unique event identifier (e.g. `PlanCreated`).
    pub event_id: String,
    /// Name of the knot that emitted this event.
    pub target_knot: String,
    /// Arbitrary key-value pairs carrying event data.
    /// Includes fields like `plan`, `description`, `source`, etc.
    #[serde(default)]
    pub payload: HashMap<String, String>,
}

// ── Intent Matching ────────────────────────────────────────────────────────

/// Check whether an [`AgentEvent`] satisfies an [`Intent`] declaration.
///
/// An intent is satisfied when **both** of the following hold:
/// 1. `event.event_id == intent.event_id` (exact match)
/// 2. `event.target_knot == intent.target_knot` (exact match)
///
/// The event's payload is **not** used for matching — it carries data the
/// consumer will read after the match succeeds.
///
/// ## Example
///
/// A consumer knot declares:
/// ```yaml
/// listens-for:
///   - target-knot: plan-creator
///     event-id: PlanCreated
///     event-description: "Emit when a plan is created"
/// ```
///
/// An event emitted by `plan-creator` with `event: PlanCreated` matches.
/// An event from a different knot, or a different event-id, does not.
pub fn matches_intent(event: &AgentEvent, intent: &Intent) -> bool {
    event.event_id == intent.event_id && event.target_knot == intent.target_knot
}

// ── Context Injection ──────────────────────────────────────────────────────

/// Build the listener context block to inject at the start of a target
/// knot's prompt.
///
/// Scans all knots' `listens-for` declarations and collects those where
/// `target-knot` matches `knot.id.0`. Groups by `event-id` — if multiple
/// consumers listen for the same event from the same knot, they are merged
/// into one event block (not duplicated).
///
/// Returns an empty string when no consumers are listening (no injection
/// needed).
///
/// The returned markdown is designed to be prepended to the knot's
/// instructions before execution.
///
/// ## Example output
///
/// ```markdown
/// Before undertaking your task, note that other knots are listening
/// for events you may emit. If an event occurs during your work,
/// include an explicit event object in your tie-off using the format
/// shown.
///
/// Events you may emit:
/// - `PlanCreated` — Emitted when a new plan is created for the first time.
///   Emit in your tie-off:
///   ```
///   event: PlanCreated
///   target-knot: plan-creator
///   plan: <plan-id>
///   description: <description>
///   scope: <scope>
///   ```
/// ```
///
pub fn build_listener_context(knot: &Knot, all_knots: &[Knot]) -> String {
    // Collect all intents where this knot is the target.
    let mut matching_intents: Vec<&Intent> = Vec::new();
    for other in all_knots {
        for intent in &other.listens_for {
            if intent.target_knot == knot.id.0 {
                matching_intents.push(intent);
            }
        }
    }

    // No listeners — no injection needed.
    if matching_intents.is_empty() {
        return String::new();
    }

    // Group by event-id. Use a Vec of (event_id, description, description)
    // preserving insertion order (first seen event-id wins).
    let mut seen_ids = std::collections::HashSet::new();
    let mut unique_events: Vec<&Intent> = Vec::new();
    for intent in matching_intents {
        if seen_ids.insert(intent.event_id.clone()) {
            unique_events.push(intent);
        }
    }

    let mut output = String::from(
        "Before undertaking your task, note that other knots are \
         listening for events you may emit. If an event occurs \
         during your work, include an explicit event object in \
         your tie-off using the format shown.\n\n",
    );
    output.push_str("Events you may emit:\n");

    for intent in &unique_events {
        output.push_str(&format!(
            "- `{}` — {}\n",
            intent.event_id, intent.event_description
        ));
        output.push_str("  Emit in your tie-off:\n");
        output.push_str("  ```\n");
        output.push_str(&format!("  event: {}\n", intent.event_id));
        output.push_str(&format!("  target-knot: {}\n", knot.id.0));
        // The event-description field describes the event semantically.
        // The actual payload keys the agent should emit come from the
        // knot's instructions and the intent's description. We don't
        // know payload schema here — the agent formats it from context.
        // So we only show the mandatory fields.
        output.push_str("  ```\n");
    }

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
    use crate::domain::entities::PromptTemplate;
    use std::path::PathBuf;

    // ── Intent Tests ─────────────────────────────────────────────

    #[test]
    fn intent_construction() {
        let intent = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a new plan is created.".to_string(),
        };

        assert_eq!(intent.target_knot, "plan-creator");
        assert_eq!(intent.event_id, "PlanCreated");
        assert_eq!(
            intent.event_description,
            "Emit when a new plan is created."
        );
    }

    #[test]
    fn intent_serialisation_roundtrip() {
        let intent = Intent {
            target_knot: "implementation-planner".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a plan is created for the first time."
                .to_string(),
        };

        let json = serde_json::to_string(&intent).unwrap();
        let deserialized: Intent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, intent);
    }

    // ── Intent Matching Tests ────────────────────────────────────

    #[test]
    fn matches_intent_exact_match_on_event_id_and_target_knot() {
        let intent = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a plan is created.".to_string(),
        };
        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            target_knot: "plan-creator".to_string(),
            payload: HashMap::new(),
        };

        assert!(matches_intent(&event, &intent));
    }

    #[test]
    fn matches_intent_no_match_when_event_id_differs() {
        let intent = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a plan is created.".to_string(),
        };
        let event = AgentEvent {
            event_id: "PlanApproved".to_string(),
            target_knot: "plan-creator".to_string(),
            payload: HashMap::new(),
        };

        assert!(!matches_intent(&event, &intent));
    }

    #[test]
    fn matches_intent_no_match_when_target_knot_differs() {
        let intent = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a plan is created.".to_string(),
        };
        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            target_knot: "other-knot".to_string(),
            payload: HashMap::new(),
        };

        assert!(!matches_intent(&event, &intent));
    }

    #[test]
    fn matches_intent_no_match_when_both_differ() {
        let intent = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a plan is created.".to_string(),
        };
        let event = AgentEvent {
            event_id: "ReviewDone".to_string(),
            target_knot: "reviewer".to_string(),
            payload: HashMap::new(),
        };

        assert!(!matches_intent(&event, &intent));
    }

    #[test]
    fn matches_intent_payload_does_not_affect_matching() {
        let intent = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a plan is created.".to_string(),
        };

        // Event with rich payload — should still match
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-001".to_string());
        payload.insert("scope".to_string(), "event routing".to_string());
        let event_with_payload = AgentEvent {
            event_id: "PlanCreated".to_string(),
            target_knot: "plan-creator".to_string(),
            payload,
        };

        // Event with empty payload — should also match
        let event_empty = AgentEvent {
            event_id: "PlanCreated".to_string(),
            target_knot: "plan-creator".to_string(),
            payload: HashMap::new(),
        };

        assert!(matches_intent(&event_with_payload, &intent));
        assert!(matches_intent(&event_empty, &intent));
    }

    #[test]
    fn matches_intent_case_sensitive_event_id() {
        let intent = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a plan is created.".to_string(),
        };
        let event = AgentEvent {
            event_id: "plancreated".to_string(),
            target_knot: "plan-creator".to_string(),
            payload: HashMap::new(),
        };

        // event_id match is exact (case-sensitive)
        assert!(!matches_intent(&event, &intent));
    }

    #[test]
    fn matches_intent_case_sensitive_target_knot() {
        let intent = Intent {
            target_knot: "Plan-Creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a plan is created.".to_string(),
        };
        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            target_knot: "plan-creator".to_string(),
            payload: HashMap::new(),
        };

        // target_knot match is exact (case-sensitive)
        assert!(!matches_intent(&event, &intent));
    }

    #[test]
    fn matches_intent_multiple_intents_different_events() {
        // Two intents on the same target knot but different event-ids.
        // Only the matching intent should return true.
        let intent_created = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when a plan is created.".to_string(),
        };
        let intent_approved = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanApproved".to_string(),
            event_description: "Emit when a plan is approved.".to_string(),
        };

        let created_event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            target_knot: "plan-creator".to_string(),
            payload: HashMap::new(),
        };

        assert!(matches_intent(&created_event, &intent_created));
        assert!(!matches_intent(&created_event, &intent_approved));
    }

    #[test]
    fn matches_intent_same_event_id_different_producers() {
        // Two intents for the same event-id but from different knots.
        // Only the intent whose target_knot matches should return true.
        let intent_from_creator = Intent {
            target_knot: "plan-creator".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when plan-creator makes a plan.".to_string(),
        };
        let intent_from_reviewer = Intent {
            target_knot: "reviewer".to_string(),
            event_id: "PlanCreated".to_string(),
            event_description: "Emit when reviewer creates a plan.".to_string(),
        };

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            target_knot: "plan-creator".to_string(),
            payload: HashMap::new(),
        };

        assert!(matches_intent(&event, &intent_from_creator));
        assert!(!matches_intent(&event, &intent_from_reviewer));
    }

    // ── Context Injection Tests ──────────────────────────────────

    fn make_test_knot(
        id: &str,
        listens_for: Vec<Intent>,
    ) -> Knot {
        Knot {
            id: KnotId(id.to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "test".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
            listens_for,
        }
    }

    #[test]
    fn build_listener_context_no_listeners_returns_empty() {
        let producer = make_test_knot("plan-creator", vec![]);
        let consumer = make_test_knot(
            "consumer",
            vec![Intent {
                target_knot: "other-knot".to_string(),
                event_id: "PlanCreated".to_string(),
                event_description: "When a plan is created".to_string(),
            }],
        );

        let ctx = build_listener_context(&producer, &[consumer.clone()]);
        assert!(
            ctx.is_empty(),
            "should be empty when no consumers listen to this knot"
        );
    }

    #[test]
    fn build_listener_context_no_knots_returns_empty() {
        let producer = make_test_knot("plan-creator", vec![]);
        let ctx = build_listener_context(&producer, &[]);
        assert!(ctx.is_empty());
    }

    #[test]
    fn build_listener_context_single_consumer_single_event() {
        let producer = make_test_knot("plan-creator", vec![]);
        let consumer = make_test_knot(
            "consumer",
            vec![Intent {
                target_knot: "plan-creator".to_string(),
                event_id: "PlanCreated".to_string(),
                event_description: "Emit when a plan is created.".to_string(),
            }],
        );

        let ctx = build_listener_context(&producer, &[consumer.clone()]);

        assert!(
            !ctx.is_empty(),
            "should produce context when consumer listens"
        );
        assert!(
            ctx.contains("Before undertaking your task"),
            "should contain preamble"
        );
        assert!(
            ctx.contains("Events you may emit:"),
            "should contain event header"
        );
        assert!(
            ctx.contains("`PlanCreated`"),
            "should contain event-id: {}",
            ctx
        );
        assert!(
            ctx.contains("Emit when a plan is created."),
            "should contain event description"
        );
        assert!(
            ctx.contains("event: PlanCreated"),
            "should show emit format"
        );
        assert!(
            ctx.contains("target-knot: plan-creator"),
            "should show target-knot in format block"
        );
    }

    #[test]
    fn build_listener_context_deduplicates_same_event_id() {
        // Two consumers listening for the same event-id from the same
        // knot — should produce only one event block.
        let producer = make_test_knot("plan-creator", vec![]);
        let consumer_a = make_test_knot(
            "consumer-a",
            vec![Intent {
                target_knot: "plan-creator".to_string(),
                event_id: "PlanCreated".to_string(),
                event_description: "Emit when a plan is created.".to_string(),
            }],
        );
        let consumer_b = make_test_knot(
            "consumer-b",
            vec![Intent {
                target_knot: "plan-creator".to_string(),
                event_id: "PlanCreated".to_string(),
                event_description: "When the plan is first created.".to_string(),
            }],
        );

        let ctx =
            build_listener_context(&producer, &[consumer_a, consumer_b]);

        // Should contain the event header once
        let event_count = ctx.matches("`PlanCreated`").count();
        assert_eq!(
            event_count, 1,
            "should deduplicate: only one PlanCreated block, got:\n{}",
            ctx
        );
    }

    #[test]
    fn build_listener_context_multiple_different_events() {
        let producer = make_test_knot("plan-creator", vec![]);
        let consumer = make_test_knot(
            "consumer",
            vec![
                Intent {
                    target_knot: "plan-creator".to_string(),
                    event_id: "PlanCreated".to_string(),
                    event_description: "When a plan is created.".to_string(),
                },
                Intent {
                    target_knot: "plan-creator".to_string(),
                    event_id: "PlanApproved".to_string(),
                    event_description: "When a plan is approved.".to_string(),
                },
            ],
        );

        let ctx = build_listener_context(&producer, &[consumer.clone()]);

        assert!(ctx.contains("`PlanCreated`"));
        assert!(ctx.contains("`PlanApproved`"));
        // Each event appears exactly once
        assert_eq!(ctx.matches("`PlanCreated`").count(), 1);
        assert_eq!(ctx.matches("`PlanApproved`").count(), 1);
    }

    #[test]
    fn build_listener_context_multiple_consumers_different_events() {
        let producer = make_test_knot("plan-creator", vec![]);
        let consumer_a = make_test_knot(
            "consumer-a",
            vec![Intent {
                target_knot: "plan-creator".to_string(),
                event_id: "PlanCreated".to_string(),
                event_description: "When a plan is created.".to_string(),
            }],
        );
        let consumer_b = make_test_knot(
            "consumer-b",
            vec![Intent {
                target_knot: "plan-creator".to_string(),
                event_id: "PlanApproved".to_string(),
                event_description: "When a plan is approved.".to_string(),
            }],
        );

        let ctx =
            build_listener_context(&producer, &[consumer_a, consumer_b]);

        // Both events should appear
        assert!(ctx.contains("`PlanCreated`"));
        assert!(ctx.contains("`PlanApproved`"));
        assert_eq!(ctx.matches("`PlanCreated`").count(), 1);
        assert_eq!(ctx.matches("`PlanApproved`").count(), 1);
    }

    #[test]
    fn build_listener_context_ignores_other_targets() {
        let producer = make_test_knot("plan-creator", vec![]);
        let other_producer = make_test_knot("other-knot", vec![]);
        let consumer = make_test_knot(
            "consumer",
            vec![Intent {
                target_knot: "other-knot".to_string(),
                event_id: "Something".to_string(),
                event_description: "When something happens".to_string(),
            }],
        );

        let ctx = build_listener_context(
            &producer,
            &[other_producer, consumer.clone()],
        );

        assert!(
            ctx.is_empty(),
            "should be empty when consumer listens to a different knot"
        );
    }

    #[test]
    fn build_listener_context_mixed_targets_filters_correctly() {
        let producer = make_test_knot("plan-creator", vec![]);
        let other = make_test_knot("other-knot", vec![]);
        let consumer = make_test_knot(
            "consumer",
            vec![
                Intent {
                    target_knot: "plan-creator".to_string(),
                    event_id: "PlanCreated".to_string(),
                    event_description: "When a plan is created.".to_string(),
                },
                Intent {
                    target_knot: "other-knot".to_string(),
                    event_id: "Something".to_string(),
                    event_description: "When something happens".to_string(),
                },
            ],
        );

        let ctx =
            build_listener_context(&producer, &[other, consumer.clone()]);

        // Should contain PlanCreated but NOT Something
        assert!(ctx.contains("`PlanCreated`"));
        assert!(
            !ctx.contains("`Something`"),
            "should not include events targeting other knots"
        );
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
            target_knot: "plan-creator".to_string(),
            payload,
        };

        assert_eq!(event.event_id, "PlanCreated");
        assert_eq!(event.target_knot, "plan-creator");
        assert_eq!(event.payload.len(), 2);
        assert_eq!(
            event.payload.get("plan"),
            Some(&"PLAN-001".to_string())
        );
    }

    #[test]
    fn agent_event_serialisation_roundtrip() {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-007".to_string());

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            target_knot: "implementation-planner".to_string(),
            payload,
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, event);
    }

    #[test]
    fn agent_event_empty_payload_defaults() {
        let event = AgentEvent {
            event_id: "Something".to_string(),
            target_knot: "knot-a".to_string(),
            payload: HashMap::new(),
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
        let json = r#"{"event_id":"Test","target_knot":"k1"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_id, "Test");
        assert_eq!(event.target_knot, "k1");
        assert!(event.payload.is_empty());
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
        Knot {
            id: KnotId(id.to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "Test instructions.".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
            listens_for: Vec::new(),
        }
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
