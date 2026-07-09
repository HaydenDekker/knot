use regex::Regex;

// ── Domain Types ───────────────────────────────────────────────────────────

/// A single parsed section from a tie-off file.
///
/// Represents one processing event: knot name, event type, strand path,
/// timestamp, and the agent's response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TieOffSection {
    pub knot_name: String,
    pub event_type: String,
    pub strand_path: String,
    pub timestamp: String,
    pub body: String,
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Header line regex for tie-off sections.
///
/// Format: `## {knot_name} triggered by {event_type} {strand_path}`
/// The knot name, event type, and strand path are separated by whitespace
/// with the literal `triggered by` in between.
const HEADER_RE: &str = r"^## (?P<knot_name>[^\s]+)\s+triggered by\s+(?P<event_type>[^\s]+)\s+(?P<strand_path>.+)$";

/// Parse a tie-off file's content into structured sections.
///
/// The tie-off format uses `---` as section delimiters. Each section has:
/// - A header line: `## {knot_name} triggered by {event_type} {strand_path}`
/// - A timestamp line: `Timestamp: {iso8601}`
/// - A `---` delimiter
/// - The agent's response body
///
/// Sections without a valid header line are skipped gracefully.
pub fn parse_sections(content: &str) -> Vec<TieOffSection> {
    let header_re = Regex::new(HEADER_RE).unwrap();
    let mut sections: Vec<TieOffSection> = Vec::new();
    let mut current: Option<TieOffSection> = None;
    let mut body_lines: Vec<String> = Vec::new();

    for line in content.lines() {
        // Check for new section header (takes priority over state)
        if line.starts_with("## ") {
            // Finalize any previous section
            if let Some(mut section) = current.take() {
                section.body = body_lines.join("\n");
                sections.push(section);
            }
            body_lines.clear();

            // Parse header fields
            if let Some(captures) = header_re.captures(line) {
                let section = TieOffSection {
                    knot_name: captures
                        .name("knot_name")
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default(),
                    event_type: captures
                        .name("event_type")
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default(),
                    strand_path: captures
                        .name("strand_path")
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default(),
                    timestamp: String::new(),
                    body: String::new(),
                };
                current = Some(section);
            }
            // If no capture match, the header is malformed — skip this section
            // by leaving current as None; body_lines is already cleared.
            continue;
        }

        if current.is_some() {
            // Parse timestamp line
            if let Some(ts) = line.strip_prefix("Timestamp: ") {
                if let Some(ref mut section) = current {
                    section.timestamp = ts.to_string();
                }
                continue;
            }

            // Check for section delimiter (---)
            if line.trim() == "---" {
                continue;
            }

            // Body content
            body_lines.push(line.to_string());
        }
    }

    // Finalize the last section
    if let Some(mut section) = current.take() {
        section.body = body_lines.join("\n");
        sections.push(section);
    }

    sections
}

/// Extract the last N tie-off sections for a specific strand.
///
/// Parses the content, filters sections matching `strand_path`, and returns
/// at most `n` entries from the end. Returns an empty vec if no matches
/// are found.
pub fn extract_last_n(
    content: &str,
    strand_path: &str,
    n: usize,
) -> Vec<TieOffSection> {
    let all_sections = parse_sections(content);
    let matching: Vec<&TieOffSection> = all_sections
        .iter()
        .filter(|s| s.strand_path == strand_path)
        .collect();
    let start = if matching.len() > n {
        matching.len() - n
    } else {
        0
    };
    matching[start..].iter().map(|s| (*s).clone()).collect()
}

