//! Context provider implementations.
//!
//! Concrete implementations of the [`ContextProvider`] trait. The domain
//! defines the interface; the application layer provides implementations
//! that have access to the rig directory and filesystem state.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::domain::entities::Knot;
use crate::domain::events::{build_listener_context, BuildContext, ContextProvider, StrandQueueAccessor};

// ── Pending Event Metadata ──────────────────────────────────────────────

/// Metadata extracted from a single dispatched event file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingEvent {
    /// The event type (e.g. `PlanCreated`).
    event_id: String,
    /// Short description from the frontmatter `description` field.
    description: Option<String>,
    /// Filename of the dispatched event (e.g. `event-2026-07-14T10-00-00Z.md`).
    filename: String,
    /// ISO 8601 timestamp from the frontmatter.
    timestamp: Option<String>,
}

// ── AgentEventsContextProvider ──────────────────────────────────────────

/// Context provider that injects agent event instructions and pending
/// event visibility into a producer knot's prompt.
///
/// Combines two concerns:
///
/// 1. **Event emission instructions** — what events the knot may emit and
///    the required format (delegates to `build_listener_context`).
/// 2. **Pending event visibility** — scans the event dispatch directories
///    for files already emitted by this producer, giving the agent
///    visibility to avoid emitting duplicative events.
///
/// Stateless — uses [`BuildContext`] for all data.
#[derive(Debug, Clone, Default)]
pub struct AgentEventsContextProvider;

impl AgentEventsContextProvider {
    /// Build the full context: emission instructions + pending events section.
    ///
    /// When no listeners exist, returns an empty string. When listeners
    /// exist but no pending events are found, returns only the emission
    /// instructions.
    fn build_full_context(&self, input: &BuildContext) -> String {
        let emission_instructions =
            build_listener_context(&input.knot, &input.loom_id, &input.all_knots);

        // No listeners — no injection needed at all.
        if emission_instructions.is_empty() {
            return String::new();
        }

        let pending = self.scan_pending_events(
            &input.knot,
            &emission_instructions,
            &input.rig_dir,
            input.strand_queue.clone(),
        );

        if pending.is_empty() {
            return emission_instructions;
        }

        // Prepend pending events section before emission instructions.
        let pending_section = format_pending_events_section(&pending);
        format!("{}\n\n{}", pending_section, emission_instructions)
    }

    /// Scan the in-memory strand queue for pending events emitted by
    /// the current producer knot.
    ///
    /// Queries the strand event queue (source of truth) for files
    /// currently waiting to be processed. Only events dispatched from
    /// this producer and matching an event ID the knot may emit are
    /// included.
    fn scan_pending_events(
        &self,
        knot: &Knot,
        emission_instructions: &str,
        rig_dir: &Path,
        strand_queue: Option<Arc<dyn StrandQueueAccessor>>,
    ) -> Vec<PendingEvent> {
        // Extract event IDs this knot may emit from the emission
        // instructions. They appear as bullet points: `- \`EventId\` — ...`
        let event_ids = extract_emitted_event_ids(emission_instructions);
        if event_ids.is_empty() {
            return Vec::new();
        }

        let producer_id = &knot.id.0;

        // Get pending strand paths from the in-memory queue.
        // If no queue reference is available, fall back to filesystem scan.
        let pending_paths = strand_queue
            .as_ref()
            .map(|q| q.pending_strand_paths())
            .unwrap_or_else(|| {
                Self::scan_dispatch_directory(rig_dir, &event_ids)
            });

        let mut pending = Vec::new();

        for strand_path in pending_paths {
            // Only consider event files (start with "event-")
            let filename = strand_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string());
            let Some(ref name) = filename else {
                continue;
            };
            if !name.starts_with("event-") {
                continue;
            }

            // Only include events from a matching event ID directory
            let parent_dir_name = strand_path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|s| s.to_string());
            let Some(ref event_dir_name) = parent_dir_name else {
                continue;
            };
            if !event_ids.contains(event_dir_name) {
                continue;
            }

            // Read frontmatter to verify producer and extract metadata
            let Ok(frontmatter) = extract_frontmatter(&strand_path) else {
                continue;
            };

            // Only include events dispatched FROM this producer.
            let target_knot = frontmatter_get(&frontmatter, "target-knot");
            if target_knot != Some(producer_id.as_str()) {
                continue;
            }

