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
/// Scans the tie-off body for ```markdown code blocks. Each block contains
/// YAML-style frontmatter (between `---` delimiters) followed by a freeform
/// body. The frontmatter contains key-value pairs where the first key must be
/// `event:` (the event identifier). All other keys become the payload.
///
/// ## Format
///
/// ```ignore
/// ```markdown
/// ---
/// event: PlanCreated
/// plan: PLAN-001
/// description: Implementation plan for event routing
/// ---
///
/// The plan covers three phases.
/// ```
///
/// Multiple events are emitted as separate ```markdown blocks.
///
/// Each block carries `event`, `occurred`, `description`, and optionally
/// `timestamp` and other payload fields. Events with `occurred: false`
/// are still returned by the parser (they are filtered at dispatch time).
///
/// ## Graceful handling
///
/// - Only ```markdown blocks are parsed (other language tags ignored).
/// - `occurred` defaults to `true` if absent (backwards compat).
/// - Missing closing `---` in frontmatter: body is None, frontmatter still
///   parsed.
/// - Empty body after `---` is allowed (body is None).
/// - Malformed lines (no `:` separator) in frontmatter are skipped.
/// - Multiple ```markdown blocks: all events are collected and returned.
pub fn extract_agent_events(
    content: &str,
) -> Vec<crate::domain::events::AgentEvent> {
    let mut events: Vec<crate::domain::events::AgentEvent> = Vec::new();

    let blocks = extract_markdown_blocks(content);

    for block_content in blocks {
        if let Some(event) = parse_event_block(&block_content) {
            events.push(event);
        }
    }

    events
}

/// Determine if the tie-off content contains any agent events.
///
/// Returns `true` if zero event blocks were found.
/// Returns `false` if at least one event block was found.
///
/// This is a lightweight check — it does not call `extract_agent_events()`
/// (which fully parses events). It only checks for the presence of
/// ```markdown blocks with `---` delimiters. If zero such blocks exist,
/// the agent produced no events at all.
///
/// This is the gate for event enforcement — if the agent was instructed
/// to emit events but produced zero event blocks, enforcement triggers.
pub fn has_no_events(content: &str) -> bool {
    let blocks = extract_markdown_blocks(content);
    if blocks.is_empty() {
        return true;
    }

    // At least one ```markdown block exists — check if any contains
    // a `---` delimiter (valid frontmatter). A block without `---`
    // is just a markdown code block, not an event block.
    for block in &blocks {
        if block.lines().any(|line| line.trim() == "---") {
            return false;
        }
    }

    true
}

/// Extract the content of ```markdown code blocks from text.
///
/// Only blocks that start with exactly ```markdown (no other language tag)
/// are included. Returns the content between the opening and closing fences.
fn extract_markdown_blocks(content: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut in_block = false;
    let mut current_lines: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "```markdown" {
            if !in_block {
                in_block = true;
                current_lines.clear();
            }
            // If already in a block, treat ```markdown as content (nested)
            continue;
        }

        if in_block {
            if trimmed == "```" {
                blocks.push(current_lines.join("\n"));
                current_lines.clear();
                in_block = false;
            } else {
                current_lines.push(line.to_string());
            }
        }
    }

    // An agent that opens a ```markdown event block but never closes it (an
    // unclosed trailing fence — observed with smaller local models) still has
    // its frontmatter delimited by `---`. Emit the trailing block so
    // `parse_event_block` can recover the event instead of dropping a
    // correctly-declared (but fence-unclosed) handoff.
    if in_block && !current_lines.is_empty() {
        blocks.push(current_lines.join("\n"));
    }

    blocks
}