/// Extract structured agent events from tie-off content.
///
/// Scans the tie-off body for indented key-value blocks that contain an
/// `event:` key. Each such block represents one `AgentEvent` emitted by a
/// target knot.
///
/// ## Format
///
/// The structured event block appears in the body of a tie-off section.
/// Lines are indented with whitespace. The first key must be `event:` (the
/// event identifier). A `target-knot:` key identifies the emitting knot.
/// All other keys become the payload.
///
/// ```text
/// ## review triggered by Created input.md
/// Timestamp: 2026-06-25T10:00:00Z
/// ---
/// Plan PLAN-001 created.
///
///   event: PlanCreated
///   target-knot: plan-creator
///   plan: PLAN-001
///   description: Implementation plan for knot event routing
///
/// Some follow-up text.
/// ```
///
/// ## Graceful handling
///
/// - Blocks without `event:` are skipped (just indented metadata).
/// - Blocks with `event:` but no `target-knot:` get an empty target-knot.
/// - Malformed lines (no `:` separator) are skipped.
/// - Non-indented lines are skipped.
pub fn extract_agent_events(
    content: &str,
) -> Vec<crate::domain::events::AgentEvent> {
    let mut events: Vec<crate::domain::events::AgentEvent> = Vec::new();
    let mut current_event_id: Option<String> = None;
    let mut current_target_knot: Option<String> = None;
    let mut current_payload: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut in_block = false;

    for line in content.lines() {
        // Check if this is an indented line (part of a key-value block)
        let trimmed = line.trim();

        if !trimmed.is_empty() && line.starts_with(|c: char| c == ' ' || c == '\t') {
            // Indented line — try to parse as key: value
            if let Some((key, value)) = parse_kv_line(trimmed) {
                if key == "event" {
                    // Starting a new event block (or continuing with event as first key)
                    // If we were already in a block, flush it first
                    if in_block {
                        events.push(crate::domain::events::AgentEvent {
                            event_id: current_event_id.take().unwrap_or_default(),
                            target_knot: current_target_knot.take().unwrap_or_default(),
                            payload: std::mem::take(&mut current_payload),
                        });
                    }
                    current_event_id = Some(value);
                    in_block = true;
                } else if in_block {
                    // Continuation of current event block
                    if key == "target-knot" {
                        current_target_knot = Some(value);
                    } else {
                        current_payload.insert(key, value);
                    }
                }
                // If not in_block and not an event key, just skip
            } else {
                // Not a valid key-value line (no `:` separator) — still indented
                // but not metadata. If we're in a block, this malformed line
                // is just skipped gracefully (block continues).
            }
        } else {
            // Non-indented line — ends any current block
            if in_block {
                events.push(crate::domain::events::AgentEvent {
                    event_id: current_event_id.take().unwrap_or_default(),
                    target_knot: current_target_knot.take().unwrap_or_default(),
                    payload: std::mem::take(&mut current_payload),
                });
                in_block = false;
                current_target_knot = None;
            }
        }
    }

    // Flush any remaining block at end of content
    if in_block {
        events.push(crate::domain::events::AgentEvent {
            event_id: current_event_id.take().unwrap_or_default(),
            target_knot: current_target_knot.take().unwrap_or_default(),
            payload: std::mem::take(&mut current_payload),
        });
    }

    events
}