            let description =
                frontmatter_get(&frontmatter, "description").map(String::from);
            let timestamp =
                frontmatter_get(&frontmatter, "timestamp").map(String::from);

            pending.push(PendingEvent {
                event_id: event_dir_name.clone(),
                description,
                filename: filename.unwrap_or_default(),
                timestamp,
            });
        }

        pending
    }

    /// Scan the dispatch directory for event files matching given
    /// event IDs. Used as a fallback when no queue reference is
    /// available (e.g. tests without a queue).
    fn scan_dispatch_directory(
        rig_dir: &Path,
        event_ids: &HashSet<String>,
    ) -> Vec<std::path::PathBuf> {
        let dispatch_base = rig_dir.join("tie-offs");
        let mut paths = Vec::new();

        let Ok(looms) = std::fs::read_dir(&dispatch_base) else {
            return paths;
        };

        for loom_entry in looms.filter_map(|e| e.ok()) {
            let loom_path = loom_entry.path();
            if !loom_path.is_dir() {
                continue;
            }

            for event_id in event_ids {
                let event_dir = loom_path.join(event_id);
                if !event_dir.is_dir() {
                    continue;
                }

                let Ok(entries) = std::fs::read_dir(&event_dir) else {
                    continue;
                };

                for file_entry in entries.filter_map(|e| e.ok()) {
                    let file_path = file_entry.path();
                    if file_path.is_file() {
                        if file_path
                            .extension()
                            .and_then(|e| e.to_str())
                            == Some("md")
                        {
                            paths.push(file_path);
                        }
                    }
                }
            }
        }

        paths
    }
}

impl ContextProvider for AgentEventsContextProvider {
    fn build_context(&self, input: &BuildContext) -> String {
        self.build_full_context(input)
    }
}

// ── Frontmatter Parsing Helpers ────────────────────────────────────────

/// Extract the YAML-style frontmatter from an event file as a map of
/// key-value pairs.
///
/// Frontmatter is the text between the opening and closing `---` lines
/// at the top of the file. Each line is `key: value`.
fn extract_frontmatter(path: &Path) -> Result<Vec<(String, String)>, std::io::Error> {
    let content = std::fs::read_to_string(path)?;
    let mut result = Vec::new();
    let mut in_frontmatter = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_frontmatter {
                break; // closing delimiter
            }
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter {
            if let Some((key, value)) = line.split_once(':') {
                result.push((
                    key.trim().to_string(),
                    value.trim().to_string(),
                ));
            }
        }
    }

    Ok(result)
}

/// Get a value from frontmatter by key.
fn frontmatter_get<'a>(
    frontmatter: &'a [(String, String)],
    key: &str,
) -> Option<&'a str> {
    frontmatter
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

// ── Event ID Extraction ────────────────────────────────────────────────

/// Extract the event IDs a knot may emit from the emission instructions
/// text.
///
/// Event IDs appear as bullet points in the format: `- \`EventId\` — ...`
fn extract_emitted_event_ids(instructions: &str) -> HashSet<String> {
    let mut event_ids = HashSet::new();

    for line in instructions.lines() {
        let trimmed = line.trim();
        // Match lines like: `- \`PlanCreated\` — description`
        if let Some(rest) = trimmed.strip_prefix("- `") {
            if let Some(event_id) = rest.split('`').next() {
                if !event_id.is_empty() {
                    event_ids.insert(event_id.to_string());
                }
            }
        }
    }

    event_ids
}

// ── Pending Events Section Formatting ──────────────────────────────────