/// Parse a single ```markdown block's content into an AgentEvent.
///
/// The block contains YAML-style frontmatter between `---` delimiters,
/// followed by an optional freeform body.
///
/// Returns `None` if:
/// - `event:` key is missing
/// - No `---` delimiter found (not valid frontmatter)
///
/// The `occurred` field defaults to `true` if absent (backwards compat).
/// Boolean is parsed case-insensitively from the string value.
fn parse_event_block(
    content: &str,
) -> Option<crate::domain::events::AgentEvent> {
    let lines: Vec<&str> = content.lines().collect();

    // Find frontmatter delimiters
    let mut frontmatter_start: Option<usize> = None;
    let mut frontmatter_end: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        if line.trim() == "---" {
            if frontmatter_start.is_none() {
                frontmatter_start = Some(i);
            } else if frontmatter_end.is_none() {
                frontmatter_end = Some(i);
                break;
            }
        }
    }

    // No frontmatter opening delimiter — not a valid event block
    let start = frontmatter_start?;

    // Parse frontmatter key-value pairs
    let (mut payload, body) = if let Some(end) = frontmatter_end {
        // Both delimiters found — normal case. Front-matter lines are
        // passed **raw** (indentation preserved) so `parse_frontmatter`
        // can recognise and de-indent block-scalar bodies (plan 087 p4).
        let payload = parse_frontmatter(&lines[start + 1..end]);

        // Body is everything after the closing ---
        let body_lines: Vec<&str> = lines[end + 1..].to_vec();
        let body_text = body_lines.join("\n").trim().to_string();
        let body = if body_text.is_empty() {
            None
        } else {
            Some(body_text)
        };

        (payload, body)
    } else {
        // Opening --- found but no closing ---
        // Treat everything after opening --- as frontmatter, body is None
        (parse_frontmatter(&lines[start + 1..]), None)
    };

    // Extract event_id from payload
    let event_id = payload.get("event")?.clone();

    // Extract `occurred` field (defaults to true for backwards compat)
    let occurred = payload
        .get("occurred")
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(true);

    // Remove `event` and `occurred` from payload
    // (they are struct fields, not payload data)
    payload.remove("event");
    payload.remove("occurred");

    Some(crate::domain::events::AgentEvent {
        event_id,
        occurred,
        payload,
        body,
    })
}