/// Parse a single indented line as `key: value`.
///
/// Returns `(key, value)` if a `:` separator is found, or `None` if the
/// line is not a valid key-value pair.
fn parse_kv_line(line: &str) -> Option<(String, String)> {
    let colon_pos = line.find(':')?;
    let key = line[..colon_pos].trim().to_string();
    let value = line[colon_pos + 1..].trim().to_string();
    if key.is_empty() {
        None
    } else {
        Some((key, value))
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sections_empty_input() {
        let sections = parse_sections("");
        assert!(sections.is_empty());
    }

    #[test]
    fn parse_sections_single_section() {
        let content =
            "## review triggered by Created docs.md\nTimestamp: 2026-06-01T00:00:00Z\n---\nBody text";
        let sections = parse_sections(content);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].knot_name, "review");
        assert_eq!(sections[0].event_type, "Created");
        assert_eq!(sections[0].strand_path, "docs.md");
        assert_eq!(sections[0].timestamp, "2026-06-01T00:00:00Z");
        assert_eq!(sections[0].body, "Body text");
    }

    #[test]
    fn parse_sections_multiple_sections() {
        let content = concat!(
            "## review triggered by Created a.md\n",
            "Timestamp: 2026-06-01T00:00:00Z\n",
            "---\n",
            "Body one\n",
            "---\n",
            "## review triggered by Modified b.md\n",
            "Timestamp: 2026-06-02T00:00:00Z\n",
            "---\n",
            "Body two\n",
            "---\n",
            "## review triggered by Deleted c.md\n",
            "Timestamp: 2026-06-03T00:00:00Z\n",
            "---\n",
            "Body three",
        );
        let sections = parse_sections(content);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].strand_path, "a.md");
        assert_eq!(sections[0].body, "Body one");
        assert_eq!(sections[1].strand_path, "b.md");
        assert_eq!(sections[1].body, "Body two");
        assert_eq!(sections[2].strand_path, "c.md");
        assert_eq!(sections[2].body, "Body three");
    }

    #[test]
    fn parse_sections_preserves_body_newlines() {
        let content = concat!(
            "## review triggered by Created file.md\n",
            "Timestamp: 2026-06-01T00:00:00Z\n",
            "---\n",
            "Line one\n",
            "Line two\n",
            "Line three",
        );
        let sections = parse_sections(content);
        assert_eq!(sections.len(), 1);
        assert_eq!(
            sections[0].body,
            "Line one\nLine two\nLine three"
        );
    }

    #[test]
    fn extract_last_n_filters_by_strand() {
        let content = concat!(
            "## review triggered by Created alpha.md\n",
            "Timestamp: 2026-06-01T00:00:00Z\n",
            "---\n",
            "Alpha body\n",
            "---\n",
            "## review triggered by Created beta.md\n",
            "Timestamp: 2026-06-02T00:00:00Z\n",
            "---\n",
            "Beta body\n",
            "---\n",
            "## review triggered by Modified alpha.md\n",
            "Timestamp: 2026-06-03T00:00:00Z\n",
            "---\n",
            "Alpha updated\n",
            "---\n",
            "## review triggered by Created gamma.md\n",
            "Timestamp: 2026-06-04T00:00:00Z\n",
            "---\n",
            "Gamma body",
        );
        let result = extract_last_n(content, "alpha.md", 10);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].event_type, "Created");
        assert_eq!(result[0].body, "Alpha body");
        assert_eq!(result[1].event_type, "Modified");
        assert_eq!(result[1].body, "Alpha updated");
    }

    #[test]
    fn extract_last_n_limits_to_n() {
        let mut content_parts: Vec<String> = Vec::new();
        for i in 1..=7 {
            content_parts.push(format!(
                "## review triggered by Created strand.md\nTimestamp: 2026-06-{0:02}T00:00:00Z\n---\nBody {0}",
                i
            ));
        }
        let content = content_parts.join("\n---\n");
        let result = extract_last_n(&content, "strand.md", 3);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].body, "Body 5");
        assert_eq!(result[1].body, "Body 6");
        assert_eq!(result[2].body, "Body 7");
    }

    #[test]
    fn extract_last_n_less_than_n() {
        let content = concat!(
            "## review triggered by Created strand.md\n",
            "Timestamp: 2026-06-01T00:00:00Z\n",
            "---\n",
            "Only entry",
        );
        let result = extract_last_n(content, "strand.md", 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].body, "Only entry");
    }

    #[test]
    fn extract_last_n_no_matches() {
        let content = concat!(
            "## review triggered by Created other.md\n",
            "Timestamp: 2026-06-01T00:00:00Z\n",
            "---\n",
            "Some body",
        );
        let result = extract_last_n(content, "missing.md", 5);
        assert!(result.is_empty());
    }

    #[test]
    fn parse_sections_malformed_header() {
        // Section with no valid header line is skipped
        let content = concat!(
            "## review triggered by Created valid.md\n",
            "Timestamp: 2026-06-01T00:00:00Z\n",
            "---\n",
            "Valid body\n",
            "---\n",
            "## bad header no keyword\n",
            "Timestamp: 2026-06-02T00:00:00Z\n",
            "---\n",
            "Orphan body\n",
            "---\n",
            "## review triggered by Modified valid.md\n",
            "Timestamp: 2026-06-03T00:00:00Z\n",
            "---\n",
            "Updated body",
        );
        let sections = parse_sections(content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].strand_path, "valid.md");
        assert_eq!(sections[0].body, "Valid body");
        assert_eq!(sections[1].strand_path, "valid.md");
        assert_eq!(sections[1].body, "Updated body");
    }

    // ── extract_agent_events Tests ────────────────────────────────

    #[test]
    fn extract_agent_events_empty_input() {
        let events = extract_agent_events("");
        assert!(events.is_empty());
    }

    #[test]
    fn extract_agent_events_no_events() {
        let content = concat!(
            "## review triggered by Created file.md\n",
            "Timestamp: 2026-06-01T00:00:00Z\n",
            "---\n",
            "Normal body text without any events.",
        );
        let events = extract_agent_events(content);
        assert!(events.is_empty());
    }

    #[test]
    fn extract_agent_events_single_event() {
        let content = concat!(
            "Plan PLAN-001 created.\n",
            "\n",
            "  event: PlanCreated\n",
            "  target-knot: plan-creator\n",
            "  plan: PLAN-001\n",
            "  description: Implementation plan for knot event routing\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert_eq!(events[0].target_knot, "plan-creator");
        assert_eq!(
            events[0].payload.get("plan"),
            Some(&"PLAN-001".to_string())
        );
        assert_eq!(
            events[0].payload.get("description"),
            Some(&"Implementation plan for knot event routing".to_string())
        );
    }

    #[test]
    fn extract_agent_events_with_surrounding_text() {
        let content = concat!(
            "Here is some analysis of the PRD.\n",
            "\n",
            "The goals look solid.\n",
            "\n",
            "  event: GoalsApproved\n",
            "  target-knot: prd-reviewer\n",
            "  prd: PRD-042\n",
            "\n",
            "Additional notes follow.",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "GoalsApproved");
        assert_eq!(events[0].target_knot, "prd-reviewer");
        assert_eq!(events[0].payload.get("prd"), Some(&"PRD-042".to_string()));
    }

    #[test]
    fn extract_agent_events_multiple_events() {
        let content = concat!(
            "First event fired.\n",
            "  event: PlanCreated\n",
            "  target-knot: plan-creator\n",
            "  plan: PLAN-001\n",
            "\n",
            "Some text between events.\n",
            "\n",
            "Second event fired.\n",
            "  event: PlanApproved\n",
            "  target-knot: plan-creator\n",
            "  plan: PLAN-001\n",
            "  approver: lead-dev\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert_eq!(events[0].target_knot, "plan-creator");
        assert_eq!(events[1].event_id, "PlanApproved");
        assert_eq!(events[1].target_knot, "plan-creator");
        assert_eq!(
            events[1].payload.get("approver"),
            Some(&"lead-dev".to_string())
        );
    }

    #[test]
    fn extract_agent_events_block_without_event_key_skipped() {
        // Indented key-value block without `event:` is not an agent event.
        let content = concat!(
            "Some analysis:\n",
            "  target-knot: plan-creator\n",
            "  plan: PLAN-001\n",
        );
        let events = extract_agent_events(content);
        assert!(events.is_empty());
    }

    #[test]
    fn extract_agent_events_event_without_target_knot() {
        // `target-knot` is optional — defaults to empty string.
        let content = concat!(
            "  event: SomethingHappened\n",
            "  detail: info\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "SomethingHappened");
        assert_eq!(events[0].target_knot, "");
        assert_eq!(
            events[0].payload.get("detail"),
            Some(&"info".to_string())
        );
    }

    #[test]
    fn extract_agent_events_malformed_lines_skipped_gracefully() {
        // Lines without `:` separator inside a block are skipped.
        let content = concat!(
            "  event: DataProcessed\n",
            "  target-knot: data-pipeline\n",
            "  this line has no colon\n",
            "  record-count: 42\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "DataProcessed");
        assert_eq!(events[0].target_knot, "data-pipeline");
        assert_eq!(
            events[0].payload.get("record-count"),
            Some(&"42".to_string())
        );
    }

    #[test]
    fn extract_agent_events_values_with_colons() {
        // Values can contain colons — only the first colon splits.
        let content = concat!(
            "  event: FileProcessed\n",
            "  target-knot: file-handler\n",
            "  path: /home/user/file.md\n",
            "  timestamp: 2026-06-25T10:00:00Z\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "FileProcessed");
        assert_eq!(
            events[0].payload.get("path"),
            Some(&"/home/user/file.md".to_string())
        );
        assert_eq!(
            events[0].payload.get("timestamp"),
            Some(&"2026-06-25T10:00:00Z".to_string())
        );
    }

    #[test]
    fn extract_agent_events_non_indented_kv_ignored() {
        // Non-indented key-value lines (like header Timestamp:) are not
        // part of event blocks.
        let content = concat!(
            "Timestamp: 2026-06-01T00:00:00Z\n",
            "event: NotAnEvent\n",
        );
        let events = extract_agent_events(content);
        assert!(events.is_empty());
    }

    #[test]
    fn extract_agent_events_event_at_end_of_content() {
        // Event block at the very end of content (no trailing newline).
        let content = concat!(
            "  event: LastEvent\n",
            "  target-knot: knot-a\n",
            "  data: final\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "LastEvent");
        assert_eq!(events[0].target_knot, "knot-a");
    }

    #[test]
    fn extract_agent_events_mixed_with_full_tieoff_sections() {
        // Realistic scenario: full tie-off content with multiple sections,
        // some containing events and some not.
        let content = concat!(
            "## planner triggered by Created spec.md\n",
            "Timestamp: 2026-06-25T10:00:00Z\n",
            "---\n",
            "Spec reviewed. No issues found.\n",
            "---\n",
            "## planner triggered by Created plan.md\n",
            "Timestamp: 2026-06-25T11:00:00Z\n",
            "---\n",
            "Plan created successfully.\n",
            "\n",
            "  event: PlanCreated\n",
            "  target-knot: implementation-planner\n",
            "  plan: PLAN-007\n",
            "  description: Add intent-based event routing\n",
            "---\n",
            "## planner triggered by Modified plan.md\n",
            "Timestamp: 2026-06-25T12:00:00Z\n",
            "---\n",
            "Plan updated with new scope.\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert_eq!(events[0].target_knot, "implementation-planner");
        assert_eq!(
            events[0].payload.get("plan"),
            Some(&"PLAN-007".to_string())
        );
        assert_eq!(
            events[0].payload.get("description"),
            Some(&"Add intent-based event routing".to_string())
        );
    }

    #[test]
    fn extract_agent_events_tabs_as_indentation() {
        // Tab indentation should also work.
        let content = concat!(
            "\tevent: TabIndented\n",
            "\ttarget-knot: tab-knot\n",
            "\tdata: tabs work\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "TabIndented");
        assert_eq!(events[0].target_knot, "tab-knot");
    }

    #[test]
    fn extract_agent_events_empty_event_id() {
        // `event:` with empty value — still creates an event with empty ID.
        let content = concat!(
            "  event: \n",
            "  target-knot: knot-x\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "");
        assert_eq!(events[0].target_knot, "knot-x");
    }

    #[test]
    fn extract_agent_events_quoted_values() {
        // Values with surrounding quotes — quotes are preserved as-is
        // (consumer code handles unquoting if needed).
        let content = concat!(
            "  event: QuotedData\n",
            "  target-knot: q-knot\n",
            "  description: \"A plan with 'quotes'\"\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].payload.get("description"),
            Some(&"\"A plan with 'quotes'\"".to_string())
        );
    }
}
