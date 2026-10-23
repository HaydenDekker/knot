//! Context provider implementations.
//!
//! Concrete implementations of the [`ContextProvider`] trait. The domain
//! defines the interface; the application layer provides implementations
//! that have access to the rig directory and filesystem state.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::domain::entities::Knot;
use crate::domain::events::{
    build_listener_context_with_instructions, BuildContext,
    StrandQueueAccessor,
};
use crate::domain::knot_file::derive_runtime_root;
use crate::domain::pending_event::{
    build_pending_index, event_type_from_path, QueuedEventMeta, QueuedEventType,
};
pub use crate::domain::events::ContextProvider;

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
        // Pull the per-session instruction fresh here (never cached):
        // the content section is re-read on every new session, so user
        // edits to the configured override take effect immediately.
        // `None` (source not wired, or the source found nothing) → the
        // embedded default content section. The hard-coded core (the
        // tie-off contract, event format, rules) is never sourced.
        let tie_off_content_instruction = input
            .instruction_source
            .as_ref()
            .and_then(|s| s.read_tie_off_content_instruction());

        let emission_instructions =
            build_listener_context_with_instructions(
                &input.knot,
                &input.loom_id,
                &input.all_knots,
                input.self_continuation_desc.as_deref(),
                tie_off_content_instruction.as_deref(),
            );

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

        // Append the queued events section after the emission instructions.
        // The agent sees what it can emit first, then what is already queued.
        let pending_section = format_queued_events_section(&pending);
        format!("{}\n\n{}", emission_instructions, pending_section)
    }

    /// Build the producer's queued-events index (plan 097) from the
    /// persistent queue — the authoritative pending set — or, when no
    /// queue reference is wired, from the dispatch directories (capped
    /// fallback, plan 097 phase 0 decision D2).
    ///
    /// Metadata-only: the event type comes from the dispatch path and
    /// `queued_at` from the queue entry; the only file read is the
    /// frontmatter `target-knot` key (producer identity). No event
    /// bodies, no `description`/`timestamp` carry-over.
    fn scan_pending_events(
        &self,
        knot: &Knot,
        emission_instructions: &str,
        rig_dir: &Path,
        strand_queue: Option<Arc<dyn StrandQueueAccessor>>,
    ) -> Vec<QueuedEventType> {
        // Extract event IDs this knot may emit from the emission
        // instructions (the backticked subscriber bullets).
        let event_ids = extract_emitted_event_ids(emission_instructions);
        if event_ids.is_empty() {
            return Vec::new();
        }

        let producer_id = &knot.id.0;

        // Candidate entries: (path, queued_at, event_id, producer).
        //
        // Plan 099: a queue entry that carries a delivery names its own
        // event type and producer, so neither is read from a file. The
        // directory-scan fallback (no queue wired) still has neither and
        // falls back to path derivation + frontmatter, as does a legacy
        // dispatched file still sitting in the queue.
        let candidates: Vec<(std::path::PathBuf, String, Option<String>, Option<String>)> =
            match &strand_queue {
                Some(q) => q
                    .pending_entries()
                    .into_iter()
                    .map(|e| (e.strand_path, e.queued_at, e.event_id, e.producer))
                    .collect(),
                None => Self::scan_dispatch_directory(rig_dir, &event_ids)
                    .into_iter()
                    .map(|p| (p, String::new(), None, None))
                    .collect(),
            };

        // Only read the frontmatter of emittable-type entries the entry
        // itself does not already identify (bounds the I/O); the domain
        // builder re-scopes and applies the cap.
        let mut metas = Vec::new();
        for (path, queued_at, carried_event_id, carried_producer) in candidates {
            let Some(event_id) = carried_event_id
                .clone()
                .or_else(|| event_type_from_path(&path))
            else {
                continue;
            };
            if !event_ids.contains(&event_id) {
                continue;
            }
            let producer = carried_producer.or_else(|| {
                extract_frontmatter(&path)
                    .ok()
                    .and_then(|fm| {
                        frontmatter_get(&fm, "target-knot").map(String::from)
                    })
                    .filter(|p| !p.is_empty())
            });
            metas.push(QueuedEventMeta {
                strand_path: path,
                queued_at,
                producer,
                event_id: carried_event_id,
            });
        }

        build_pending_index(&metas, producer_id, &event_ids)
    }

    /// Scan the dispatch directory for event files matching given
    /// event IDs. Used as a fallback when no queue reference is
    /// available (e.g. tests without a queue).
    fn scan_dispatch_directory(
        rig_dir: &Path,
        event_ids: &HashSet<String>,
    ) -> Vec<std::path::PathBuf> {
        // Dispatch directories live under the rig's runtime root
        // (tie-offs/<rig-basename>/), not inside the rig directory.
        let dispatch_base = derive_runtime_root(rig_dir);
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
    let lines: Vec<&str> = instructions.lines().collect();

    // The subscriber bullets are the contiguous run of
    // `- \`EventId\`` lines directly after the marker line. Scanning only
    // that run (and stopping at the first non-bullet line) keeps the
    // parser safe against later instruction sections that happen to use
    // backticked bullet lists (e.g. the event-content guide).
    let Some(marker) = lines.iter().position(|l| {
        l.contains("The following event/s have been declared by subscribers:")
    }) else {
        return event_ids;
    };
    let mut i = marker + 1;
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    for line in &lines[i..] {
        let trimmed = line.trim();
        // Match lines like: `- \`PlanCreated\` — description`. The first
        // line that is not such a bullet ends the subscriber list.
        let Some(rest) = trimmed.strip_prefix("- `") else {
            break;
        };
        let Some(event_id) = rest.split('`').next() else {
            break;
        };
        if !event_id.is_empty() {
            event_ids.insert(event_id.to_string());
        }
    }

    event_ids
}

