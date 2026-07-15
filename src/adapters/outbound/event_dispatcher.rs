//! Filesystem adapter for [`EventDispatcherPort`].
//!
//! Creates event files in the consumer's loom tie-off directory under
//! an `{event-id}/` subdirectory. The consumer knot's `strand-dir` watches
//! this directory (or a subdirectory within it), so new event files trigger
//! the consumer's processing pipeline.

use std::path::Path;

use crate::application::ports::{EventDispatcherPort, PortError};
use crate::domain::entities::{Knot, LoomId};
use crate::domain::events::AgentEvent;

// Re-export shared timestamp helper
use crate::application::usecases::types::format_timestamp;

/// Filesystem implementation of [`EventDispatcherPort`].
///
/// Creates event files at:
/// `rig/tie-offs/{consumer-loom-id}/{event-id}/event-{timestamp}.md`
pub struct FileSystemEventDispatcher;

impl FileSystemEventDispatcher {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FileSystemEventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl EventDispatcherPort for FileSystemEventDispatcher {
    fn dispatch(
        &self,
        event: &AgentEvent,
        _consumer_knot: &Knot,
        producer_knot: &str,
        consumer_loom_id: &LoomId,
        rig_dir: &Path,
    ) -> Result<std::path::PathBuf, PortError> {
        let timestamp = format_timestamp();
        let filename = format!("event-{}.md", timestamp.replace([':', ' '], "-"));

        let event_dir = rig_dir
            .join("tie-offs")
            .join(&consumer_loom_id.0)
            .join(&event.event_id);

        // Create the event subdirectory (and parents) if it doesn't exist
        std::fs::create_dir_all(&event_dir).map_err(|e| {
            PortError::EventDispatchFailed(format!(
                "failed to create event directory '{}': {e}",
                event_dir.display()
            ))
        })?;

        let event_path = event_dir.join(&filename);

        // Build the event file content: YAML frontmatter + markdown body
        let content = Self::build_event_file_content(event, &timestamp, producer_knot);

        std::fs::write(&event_path, &content).map_err(|e| {
            PortError::EventDispatchFailed(format!(
                "failed to write event file '{}': {e}",
                event_path.display()
            ))
        })?;

        Ok(event_path)
    }
}

impl FileSystemEventDispatcher {
    /// Build the content of an event file.
    ///
    /// YAML frontmatter with event payload fields + markdown body with
    /// context about the source knot and event.
    pub(crate) fn build_event_file_content(
        event: &AgentEvent,
        timestamp: &str,
        producer_knot: &str,
    ) -> String {
        let mut lines = Vec::new();

        // Frontmatter opening
        lines.push("---".to_string());
        lines.push(format!("event-id: {}", event.event_id));
        lines.push(format!("target-knot: {}", producer_knot));

        // Timestamp: prefer the agent's timestamp if provided
        // (more semantically meaningful — when the event occurred from
        // the agent's perspective). Fall back to the dispatch system's
        // timestamp.
        let effective_timestamp = event
            .payload
            .get("timestamp")
            .map(|s| s.as_str())
            .unwrap_or(timestamp);
        lines.push(format!("timestamp: {}", effective_timestamp));

        // Payload fields into frontmatter (skip `timestamp` — already
        // written above with preferred value).
        for (key, value) in &event.payload {
            if key == "timestamp" {
                continue;
            }
            lines.push(format!("{}: {}", key, value));
        }

        // Frontmatter closing
        lines.push("---".to_string());

        // Markdown body with context
        lines.push(String::new());
        lines.push(format!(
            "## Event: {} from {}",
            event.event_id, producer_knot
        ));
        lines.push(String::new());

        // Body: event body if present, otherwise fallback
        if let Some(ref body) = event.body {
            lines.push(body.clone());
        } else if event.payload.is_empty() {
            lines.push("No payload data.".to_string());
        } else {
            lines.push("Payload:".to_string());
            lines.push(String::new());
            for (key, value) in &event.payload {
                lines.push(format!("- **{}**: {}", key, value));
            }
        }

        lines.join("\n")
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn build_event() -> AgentEvent {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-001".to_string());
        payload.insert(
            "description".to_string(),
            "Implementation plan".to_string(),
        );

        AgentEvent {
            event_id: "PlanCreated".to_string(),
            payload,
            body: None,
        }
    }

    fn build_consumer_knot() -> Knot {
        use crate::application::usecases::test_fixtures::KnotBuilder;
        use crate::domain::value_objects::StrandSource;
        use std::path::PathBuf;

        KnotBuilder::new("consumer")
            .with_instructions("React to events.")
            .with_strand_source(StrandSource::Filesystem(
                PathBuf::from("../../tie-offs/review-loom/PlanCreated"),
            ))
            .build()
    }

    #[test]
    fn dispatch_creates_event_file_with_correct_path() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let event = build_event();
        let consumer = build_consumer_knot();
        let loom_id = LoomId("consumer-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();
        let result = dispatcher.dispatch(&event, &consumer, "plan-creator", &loom_id, &rig_dir);

        assert!(result.is_ok(), "dispatch should succeed: {:?}", result);
        let path = result.unwrap();

        // Path should be rig/tie-offs/consumer-loom/PlanCreated/event-*.md
        assert!(
            path.starts_with(&rig_dir.join("tie-offs/consumer-loom/PlanCreated")),
            "path should be under correct event directory: {}",
            path.display()
        );
        assert!(
            path.file_name()
                .map(|n| n.to_string_lossy().starts_with("event-"))
                .unwrap_or(false),
            "filename should start with 'event-'",
        );
        assert!(
            path.extension()
                .map(|e| e == "md")
                .unwrap_or(false),
            "extension should be .md",
        );
    }

    #[test]
    fn dispatch_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let event = build_event();
        let consumer = build_consumer_knot();
        let loom_id = LoomId("new-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();
        let result = dispatcher.dispatch(&event, &consumer, "plan-creator", &loom_id, &rig_dir);

        assert!(result.is_ok(), "should create parent dirs: {:?}", result);

        // Verify the full directory chain exists
        let event_dir = rig_dir.join("tie-offs/new-loom/PlanCreated");
        assert!(
            event_dir.is_dir(),
            "event directory should exist: {}",
            event_dir.display()
        );
    }

    #[test]
    fn dispatch_file_content_has_frontmatter_and_body() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let event = build_event();
        let consumer = build_consumer_knot();
        let loom_id = LoomId("consumer-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();
        let path = dispatcher
            .dispatch(&event, &consumer, "plan-creator", &loom_id, &rig_dir)
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();

        // Frontmatter structure
        assert!(
            content.starts_with("---\n"),
            "content should start with frontmatter delimiter"
        );
        assert!(
            content.contains("event-id: PlanCreated"),
            "should contain event-id in frontmatter: {}",
            content
        );
        assert!(
            content.contains("target-knot: plan-creator"),
            "should contain target-knot in frontmatter: {}",
            content
        );
        assert!(
            content.contains("plan: PLAN-001"),
            "should contain payload field in frontmatter: {}",
            content
        );
        assert!(
            content.contains("description: Implementation plan"),
            "should contain description payload: {}",
            content
        );

        // Markdown body
        assert!(
            content.contains("## Event: PlanCreated from plan-creator"),
            "should contain event header in body: {}",
            content
        );
        assert!(
            content.contains("Payload:"),
            "should contain Payload section: {}",
            content
        );
        assert!(
            content.contains("- **plan**: PLAN-001"),
            "should contain payload bullet: {}",
            content
        );
    }

    #[test]
    fn dispatch_file_content_empty_payload() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let event = AgentEvent {
            event_id: "EmptyEvent".to_string(),
            payload: HashMap::new(),
            body: None,
        };
        let consumer = build_consumer_knot();
        let loom_id = LoomId("consumer-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();
        let path = dispatcher
            .dispatch(&event, &consumer, "plan-creator", &loom_id, &rig_dir)
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();

        assert!(
            content.contains("event-id: EmptyEvent"),
            "should contain event-id: {}",
            content
        );
        assert!(
            content.contains("No payload data."),
            "should indicate no payload when empty: {}",
            content
        );
    }

    #[test]
    fn dispatch_fan_out_two_consumers_same_event() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let event = build_event();
        let consumer1 = build_consumer_knot();
        let consumer2 = build_consumer_knot();
        let loom1 = LoomId("loom-alpha".to_string());
        let loom2 = LoomId("loom-beta".to_string());

        let dispatcher = FileSystemEventDispatcher::new();

        let path1 = dispatcher
            .dispatch(&event, &consumer1, "plan-creator", &loom1, &rig_dir)
            .unwrap();
        let path2 = dispatcher
            .dispatch(&event, &consumer2, "plan-creator", &loom2, &rig_dir)
            .unwrap();

        // Each consumer gets its own file in its own loom directory
        assert!(
            path1.parent().unwrap().ends_with("loom-alpha/PlanCreated"),
            "first event should be in loom-alpha: {}",
            path1.display()
        );
        assert!(
            path2.parent().unwrap().ends_with("loom-beta/PlanCreated"),
            "second event should be in loom-beta: {}",
            path2.display()
        );

        // Both files contain the same event data
        let content1 = std::fs::read_to_string(&path1).unwrap();
        let content2 = std::fs::read_to_string(&path2).unwrap();
        assert!(content1.contains("event-id: PlanCreated"));
        assert!(content2.contains("event-id: PlanCreated"));
        assert!(content1.contains("plan: PLAN-001"));
        assert!(content2.contains("plan: PLAN-001"));
    }

