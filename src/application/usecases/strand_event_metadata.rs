//! Event metadata extraction helpers.
//!
//! Self-contained functions for parsing strand files and extracting
//! event metadata for a2a traceability. Does not depend on `ProcessStrand`
//! state.

use crate::domain::entities::{EventMetadata, Knot, LoomId, StrandPath};

// ── Expected Event IDs ────────────────────────────────────────────

/// Extract the list of expected event IDs for a knot.
///
/// Scans all knots' `strand_source` entries for `EventUri` subscriptions
/// where the current knot is the producer. Returns the deduplicated list
/// of event IDs that the agent was instructed to emit.
///
/// This mirrors the scanning logic in `build_listener_context()` but
/// returns only the event IDs (not the full context string).
pub fn extract_expected_event_ids(
    knot: &Knot,
    loom_id: &LoomId,
    all_knots: &[Knot],
) -> Vec<String> {
    use crate::domain::value_objects::StrandSource;

    let mut seen_ids: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();

    for other in all_knots {
        if let StrandSource::EventUri {
            producer_knot,
            event_id,
        } = &other.strand_source
        {
            let is_knot_level = producer_knot == &knot.id.0;
            let is_loom_level =
                producer_knot.ends_with("-loom") && producer_knot == &loom_id.0;
            if is_knot_level || is_loom_level {
                seen_ids.entry(event_id.clone()).or_insert(true);
            }
        }
    }

    let mut ids: Vec<String> = seen_ids.into_keys().collect();
    ids.sort();
    ids
}

// ── Event File Detection ─────────────────────────────────────────

/// Try to read event metadata from a strand file.
///
/// When a strand file is an event file dispatched by intent-based routing
/// (filename starts with `event-` and has YAML frontmatter containing
/// `event-id`), this parses the frontmatter and returns populated
/// [`EventMetadata`] for a2a traceability in the consumer's tie-off.
///
/// Returns `None` when the file is not an event file or parsing fails.
pub fn extract_event_metadata(
    strand_path: &StrandPath,
) -> Option<EventMetadata> {
    // Quick check: only event files (filename starts with `event-`)
    let filename = strand_path
        .0
        .file_name()
        .map(|f| f.to_string_lossy().to_string())?
    ;
    if !filename.starts_with("event-") {
        return None;
    }

    let content = std::fs::read_to_string(&strand_path.0).ok()?;
    // Parse YAML frontmatter (--- delimited)
    let frontmatter = parse_yaml_frontmatter(&content)?;

    let event_id = frontmatter.get("event-id").cloned();
    let source_knot = frontmatter.get("target-knot").cloned();

    if event_id.is_none() && source_knot.is_none() {
        return None;
    }

    Some(EventMetadata {
        event_id,
        source_knot,
        original_strand: frontmatter.get("original-strand").cloned(),
    })
}