/// Format the pending events section as markdown to prepend to the
/// agent prompt.
fn format_pending_events_section(pending: &[PendingEvent]) -> String {
    let mut output = String::from(
        "## Pending Events\n\n\
         The following events have been emitted but may not yet have triggered their\n\
         consumers. Check these before deciding to emit a new event — if a pending event\n\
         covers the same outcome, you may not need to emit a duplicate.\n",
    );

    for event in pending {
        let desc = event.description.as_deref().unwrap_or("");
        if desc.is_empty() {
            output.push_str(&format!("- `{}` (file: {})\n", event.event_id, event.filename));
        } else {
            output.push_str(&format!(
                "- `{}` — \"{}\" (file: {})\n",
                event.event_id, desc, event.filename,
            ));
        }
    }

    output
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::usecases::test_fixtures::KnotBuilder;
    use crate::domain::entities::LoomId;
    use crate::domain::value_objects::StrandSource;
    use std::path::PathBuf;

    fn default_loom_id() -> LoomId {
        LoomId("test-loom".to_string())
    }

    fn make_producer_knot(id: &str) -> Knot {
        KnotBuilder::new(id).with_instructions("test").build()
    }

    fn make_consumer_knot(
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

    fn make_build_context(
        knot: Knot,
        loom_id: LoomId,
        all_knots: Vec<Knot>,
        rig_dir: PathBuf,
    ) -> BuildContext {
        BuildContext {
            knot,
            loom_id,
            all_knots,
            rig_dir,
            strand_queue: None,
        }
    }

    // Helper: create a temp rig dir with dispatched event files.
    // Returns the temp dir (kept alive via returned guard) and the path to rig.
    fn setup_rig_with_events(
        rig_dir: &Path,
        consumer_loom: &str,
        event_id: &str,
        target_knot: &str,
        description: Option<&str>,
        timestamp: &str,
        filename: &str,
    ) {
        let event_dir = rig_dir
            .join("tie-offs")
            .join(consumer_loom)
            .join(event_id);
        std::fs::create_dir_all(&event_dir).unwrap();

        let mut frontmatter_lines = vec![
            "---".to_string(),
            format!("event-id: {}", event_id),
            format!("target-knot: {}", target_knot),
            format!("timestamp: {}", timestamp),
        ];
        if let Some(desc) = description {
            frontmatter_lines.push(format!("description: {}", desc));
        }
        frontmatter_lines.push("---".to_string());
        frontmatter_lines.push(String::new());
        frontmatter_lines.push(format!("## Event: {} from {}", event_id, target_knot));

        let content = frontmatter_lines.join("\n");
        std::fs::write(event_dir.join(filename), content).unwrap();
    }

    /// No listeners → returns empty string.
    #[test]
    fn no_listeners_returns_empty() {
        let provider = AgentEventsContextProvider;
        let knot = make_producer_knot("plan-creator");
        let ctx = make_build_context(
            knot,
            default_loom_id(),
            vec![], // no other knots
            PathBuf::from("/tmp/rig"),
        );

        let result = provider.build_context(&ctx);
        assert!(
            result.is_empty(),
            "no listeners should produce empty context: '{}'",
            result
        );
    }

    /// Listeners exist but no pending events → returns only emission
    /// instructions (no pending events section).
    #[test]
    fn listeners_no_pending_events_returns_only_instructions() {
        let provider = AgentEventsContextProvider;
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let ctx = make_build_context(
            producer,
            default_loom_id(),
            vec![consumer],
            PathBuf::from("/tmp/nonexistent"), // no rig dir
        );

        let result = provider.build_context(&ctx);
        assert!(
            result.contains("## Agent Events"),
            "should contain emission instructions: {}",
            result
        );
        assert!(
            !result.contains("## Pending Events"),
            "should NOT contain pending events section: {}",
            result
        );
    }

    /// Pending events exist → includes both emission instructions and
    /// pending events section.
    #[test]
    fn pending_events_includes_both_sections() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        // Create a dispatched event file
        setup_rig_with_events(
            &rig_dir,
            "review-loom",
            "PlanCreated",
            "plan-creator",
            Some("Implementation plan for feature X"),
            "2026-07-14T10:00:00Z",
            "event-2026-07-14T10-00-00Z.md",
        );

        let provider = AgentEventsContextProvider;
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let ctx = make_build_context(
            producer,
            default_loom_id(),
            vec![consumer],
            rig_dir.clone(),
        );

        let result = provider.build_context(&ctx);

        assert!(
            result.contains("## Pending Events"),
            "should contain pending events section: {}",
            result
        );
        assert!(
            result.contains("## Agent Events"),
            "should contain emission instructions: {}",
            result
        );
        assert!(
            result.contains("PlanCreated"),
            "should reference the event type: {}",
            result
        );
        assert!(
            result.contains("Implementation plan for feature X"),
            "should contain the description: {}",
            result
        );
    }

    /// Pending events section correctly extracts event-id, description,
    /// and timestamp from dispatched event file frontmatter.
    #[test]
    fn pending_events_section_correctly_extracts_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        setup_rig_with_events(
            &rig_dir,
            "review-loom",
            "PlanCreated",
            "plan-creator",
            Some("My plan description"),
            "2026-07-14T10:00:00Z",
            "event-2026-07-14T10-00-00Z.md",
        );

        let provider = AgentEventsContextProvider;
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let ctx = make_build_context(
            producer,
            default_loom_id(),
            vec![consumer],
            rig_dir.clone(),
        );

        let result = provider.build_context(&ctx);

        assert!(
            result.contains("`PlanCreated`"),
            "should contain event-id: {}",
            result
        );
        assert!(
            result.contains("My plan description"),
            "should contain description: {}",
            result
        );
        assert!(
            result.contains("event-2026-07-14T10-00-00Z.md"),
            "should contain filename: {}",
            result
        );
    }

    /// Multiple event types, each with their own pending events.
    #[test]
    fn multiple_event_types_with_pending_events() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        // PlanCreated pending event
        setup_rig_with_events(
            &rig_dir,
            "review-loom",
            "PlanCreated",
            "plan-creator",
            Some("Plan for feature X"),
            "2026-07-14T10:00:00Z",
            "event-2026-07-14T10-00-00Z.md",
        );

        // ValidationFailed pending event
        setup_rig_with_events(
            &rig_dir,
            "review-loom",
            "ValidationFailed",
            "plan-creator",
            Some("Validation failed on PRD"),
            "2026-07-14T11:00:00Z",
            "event-2026-07-14T11-00-00Z.md",
        );

        let provider = AgentEventsContextProvider;
        let producer = make_producer_knot("plan-creator");
        let consumer1 = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let consumer2 = make_consumer_knot(
            "plan-fixer",
            "plan-creator",
            "ValidationFailed",
            Some("When validation fails".to_string()),
        );
        let ctx = make_build_context(
            producer,
            default_loom_id(),
            vec![consumer1, consumer2],
            rig_dir.clone(),
        );

        let result = provider.build_context(&ctx);

        assert!(
            result.contains("PlanCreated"),
            "should contain PlanCreated: {}",
            result
        );
        assert!(
            result.contains("ValidationFailed"),
            "should contain ValidationFailed: {}",
            result
        );
        assert!(
            result.contains("Plan for feature X"),
            "should contain PlanCreated description: {}",
            result
        );
        assert!(
            result.contains("Validation failed on PRD"),
            "should contain ValidationFailed description: {}",
            result
        );
    }

    /// Missing dispatch directory (no events emitted yet) → graceful
    /// empty (no crash, returns only emission instructions).
    #[test]
    fn missing_dispatch_directory_graceful() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();
        // Do NOT create any tie-offs directory

        let provider = AgentEventsContextProvider;
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let ctx = make_build_context(
            producer,
            default_loom_id(),
            vec![consumer],
            rig_dir.clone(),
        );

        let result = provider.build_context(&ctx);

        assert!(
            result.contains("## Agent Events"),
            "should still contain emission instructions: {}",
            result
        );
        assert!(
            !result.contains("## Pending Events"),
            "should NOT contain pending events section when no events exist: {}",
            result
        );
    }

    /// Event file with no description in frontmatter → shown without
    /// description (just event-id and filename).
    #[test]
    fn event_file_with_no_description() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        // Event file WITHOUT description field
        setup_rig_with_events(
            &rig_dir,
            "review-loom",
            "PlanCreated",
            "plan-creator",
            None, // no description
            "2026-07-14T10:00:00Z",
            "event-2026-07-14T10-00-00Z.md",
        );

        let provider = AgentEventsContextProvider;
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let ctx = make_build_context(
            producer,
            default_loom_id(),
            vec![consumer],
            rig_dir.clone(),
        );

        let result = provider.build_context(&ctx);

        assert!(
            result.contains("## Pending Events"),
            "should contain pending events section: {}",
            result
        );
        assert!(
            result.contains("`PlanCreated`"),
            "should contain event-id: {}",
            result
        );
        assert!(
            result.contains("event-2026-07-14T10-00-00Z.md"),
            "should contain filename: {}",
            result
        );
    }
}