    #[test]
    fn dispatch_fan_out_same_loom_different_event_ids() {
        // Two different event types dispatched to the same loom
        // should create files in different subdirectories.
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let event1 = AgentEvent {
            event_id: "PlanCreated".to_string(),
            payload: HashMap::new(),
            body: None,
        };
        let event2 = AgentEvent {
            event_id: "PlanApproved".to_string(),
            payload: HashMap::new(),
            body: None,
        };
        let consumer = build_consumer_knot();
        let loom_id = LoomId("consumer-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();

        let path1 = dispatcher
            .dispatch(&event1, &consumer, "plan-creator", &loom_id, &rig_dir)
            .unwrap();
        let path2 = dispatcher
            .dispatch(&event2, &consumer, "plan-creator", &loom_id, &rig_dir)
            .unwrap();

        // Different event-id subdirectories
        assert!(
            path1.parent().unwrap().ends_with("consumer-loom/PlanCreated"),
            "PlanCreated should be in PlanCreated/ dir"
        );
        assert!(
            path2.parent().unwrap().ends_with("consumer-loom/PlanApproved"),
            "PlanApproved should be in PlanApproved/ dir"
        );
    }

    #[test]
    fn dispatch_timestamp_in_filename_is_sane() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let event = build_event();
        let consumer = build_consumer_knot();
        let loom_id = LoomId("consumer-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();
        let path = dispatcher
            .dispatch(&event, &consumer, "plan-creator", &loom_id, &rig_dir)
            .unwrap();

        let filename = path.file_name().unwrap().to_string_lossy();
        // Filename: event-YYYY-MM-DDTHH-MM-SSZ.md (colons replaced)
        assert!(filename.starts_with("event-"));
        assert!(filename.ends_with(".md"));
        // No colons in filename (replaced with hyphens for filesystem safety)
        assert!(
            !filename.contains(':'),
            "filename should not contain colons: {}",
            filename
        );
    }

    #[test]
    fn build_event_file_content_structure() {
        let event = build_event();
        let timestamp = "2026-07-09T12:00:00Z".to_string();

        let content = FileSystemEventDispatcher::build_event_file_content(
            &event,
            &timestamp,
            "plan-creator",
        );

        let lines: Vec<&str> = content.lines().collect();

        // First line is frontmatter opening
        assert_eq!(lines[0], "---");
        // event-id is present
        assert!(lines.iter().any(|l| *l == "event-id: PlanCreated"));
        // target-knot is present
        assert!(
            lines.iter().any(|l| *l == "target-knot: plan-creator"),
            "target-knot should be in frontmatter"
        );
        // timestamp is present
        assert!(
            lines.iter().any(|l| *l == "timestamp: 2026-07-09T12:00:00Z"),
            "timestamp should be in frontmatter"
        );
        // payload fields are in frontmatter
        assert!(lines.iter().any(|l| *l == "plan: PLAN-001"));
        assert!(
            lines.iter().any(|l| *l == "description: Implementation plan")
        );
        // Body section exists
        assert!(
            lines.iter().any(|l| *l == "## Event: PlanCreated from plan-creator"),
            "body should have event header"
        );
    }

    #[test]
    fn dispatched_file_includes_event_body_in_markdown_content() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-001".to_string());

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            payload,
            body: Some(
                "The plan covers three phases: planning, review, and approval."
                    .to_string(),
            ),
        };
        let consumer = build_consumer_knot();
        let loom_id = LoomId("consumer-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();
        let path = dispatcher
            .dispatch(&event, &consumer, "plan-creator", &loom_id, &rig_dir)
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();

        // Event body appears in the markdown content
        assert!(
            content
                .contains("The plan covers three phases: planning, review, and approval."),
            "dispatched file should contain event body: {}",
            content
        );
        // Fallback content should NOT appear when body is present
        assert!(
            !content.contains("Payload:"),
            "should not show payload fallback when body is present: {}",
            content
        );
        assert!(
            !content.contains("No payload data."),
            "should not show empty payload fallback when body is present: {}",
            content
        );
    }

