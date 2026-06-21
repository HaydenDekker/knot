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
}
