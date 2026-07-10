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
        let content = Self::build_event_file_content(event, &timestamp);

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
    fn build_event_file_content(event: &AgentEvent, timestamp: &str) -> String {
        let mut lines = Vec::new();

        // Frontmatter opening
        lines.push("---".to_string());
        lines.push(format!("event-id: {}", event.event_id));
        lines.push(format!("target-knot: {}", event.target_knot));
        lines.push(format!("timestamp: {}", timestamp));

        // Payload fields into frontmatter
        for (key, value) in &event.payload {
            lines.push(format!("{}: {}", key, value));
        }

        // Frontmatter closing
        lines.push("---".to_string());

        // Markdown body with context
        lines.push(String::new());
        lines.push(format!(
            "## Event: {} from {}",
            event.event_id, event.target_knot
        ));
        lines.push(String::new());

        if event.payload.is_empty() {
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
            target_knot: "plan-creator".to_string(),
            payload,
        }
    }

    fn build_consumer_knot() -> Knot {
        use crate::domain::entities::{KnotId, PromptTemplate};
        use crate::domain::value_objects::StrandSource;
        use std::path::PathBuf;

        Knot {
            id: KnotId("consumer".to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "React to events.".to_string(),
            },
            git_versioned: true,
            strand_source: StrandSource::Filesystem(PathBuf::from("../../tie-offs/review-loom/PlanCreated")),
            event_description: None,
        }
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
        let result = dispatcher.dispatch(&event, &consumer, &loom_id, &rig_dir);

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
        let result = dispatcher.dispatch(&event, &consumer, &loom_id, &rig_dir);

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
            .dispatch(&event, &consumer, &loom_id, &rig_dir)
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
            target_knot: "source-knot".to_string(),
            payload: HashMap::new(),
        };
        let consumer = build_consumer_knot();
        let loom_id = LoomId("consumer-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();
        let path = dispatcher
            .dispatch(&event, &consumer, &loom_id, &rig_dir)
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
            .dispatch(&event, &consumer1, &loom1, &rig_dir)
            .unwrap();
        let path2 = dispatcher
            .dispatch(&event, &consumer2, &loom2, &rig_dir)
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
            target_knot: "plan-creator".to_string(),
            payload: HashMap::new(),
        };
        let event2 = AgentEvent {
            event_id: "PlanApproved".to_string(),
            target_knot: "plan-creator".to_string(),
            payload: HashMap::new(),
        };
        let consumer = build_consumer_knot();
        let loom_id = LoomId("consumer-loom".to_string());

        let dispatcher = FileSystemEventDispatcher::new();

        let path1 = dispatcher
            .dispatch(&event1, &consumer, &loom_id, &rig_dir)
            .unwrap();
        let path2 = dispatcher
            .dispatch(&event2, &consumer, &loom_id, &rig_dir)
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
            .dispatch(&event, &consumer, &loom_id, &rig_dir)
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
}