    #[test]
    fn dispatched_file_without_body_uses_fallback_content() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        // Event with no body but with payload — should show bullet list
        let event = build_event();
        let consumer = build_consumer_knot();
        let loom_id = LoomId("consumer-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();
        let path = dispatcher
            .dispatch(&event, &consumer, "plan-creator", &loom_id, &rig_dir)
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("Payload:"),
            "should show payload fallback when body is None: {}",
            content
        );
        assert!(
            content.contains("- **plan**: PLAN-001"),
            "should list payload bullets when body is None: {}",
            content
        );

        // Event with no body and no payload — should show "No payload data."
        let event_empty = AgentEvent {
            event_id: "EmptyEvent".to_string(),
            payload: HashMap::new(),
            body: None,
        };
        let path2 = dispatcher
            .dispatch(&event_empty, &consumer, "plan-creator", &loom_id, &rig_dir)
            .unwrap();

        let content2 = std::fs::read_to_string(&path2).unwrap();
        assert!(
            content2.contains("No payload data."),
            "should show empty fallback when body is None and payload is empty: {}",
            content2
        );
    }

    /// When the agent provides a `timestamp` in the payload, it is
    /// preferred over the dispatch system's timestamp (more semantically
    /// meaningful — when the event occurred from the agent's perspective).
    #[test]
    fn build_event_file_content_prefers_agent_timestamp() {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-001".to_string());
        payload.insert("description".to_string(), "Implementation plan".to_string());
        payload.insert(
            "timestamp".to_string(),
            "2026-07-14T09:00:00Z".to_string(), // agent's timestamp
        );

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            payload,
            body: None,
        };