// ── Queued Events Section Formatting ─────────────────────────────────

/// Format the queued events section as markdown to append to the agent
/// prompt (plan 097): per emittable event type, the full count plus the
/// newest capped entries. The dedup norm itself lives in the tie-off
/// content instructions (the per-rig instruction surface) — this section
/// only points at it.
fn format_queued_events_section(pending: &[QueuedEventType]) -> String {
    let mut output = String::from(
        "## Queued Events\n\n\
         The following events of the types you may emit are already queued, \
         awaiting consumer invocation. If a queued event already covers the \
         outcome you would emit, do not queue a duplicate — see the \
         event-content norm in your tie-off instructions.\n\n",
    );

    for ty in pending {
        if ty.count > ty.entries.len() {
            output.push_str(&format!(
                "- `{}` — {} queued (newest {} shown):\n",
                ty.event_id,
                ty.count,
                ty.entries.len()
            ));
        } else {
            output.push_str(&format!("- `{}` — {} queued:\n", ty.event_id, ty.count));
        }
        for (path, queued_at) in &ty.entries {
            if queued_at.is_empty() {
                output.push_str(&format!("  - {}\n", path.display()));
            } else {
                output.push_str(&format!("  - {} (queued: {})\n", path.display(), queued_at));
            }
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
            self_continuation_desc: None,
            instruction_source: None,
        }
    }

    /// Test double for the per-session instruction source: fixed
    /// content, `None` to emulate "linked file missing → default".
    #[derive(Debug, Default)]
    struct FixedInstructions {
        content: Option<String>,
    }

    impl crate::domain::events::PromptInstructionSource for FixedInstructions {
        fn read_tie_off_content_instruction(&self) -> Option<String> {
            self.content.clone()
        }
    }

    /// A wired instruction source replaces **only the content section**
    /// in the built context; the hard-coded core stays present. A `None`
    /// source keeps the embedded default content section.
    #[test]
    fn provider_pulls_instructions_from_source_per_session() {
        let provider = AgentEventsContextProvider;
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let mut ctx = make_build_context(
            producer,
            default_loom_id(),
            vec![consumer],
            PathBuf::from("/tmp/nonexistent"),
        );
        ctx.instruction_source = Some(std::sync::Arc::new(FixedInstructions {
            content: Some("SKILL-SOURCED CONTENT TEXT".to_string()),
        }));

        let result = provider.build_context(&ctx);
        assert!(
            result.contains("SKILL-SOURCED CONTENT TEXT"),
            "skill-sourced content text must appear: {}",
            result
        );
        assert!(
            !result.contains("## Tie-Off Content"),
            "the default content section must be replaced: {}",
            result
        );
        assert!(
            !result.contains("## Event Content Guide"),
            "the default event-content guide must be replaced: {}",
            result
        );
        // The hard-coded core is always present — never replaced by an
        // override:
        assert!(
            result.contains("## Tie-Off Contract"),
            "the hard-coded contract must stay: {}",
            result
        );
        assert!(
            result.contains("## Event Format"),
            "the hard-coded event format must stay: {}",
            result
        );
        assert!(
            result.contains("## Rules"),
            "the hard-coded rules must stay: {}",
            result
        );

        // "Pulled per session": a second build with a different source
        // content reflects the new content (nothing is cached).
        ctx.instruction_source = Some(std::sync::Arc::new(FixedInstructions {
            content: Some("UPDATED CONTENT TEXT".to_string()),
        }));
        let again = provider.build_context(&ctx);
        assert!(again.contains("UPDATED CONTENT TEXT"));
        assert!(!again.contains("SKILL-SOURCED CONTENT TEXT"));
        // The hard-coded core survives the content change untouched.
        assert!(again.contains("## Tie-Off Contract"));
        assert!(again.contains("## Event Format"));
        assert!(again.contains("## Rules"));
    }

    /// `extract_emitted_event_ids` parses the subscriber bullets only —
    /// backticked bullets inside the default event-content guide (or a
    /// custom skill file) must not leak in as event IDs.
    #[test]
    fn extract_emitted_event_ids_ignores_guide_bullets() {
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
            PathBuf::from("/tmp/nonexistent"),
        );
        // Default instructions include the event-content guide, whose
        // bullets use backticked field names (`description`,
        // `timestamp`) — none of which are subscriber event IDs.
        let instructions = crate::domain::events::build_listener_context(
            &ctx.knot,
            &ctx.loom_id,
            &ctx.all_knots,
            None,
        );
        let ids = extract_emitted_event_ids(&instructions);
        assert_eq!(ids, std::collections::HashSet::from(["PlanCreated".to_string()]));
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
        // Event files live under the runtime root (not in the rig dir).
        let event_dir = crate::domain::knot_file::derive_runtime_root(rig_dir)
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
            result.contains("# Subscriber Events"),
            "should contain emission instructions: {}",
            result
        );
        assert!(
            !result.contains("## Queued Events"),
            "should NOT contain queued events section: {}",
            result
        );
    }

    /// Pending events exist → includes both emission instructions and
    /// queued events section.
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
            result.contains("## Queued Events"),
            "should contain queued events section: {}",
            result
        );
        assert!(
            result.contains("# Subscriber Events"),
            "should contain emission instructions: {}",
            result
        );
        assert!(
            result.contains("PlanCreated"),
            "should reference the event type: {}",
            result
        );
        // Plan 097: metadata-only — the frontmatter description must NOT
        // leak into the section.
        assert!(
            !result.contains("Implementation plan for feature X"),
            "description must not appear in the section: {}",
            result
        );
    }

    /// Queued events section shows count + path (no description /
    /// timestamp carry-over — plan 097 metadata-only).
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
            !result.contains("My plan description"),
            "description must not appear: {}",
            result
        );
        assert!(
            result.contains("event-2026-07-14T10-00-00Z.md"),
            "should contain the event file path: {}",
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
        // Plan 097: no description carry-over.
        assert!(!result.contains("Plan for feature X"));
        assert!(!result.contains("Validation failed on PRD"));
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
            result.contains("# Subscriber Events"),
            "should still contain emission instructions: {}",
            result
        );
        assert!(
            !result.contains("## Queued Events"),
            "should NOT contain queued events section when no events exist: {}",
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
            result.contains("## Queued Events"),
            "should contain queued events section: {}",
            result
        );
        assert!(
            result.contains("`PlanCreated`"),
            "should contain event-id: {}",
            result
        );
        assert!(
            result.contains("event-2026-07-14T10-00-00Z.md"),
            "should contain the event file path: {}",
            result
        );
    }

    // ── Queue-wired path (plan 097) ────────────────────────────────

    /// Test double for the strand queue accessor: fixed entries
    /// (path + `queued_at`) — the queue-backed source of truth.
    #[derive(Debug, Default)]
    struct FakeQueue {
        entries: Vec<crate::domain::pending_event::QueueEntryView>,
    }

    impl crate::domain::events::StrandQueueAccessor for FakeQueue {
        fn pending_strand_paths(&self) -> Vec<std::path::PathBuf> {
            self.entries
                .iter()
                .map(|e| e.strand_path.clone())
                .collect()
        }

        fn pending_entries(&self) -> Vec<crate::domain::pending_event::QueueEntryView> {
            self.entries.clone()
        }

        fn delete(&self, _id: &crate::domain::pending_event::PendingEventId) -> bool {
            true
        }
    }

    /// One queued event: a real file on disk (frontmatter `target-knot`
    /// = the given producer) + returns the event file path. Callers build
    /// the `QueueEntryView` (with the queue's own `queued_at`).
    fn seed_queued_event(
        rig_dir: &Path,
        consumer_loom: &str,
        event_id: &str,
        target_knot: &str,
        filename: &str,
    ) -> std::path::PathBuf {
        setup_rig_with_events(
            rig_dir,
            consumer_loom,
            event_id,
            target_knot,
            Some("seeded description that must not leak"),
            "2026-07-14T09:00:00Z", // frontmatter ts — must NOT appear
            filename,
        );
        crate::domain::knot_file::derive_runtime_root(rig_dir)
            .join(consumer_loom)
            .join(event_id)
            .join(filename)
    }

    fn make_ctx_with_queue(
        producer: Knot,
        consumer: Knot,
        rig_dir: PathBuf,
        queue: FakeQueue,
    ) -> BuildContext {
        let mut ctx = make_build_context(
            producer,
            default_loom_id(),
            vec![consumer],
            rig_dir,
        );
        ctx.strand_queue = Some(std::sync::Arc::new(queue));
        ctx
    }

    /// Wired queue → the section is built from the queue's entries and
    /// timestamps (not the frontmatter, not the dispatch directory).
    #[test]
    fn queued_events_from_wired_queue_use_queue_timestamps() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let path = seed_queued_event(
            &rig_dir,
            "review-loom",
            "PlanCreated",
            "plan-creator",
            "event-2026-07-14T10-00-00Z.md",
        );
        let queue = FakeQueue {
            entries: vec![crate::domain::pending_event::QueueEntryView {
                strand_path: path.clone(),
                queued_at: "2026-10-18T15:30:00+02:00".to_string(),
                event_id: None,
                producer: None,
            }],
        };
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let ctx = make_ctx_with_queue(producer, consumer, rig_dir.clone(), queue);

        let result = AgentEventsContextProvider.build_context(&ctx);

        assert!(
            result.contains("## Queued Events"),
            "should contain queued events section: {}",
            result
        );
        assert!(
            result.contains("`PlanCreated` — 1 queued"),
            "should show the per-type count: {}",
            result
        );
        assert!(
            result.contains(&path.display().to_string()),
            "should show the queued entry path: {}",
            result
        );
        // `queued_at` comes from the queue entry, not the frontmatter
        // timestamp.
        assert!(
            result.contains("(queued: 2026-10-18T15:30:00+02:00)"),
            "should show the queue's queued_at: {}",
            result
        );
        assert!(
            !result.contains("2026-07-14T09:00:00Z"),
            "frontmatter timestamp must not leak: {}",
            result
        );
        assert!(
            !result.contains("seeded description that must not leak"),
            "frontmatter description must not leak: {}",
            result
        );
        // Plan 097 phase 3: the section points at the dedup norm, which is
        // present in the prompt via the (default) content section — the
        // pointer and its target coexist in the same prompt.
        assert!(
            result.contains("Deduplication"),
            "the dedup norm (rig wording) must be in the same prompt as the section: {}",
            result
        );
    }

    /// A backlog of 50 queued entries of one type must not become 50
    /// prompt entries: count 50, newest 10 shown (the cap).
    #[test]
    fn queued_events_backlog_is_capped_at_newest_ten() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let mut entries = Vec::new();
        for i in 0..50 {
            let filename = format!("event-{i:02}.md");
            let queued_at = format!("2026-10-18T{0:02}:00:00+02:00", i);
            let path = seed_queued_event(
                &rig_dir,
                "review-loom",
                "PlanCreated",
                "plan-creator",
                &filename,
            );
            entries.push(crate::domain::pending_event::QueueEntryView {
                strand_path: path,
                queued_at,
                event_id: None,
                producer: None,
            });
        }
        let queue = FakeQueue { entries };
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let ctx = make_ctx_with_queue(producer, consumer, rig_dir.clone(), queue);

        let result = AgentEventsContextProvider.build_context(&ctx);

        assert!(
            result.contains("`PlanCreated` — 50 queued (newest 10 shown)"),
            "should show full count + capped listing: {}",
            result
        );
        // Newest first: the highest timestamp (49) leads, the oldest (00)
        // is not listed at all.
        assert!(result.contains("event-49.md"), "newest entry must be listed: {}", result);
        assert!(result.contains("event-40.md"), "entry #40 must be listed: {}", result);
        assert!(
            !result.contains("event-39.md"),
            "entries beyond the cap must not be listed: {}",
            result
        );
    }

    /// Queue-wired: entries from other producers are excluded — when the
    /// queue holds nothing FROM this producer of an emittable type, the
    /// section is omitted entirely.
    #[test]
    fn queued_events_omitted_when_only_other_producers_queued() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let path = seed_queued_event(
            &rig_dir,
            "review-loom",
            "PlanCreated",
            "some-other-producer",
            "event-2026-07-14T10-00-00Z.md",
        );
        let queue = FakeQueue {
            entries: vec![crate::domain::pending_event::QueueEntryView {
                strand_path: path,
                queued_at: "2026-10-18T15:30:00+02:00".to_string(),
                event_id: None,
                producer: None,
            }],
        };
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let ctx = make_ctx_with_queue(producer, consumer, rig_dir.clone(), queue);

        let result = AgentEventsContextProvider.build_context(&ctx);

        assert!(
            result.contains("# Subscriber Events"),
            "emission instructions still present: {}",
            result
        );
        assert!(
            !result.contains("## Queued Events"),
            "section must be omitted for other producers' entries: {}",
            result
        );
    }

    /// Queue-wired: entries of non-emittable types are excluded even
    /// when they are this producer's.
    #[test]
    fn queued_events_scoped_to_emittable_types() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        // This producer's own event, but of a type no consumer subscribes
        // to (not in the emittable list).
        let path = seed_queued_event(
            &rig_dir,
            "review-loom",
            "PlanScoped",
            "plan-creator",
            "event-2026-07-14T10-00-00Z.md",
        );
        let queue = FakeQueue {
            entries: vec![crate::domain::pending_event::QueueEntryView {
                strand_path: path,
                queued_at: "2026-10-18T15:30:00+02:00".to_string(),
                event_id: None,
                producer: None,
            }],
        };
        let producer = make_producer_knot("plan-creator");
        let consumer = make_consumer_knot(
            "plan-validator",
            "plan-creator",
            "PlanCreated",
            Some("When a plan is created".to_string()),
        );
        let ctx = make_ctx_with_queue(producer, consumer, rig_dir.clone(), queue);

        let result = AgentEventsContextProvider.build_context(&ctx);

        assert!(
            !result.contains("## Queued Events"),
            "non-emittable types must be scoped out: {}",
            result
        );
    }
}