/// Parse YAML frontmatter from a markdown file.
///
/// Expects `---` delimited frontmatter at the start of the file.
/// Returns a simple key-value map (no nested YAML support needed
/// for event files).
pub fn parse_yaml_frontmatter(
    content: &str,
) -> Option<std::collections::HashMap<String, String>> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---\n") && !trimmed.starts_with("---\r\n")
        && trimmed != "---"
    {
        return None;
    }

    // Find the opening delimiter
    let after_open = if trimmed.starts_with("---\n") {
        &trimmed[4..]
    } else if trimmed.starts_with("---\r\n") {
        &trimmed[6..]
    } else {
        // Bare "---" with no content — no closing delimiter
        return None;
    };

    // Find the closing delimiter
    let close_pos = after_open.find("\n---").or_else(|| {
        after_open.find("\r\n---")
    })?;

    let frontmatter_text = &after_open[..close_pos];
    let mut map = std::collections::HashMap::new();

    for line in frontmatter_text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            map.insert(
                key.trim().to_string(),
                value.trim().to_string(),
            );
        }
    }

    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod event_metadata_tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // ── parse_yaml_frontmatter Tests ─────────────────────────────────

    #[test]
    fn parse_yaml_frontmatter_valid_basic() {
        let content = "---\nevent-id: PlanCreated\ntarget-knot: plan-creator\ntimestamp: 2026-07-09T12:00:00Z\n---\n\nBody text";
        let result = parse_yaml_frontmatter(content);
        assert!(result.is_some());
        let map = result.unwrap();
        assert_eq!(map.get("event-id"), Some(&"PlanCreated".to_string()));
        assert_eq!(map.get("target-knot"), Some(&"plan-creator".to_string()));
        assert_eq!(
            map.get("timestamp"),
            Some(&"2026-07-09T12:00:00Z".to_string())
        );
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn parse_yaml_frontmatter_with_payload_fields() {
        let content = "---\nevent-id: PlanCreated\ntarget-knot: plan-creator\nplan: PLAN-001\ndescription: Test plan\n---\n\nBody";
        let result = parse_yaml_frontmatter(content);
        assert!(result.is_some());
        let map = result.unwrap();
        assert_eq!(map.get("event-id"), Some(&"PlanCreated".to_string()));
        assert_eq!(map.get("plan"), Some(&"PLAN-001".to_string()));
        assert_eq!(
            map.get("description"),
            Some(&"Test plan".to_string())
        );
    }

    #[test]
    fn parse_yaml_frontmatter_no_frontmatter_returns_none() {
        let content = "No frontmatter here\nJust body text";
        assert!(parse_yaml_frontmatter(content).is_none());
    }

    #[test]
    fn parse_yaml_frontmatter_empty_frontmatter_returns_none() {
        let content = "---\n---\n\nBody text";
        assert!(parse_yaml_frontmatter(content).is_none());
    }

    #[test]
    fn parse_yaml_frontmatter_whitespace_trimming() {
        let content = "---\nevent-id: PlanCreated \n target-knot: plan-creator \n---\n\nBody";
        let result = parse_yaml_frontmatter(content);
        assert!(result.is_some());
        let map = result.unwrap();
        // Values should be trimmed
        assert_eq!(map.get("event-id"), Some(&"PlanCreated".to_string()));
        assert_eq!(map.get("target-knot"), Some(&"plan-creator".to_string()));
    }

    #[test]
    fn parse_yaml_frontmatter_skips_empty_lines() {
        let content = "---\nevent-id: PlanCreated\n\ntarget-knot: plan-creator\n\n---\n\nBody";
        let result = parse_yaml_frontmatter(content);
        assert!(result.is_some());
        let map = result.unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("event-id"), Some(&"PlanCreated".to_string()));
        assert_eq!(map.get("target-knot"), Some(&"plan-creator".to_string()));
    }

    // ── extract_event_metadata Tests ─────────────────────────────────

    fn write_event_file(
        dir: &TempDir,
        filename: &str,
        content: &str,
    ) -> PathBuf {
        let path = dir.path().join(filename);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn extract_event_metadata_from_event_file() {
        let dir = TempDir::new().unwrap();
        let path = write_event_file(
            &dir,
            "event-2026-07-09T12-00-00Z.md",
            "---\nevent-id: PlanCreated\ntarget-knot: plan-creator\ntimestamp: 2026-07-09T12:00:00Z\n---\n\n## Event: PlanCreated from plan-creator\n\nPayload:\n\n- **plan**: PLAN-001",
        );
        let result =
            extract_event_metadata(&StrandPath(path.clone()));

        assert!(result.is_some(), "should extract metadata from event file");
        let meta = result.unwrap();
        assert_eq!(meta.event_id.as_deref(), Some("PlanCreated"));
        assert_eq!(meta.source_knot.as_deref(), Some("plan-creator"));
        assert!(meta.original_strand.is_none());
    }

    #[test]
    fn extract_event_metadata_returns_none_for_regular_file() {
        let dir = TempDir::new().unwrap();
        let path = write_event_file(
            &dir,
            "normal-strand.md",
            "This is a normal file.",
        );
        let result =
            extract_event_metadata(&StrandPath(path.clone()));
        assert!(
            result.is_none(),
            "should return None for non-event files"
        );
    }

    #[test]
    fn extract_event_metadata_returns_none_for_event_file_without_frontmatter() {
        let dir = TempDir::new().unwrap();
        let path = write_event_file(
            &dir,
            "event-2026-07-09T12-00-00Z.md",
            "No frontmatter, just body text.",
        );
        let result =
            extract_event_metadata(&StrandPath(path.clone()));
        assert!(
            result.is_none(),
            "should return None when event file has no YAML frontmatter"
        );
    }

    #[test]
    fn extract_event_metadata_returns_none_for_missing_file() {
        let path = StrandPath(PathBuf::from("/nonexistent/event-123.md"));
        let result = extract_event_metadata(&path);
        assert!(
            result.is_none(),
            "should return None for missing file"
        );
    }

    #[test]
    fn extract_event_metadata_returns_none_for_empty_event_id_and_target() {
        // Event filename but frontmatter has no event-id or target-knot
        let dir = TempDir::new().unwrap();
        let path = write_event_file(
            &dir,
            "event-2026-07-09T12-00-00Z.md",
            "---\nsome-other-field: value\n---\n\nBody",
        );
        let result =
            extract_event_metadata(&StrandPath(path.clone()));
        assert!(
            result.is_none(),
            "should return None when event-id and target-knot are both missing"
        );
    }

    #[test]
    fn extract_event_metadata_partial_fields() {
        // Event file with only event-id, no target-knot
        let dir = TempDir::new().unwrap();
        let path = write_event_file(
            &dir,
            "event-2026-07-09T12-00-00Z.md",
            "---\nevent-id: PlanCreated\n---\n\nBody",
        );
        let result =
            extract_event_metadata(&StrandPath(path.clone()));

        assert!(result.is_some());
        let meta = result.unwrap();
        assert_eq!(meta.event_id.as_deref(), Some("PlanCreated"));
        assert!(meta.source_knot.is_none());
    }

    #[test]
    fn extract_event_metadata_with_original_strand() {
        // Event file that includes original-strand in frontmatter
        let dir = TempDir::new().unwrap();
        let path = write_event_file(
            &dir,
            "event-2026-07-09T12-00-00Z.md",
            "---\nevent-id: PlanCreated\ntarget-knot: plan-creator\noriginal-strand: 001-feature.md\n---\n\nBody",
        );
        let result =
            extract_event_metadata(&StrandPath(path.clone()));

        assert!(result.is_some());
        let meta = result.unwrap();
        assert_eq!(meta.event_id.as_deref(), Some("PlanCreated"));
        assert_eq!(meta.source_knot.as_deref(), Some("plan-creator"));
        assert_eq!(meta.original_strand.as_deref(), Some("001-feature.md"));
    }
}