        let system_timestamp = "2026-07-14T12:00:00Z".to_string(); // dispatch system's timestamp

        let content = FileSystemEventDispatcher::build_event_file_content(
            &event,
            &system_timestamp,
            "plan-creator",
        );

        // Should use the agent's timestamp
        assert!(
            content.contains("timestamp: 2026-07-14T09:00:00Z"),
            "should use agent's timestamp: {}",
            content
        );

        // Should NOT contain the system timestamp
        assert!(
            !content.contains("timestamp: 2026-07-14T12:00:00Z"),
            "should NOT use system timestamp when agent provides one: {}",
            content
        );

        // `timestamp` should appear only once in frontmatter (not duplicated
        // from payload iteration)
        let timestamp_count = content.matches("timestamp:").count();
        assert_eq!(
            timestamp_count, 1,
            "timestamp should appear exactly once in frontmatter, found {}: {}",
            timestamp_count, content
        );
    }

    /// When no agent timestamp is in the payload, the dispatch system's
    /// timestamp is used as fallback.
    #[test]
    fn build_event_file_content_falls_back_to_system_timestamp() {
        let mut payload = HashMap::new();
        payload.insert("plan".to_string(), "PLAN-001".to_string());
        payload.insert(
            "description".to_string(),
            "Implementation plan".to_string(),
        );
        // No timestamp in payload

        let event = AgentEvent {
            event_id: "PlanCreated".to_string(),
            payload,
            body: None,
        };

        let system_timestamp = "2026-07-14T12:00:00Z".to_string();

        let content = FileSystemEventDispatcher::build_event_file_content(
            &event,
            &system_timestamp,
            "plan-creator",
        );

        // Should use the system timestamp
        assert!(
            content.contains("timestamp: 2026-07-14T12:00:00Z"),
            "should use system timestamp as fallback: {}",
            content
        );

        // timestamp should appear only once
        let timestamp_count = content.matches("timestamp:").count();
        assert_eq!(
            timestamp_count, 1,
            "timestamp should appear exactly once: {}",
            timestamp_count
        );
    }
}