/// Parse front-matter lines into a `HashMap` of key-value pairs.
///
/// Handles plain `key: value` lines and YAML **block scalars** (`key: |` /
/// `key: >`, with an optional `+`/`-` chomping indicator). A block scalar's
/// value is the indented body that follows the marker line, de-indented by
/// the common leading whitespace and joined with newlines — this is what
/// carries the multi-line `next-task-context` / `background-additional`
/// fields of a `TasksIncomplete` handoff through to the next continuation
/// hop (plan 087 phase 4).
///
/// `lines` must be **raw** (indentation preserved): block-scalar bodies are
/// recognised by their indentation relative to the column-0 key lines.
/// Non-blank lines without a `:` separator that are not part of a block
/// scalar are skipped.
pub fn parse_frontmatter(
    lines: &[&str],
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        // An indented line reaching the top of the loop is stray content —
        // a block-scalar body is consumed by the block branch below. Skip.
        if raw.starts_with(' ') || raw.starts_with('\t') {
            i += 1;
            continue;
        }
        if let Some((key, value)) = parse_kv_line(trimmed) {
            let value = value.trim().to_string();
            if is_block_scalar_marker(&value) {
                // Collect the indented body that follows the marker line.
                i += 1;
                let mut body: Vec<&str> = Vec::new();
                while i < lines.len() {
                    let next = lines[i];
                    if next.trim().is_empty() {
                        body.push("");
                        i += 1;
                    } else if next.starts_with(' ') || next.starts_with('\t') {
                        body.push(next);
                        i += 1;
                    } else {
                        break;
                    }
                }
                // Drop trailing blank lines from the body.
                while body.last().is_some_and(|l| l.is_empty()) {
                    body.pop();
                }
                map.insert(key, dedent_block(&body));
            } else {
                map.insert(key, value);
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    map
}

/// Returns `true` when `value` is a YAML block-scalar indicator: `|`, `|-`,
/// `|+`, `>`, `>-`, or `>+`.
fn is_block_scalar_marker(value: &str) -> bool {
    let v = value.trim();
    let rest = match v.chars().next() {
        Some(c) if c == '|' || c == '>' => &v[1..],
        _ => return false,
    };
    rest.chars().all(|c| c == '-' || c == '+')
}

/// De-indent a block-scalar body — drop the common leading indentation from
/// every non-blank line — and join the lines with newlines. Relative
/// indentation (e.g. a sub-bullet under a bullet) is preserved.
fn dedent_block(body: &[&str]) -> String {
    let min_indent = body
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    body.iter()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                l[min_indent.min(l.len())..].to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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


    // ── extract_agent_events Tests ── markdown block format ─────────

    #[test]
    fn extract_agent_events_empty_input() {
        let events = extract_agent_events("");
        assert!(events.is_empty());
    }

    #[test]
    fn extract_agent_events_no_markdown_blocks() {
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
    fn extract_agent_events_single_markdown_block_with_frontmatter_and_body() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "description: Implementation plan for event routing\n",
            "---\n",
            "\n",
            "The plan covers three phases.\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert_eq!(
            events[0].payload.get("plan"),
            Some(&"PLAN-001".to_string())
        );
        assert_eq!(
            events[0].payload.get("description"),
            Some(&"Implementation plan for event routing".to_string())
        );
        assert_eq!(
            events[0].body,
            Some("The plan covers three phases.".to_string())
        );
    }

    #[test]
    fn extract_agent_events_multiple_markdown_blocks() {
        let content = concat!(
            "Some intro text.\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n",
            "\n",
            "Plan created with initial scope.\n",
            "```\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: ScopeChanged\n",
            "plan: PLAN-001\n",
            "description: Phase 2 removed from scope\n",
            "---\n",
            "\n",
            "Phase 2 was removed after stakeholder review.\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert_eq!(events[0].body, Some("Plan created with initial scope.".to_string()));
        assert_eq!(events[1].event_id, "ScopeChanged");
        assert_eq!(
            events[1].payload.get("description"),
            Some(&"Phase 2 removed from scope".to_string())
        );
        assert_eq!(
            events[1].body,
            Some("Phase 2 was removed after stakeholder review.".to_string())
        );
    }

    #[test]
    fn extract_agent_events_non_markdown_language_tag_ignored() {
        let content = concat!(
            "```yaml\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n",
            "body text\n",
            "```\n",
        );
        let events = extract_agent_events(content);
        assert!(events.is_empty());
    }

    #[test]
    fn extract_agent_events_occurred_false_not_dispatched() {
        // occurred: false events are still parsed but should not be
        // dispatched (filtering happens at dispatch time, not parse time).
        // The parser returns them so enforcement can count them.
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "occurred: false\n",
            "description: No plan was created this session\n",
            "---\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert!(!events[0].occurred, "occurred should be false");
    }

    #[test]
    fn extract_agent_events_occurred_true_default() {
        // When occurred is omitted, it defaults to true (backwards compat)
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "description: Plan was created\n",
            "---\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert!(events[0].occurred, "occurred should default to true");
    }

    #[test]
    fn extract_agent_events_missing_closing_frontmatter_delimiter() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "description: No closing delimiter\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert_eq!(
            events[0].payload.get("plan"),
            Some(&"PLAN-001".to_string())
        );
        assert_eq!(events[0].body, None);
    }

    #[test]
    fn extract_agent_events_empty_body() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert_eq!(events[0].body, None);
    }

    #[test]
    fn extract_agent_events_body_with_blank_lines_and_formatting() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n",
            "\n",
            "## Summary\n",
            "\n",
            "Key points:\n",
            "- Point one\n",
            "- Point two\n",
            "\n",
            "That's it.\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].body,
            Some(concat!(
                "## Summary\n",
                "\n",
                "Key points:\n",
                "- Point one\n",
                "- Point two\n",
                "\n",
                "That's it."
            ).to_string())
        );
    }

    #[test]
    fn extract_agent_events_values_with_colons() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: FileProcessed\n",
            "path: /home/user/file.md\n",
            "timestamp: 2026-06-25T10:00:00Z\n",
            "---\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
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
    fn extract_agent_events_malformed_frontmatter_lines_skipped() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: DataProcessed\n",
            "this line has no colon\n",
            "record-count: 42\n",
            "---\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].payload.get("record-count"),
            Some(&"42".to_string())
        );
    }

    #[test]
    fn extract_agent_events_block_without_event_key_skipped() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "plan: PLAN-001\n",
            "description: No event key\n",
            "---\n",
            "body text\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert!(events.is_empty());
    }

    #[test]
    fn extract_agent_events_with_surrounding_text() {
        let content = concat!(
            "Here is some analysis of the PRD.\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: GoalsApproved\n",
            "prd: PRD-042\n",
            "---\n",
            "\n",
            "Goals were reviewed and approved.\n",
            "```\n",
            "\n",
            "Additional notes follow.",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "GoalsApproved");
        assert_eq!(events[0].payload.get("prd"), Some(&"PRD-042".to_string()));
    }

    #[test]
    fn extract_agent_events_unclosed_trailing_fence_still_parsed() {
        // Plan 086 regression: a smaller model opened the ```markdown event
        // block (with `|` block-scalar fields) but never closed the fence.
        // The frontmatter `---` delimiters are intact, so the event must still
        // be recovered instead of dropped as "handoff-missed".
        let content = concat!(
            "Wrapping up per the water-mark.\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: TasksIncomplete\n",
            "occurred: true\n",
            "description: Batch stopped at the water-mark with work remaining\n",
            "timestamp: 2026-09-08T16:10:00+10:00\n",
            "next-task-context: |\n",
            "  Resume by reading work/checklist.md and continue top to bottom.\n",
            "  Mark each item done only after its deliverable is written.\n",
            "background-additional: |\n",
            "  [hop 2]\n",
            "  T1 done; T2-T6 remain pending.\n",
            "---\n",
            "\n",
            "Stopping here per the water-mark note. (fence intentionally left open)",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1, "unclosed trailing fence must not drop the event");
        assert_eq!(events[0].event_id, "TasksIncomplete");
        assert!(events[0].occurred);
        // Plan 087 p4: the `|` block-scalar bodies must be captured verbatim
        // (de-indented), not collapsed to the literal "|" marker.
        assert_eq!(
            events[0].payload.get("next-task-context"),
            Some(&"Resume by reading work/checklist.md and continue top to bottom.\nMark each item done only after its deliverable is written.".to_string()),
            "next-task-context block-scalar body must round-trip"
        );
        assert_eq!(
            events[0].payload.get("background-additional"),
            Some(&"[hop 2]\nT1 done; T2-T6 remain pending.".to_string()),
            "background-additional block-scalar body must round-trip"
        );
    }

    #[test]
    fn parse_frontmatter_block_scalar_captured_verbatim() {
        // Plan 087 p4: multi-line `|` block-scalar fields are captured
        // verbatim (de-indented). Body lines that contain a `:` (which the
        // naive splitter would mis-parse as a new top-level key) must stay
        // inside the block, not leak into the payload.
        let lines: Vec<&str> = vec![
            "event: TasksIncomplete",
            "occurred: true",
            "plan: project/plans/01-x/plan.md",
            "next-task-context: |",
            "  Run the two app-UI legs: test:e2e, then test:e2e-app.",
            "  1. test:e2e (showroom): prerequisite — start the dev server.",
            "  2. test:e2e-app (app): prerequisite — build then preview.",
            "  Once both pass, mark the gate item [x].",
            "background-additional: |",
            "  - Only the new data layer changed; app wiring untouched.",
            "  - Build green (tsc --noEmit && vite build).",
        ];
        let map = parse_frontmatter(&lines);
        assert_eq!(map.get("event"), Some(&"TasksIncomplete".to_string()));
        assert_eq!(map.get("occurred"), Some(&"true".to_string()));
        assert_eq!(
            map.get("plan"),
            Some(&"project/plans/01-x/plan.md".to_string())
        );
        assert_eq!(
            map.get("next-task-context"),
            Some(&"Run the two app-UI legs: test:e2e, then test:e2e-app.\n1. test:e2e (showroom): prerequisite — start the dev server.\n2. test:e2e-app (app): prerequisite — build then preview.\nOnce both pass, mark the gate item [x].".to_string())
        );
        assert_eq!(
            map.get("background-additional"),
            Some(&"- Only the new data layer changed; app wiring untouched.\n- Build green (tsc --noEmit && vite build).".to_string())
        );
        // No spurious keys leaked from the block-scalar bodies.
        assert!(!map.contains_key("test"));
        assert!(!map.contains_key("1. test"));
        assert_eq!(map.len(), 5);
    }

    #[test]
    fn parse_frontmatter_block_scalar_folding_and_chomp_markers() {
        // `>`, `>-`, `|+` are all recognised as block-scalar indicators
        // (plan 087 p4); v1 stores the de-indented body for each.
        let lines: Vec<&str> = vec![
            "folded: >",
            "  line one",
            "  line two",
            "kept: |-",
            "  alpha",
            "  beta",
        ];
        let map = parse_frontmatter(&lines);
        assert_eq!(map.get("folded"), Some(&"line one\nline two".to_string()));
        assert_eq!(map.get("kept"), Some(&"alpha\nbeta".to_string()));
    }

    #[test]
    fn extract_agent_events_three_events_all_collected() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n",
            "Plan created.\n",
            "```\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: ScopeChanged\n",
            "plan: PLAN-001\n",
            "---\n",
            "Scope reduced.\n",
            "```\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: GoalsApproved\n",
            "plan: PLAN-001\n",
            "approver: lead\n",
            "---\n",
            "Goals approved.\n",
            "```",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert_eq!(events[1].event_id, "ScopeChanged");
        assert_eq!(events[2].event_id, "GoalsApproved");
    }

    #[test]
    fn extract_agent_events_mixed_occurred_true_and_false() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "occurred: true\n",
            "plan: PLAN-001\n",
            "---\n",
            "Plan created.\n",
            "```\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: ValidationFailed\n",
            "occurred: false\n",
            "description: Validation did not fail\n",
            "---\n",
            "```\n",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert!(events[0].occurred);
        assert_eq!(events[1].event_id, "ValidationFailed");
        assert!(!events[1].occurred);
    }

    #[test]
    fn extract_agent_events_mixed_with_full_tieoff_sections() {
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
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-007\n",
            "description: Add intent-based event routing\n",
            "---\n",
            "\n",
            "Plan created with three phases.\n",
            "```\n",
            "---\n",
            "## planner triggered by Modified plan.md\n",
            "Timestamp: 2026-06-25T12:00:00Z\n",
            "---\n",
            "Plan updated with new scope.",
        );
        let events = extract_agent_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "PlanCreated");
        assert_eq!(
            events[0].payload.get("plan"),
            Some(&"PLAN-007".to_string())
        );
        assert_eq!(
            events[0].payload.get("description"),
            Some(&"Add intent-based event routing".to_string())
        );
        assert_eq!(
            events[0].body,
            Some("Plan created with three phases.".to_string())
        );
    }

    // ── extract_markdown_blocks Tests ─────────────────────────────

    #[test]
    fn extract_markdown_blocks_finds_single_block() {
        let content = concat!(
            "Some text.\n",
            "```markdown\n",
            "---\n",
            "event: Test\n",
            "---\n",
            "body\n",
            "```\n",
            "More text.",
        );
        let blocks = extract_markdown_blocks(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0],
            concat!("---\n", "event: Test\n", "---\n", "body")
        );
    }

    #[test]
    fn extract_markdown_blocks_finds_multiple_blocks() {
        let content = concat!(
            "```markdown\n",
            "block one\n",
            "```\n",
            "\n",
            "```markdown\n",
            "block two\n",
            "```",
        );
        let blocks = extract_markdown_blocks(content);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], "block one");
        assert_eq!(blocks[1], "block two");
    }

    #[test]
    fn extract_markdown_blocks_ignores_other_language_tags() {
        let content = concat!(
            "```yaml\n",
            "event: Test\n",
            "```\n",
            "\n",
            "```markdown\n",
            "real block\n",
            "```\n",
            "\n",
            "```json\n",
            "ignored\n",
            "```",
        );
        let blocks = extract_markdown_blocks(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0], "real block");
    }

    // ── parse_event_block Tests ───────────────────────────────────

    #[test]
    fn parse_event_block_missing_frontmatter_returns_none() {
        let content = "event: Test\nplan: PLAN-001";
        let result = parse_event_block(content);
        assert!(result.is_none());
    }

    #[test]
    fn parse_event_block_no_event_key_returns_none() {
        let content = concat!(
            "---\n",
            "plan: PLAN-001\n",
            "---\n",
            "body",
        );
        let result = parse_event_block(content);
        assert!(result.is_none());
    }

    // ── has_no_events Tests (Phase 1) ─────────────────────────────

    #[test]
    fn has_no_events_empty_input_returns_true() {
        assert!(has_no_events(""));
    }

    #[test]
    fn has_no_events_normal_body_returns_true() {
        let content = concat!(
            "## review triggered by Created file.md\n",
            "Timestamp: 2026-06-01T00:00:00Z\n",
            "---\n",
            "Normal body text without any events.",
        );
        assert!(has_no_events(content));
    }

    #[test]
    fn has_no_events_with_event_block_returns_false() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n",
            "Plan created.\n",
            "```",
        );
        assert!(!has_no_events(content));
    }

    #[test]
    fn has_no_events_with_occurred_false_returns_false() {
        // occurred: false still counts as an event block (acknowledgement)
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "occurred: false\n",
            "description: No plan created\n",
            "---\n",
            "```",
        );
        assert!(!has_no_events(content));
    }

    #[test]
    fn has_no_events_with_multiple_events_returns_false() {
        let content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "---\n",
            "```
",
            "```markdown\n",
            "---\n",
            "event: ScopeChanged\n",
            "---\n",
            "```",
        );
        assert!(!has_no_events(content));
    }

    #[test]
    fn has_no_events_yaml_block_ignored() {
        let content = concat!(
            "```yaml\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n",
            "body text\n",
            "```",
        );
        assert!(has_no_events(content));
    }
}
