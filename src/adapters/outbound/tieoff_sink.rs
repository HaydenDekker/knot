use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::adapters::logging;
use crate::application::ports::{PortError, TieOffSink};
use crate::domain::entities::{TieOff, TieOffPath};

/// Plan 099 (Phase 4): process-wide serialisation of tie-off appends.
///
/// Knots run concurrently on the dispatch threads and every section
/// goes to the one unified `tie-offs/<rig>/tie-offs.log`. The service
/// is the sole writer of the file; this lock serialises its appends so
/// sections never interleave.
static TIEOFF_APPEND_LOCK: Mutex<()> = Mutex::new(());

/// Filesystem-backed implementation of `TieOffSink`.
///
/// Plan 099: the sink appends every tie-off section to the single
/// accumulating `tie-offs/<rig>/tie-offs.log` — the path arrives on
/// the `TieOff` entity and is the same file for all knots. The section
/// opens with one structured, greppable header line (knot, event,
/// strand, timestamp, session, source, original strand), then the
/// content with the event audit slimmed to one line per event (id,
/// occurred, reason, timestamp — no payload bodies; the commit stays
/// the fidelity source per plan 099 D7, and the full session narrative
/// stays reachable via `pi --session-id`).
pub struct FileSystemTieOffSink {
    #[allow(dead_code)]
    tie_off_dir: PathBuf,
}

impl FileSystemTieOffSink {
    /// Create a new sink. `tie_off_dir` is retained for construction
    /// compatibility; the append target is the path carried by each
    /// `TieOff` (the unified log).
    pub fn new(tie_off_dir: PathBuf) -> Self {
        Self { tie_off_dir }
    }

    /// Render the section's structured first line — one greppable
    /// line carrying the trigger metadata. `None` fields are omitted,
    /// never written empty.
    fn render_section_header(tie_off: &TieOff) -> String {
        let knot_name = tie_off
            .knot_name
            .clone()
            .unwrap_or_else(|| "knot".to_string());
        let event_label = tie_off
            .event_type
            .clone()
            .unwrap_or_else(|| "Processed".to_string());
        let strand_label = tie_off.strand_path.clone().unwrap_or_default();
        let timestamp = tie_off
            .timestamp
            .clone()
            .unwrap_or_else(logging::format_timestamp);

        let mut header = format!(
            "## knot={knot_name} event={event_label}"
        );
        // The triggering event id (when the run was triggered by a
        // delivered emission or an `event-*` strand) rides right
        // after the run's own event label.
        let metadata = &tie_off.event_metadata;
        if let Some(ref event_id) = metadata.event_id {
            header.push_str(&format!(" event-id={event_id}"));
        }
        header.push_str(&format!(
            " strand={strand_label} timestamp={timestamp}"
        ));
        if let Some(ref session_id) = tie_off.session_id {
            header.push_str(&format!(" session={session_id}"));
        }
        if let Some(ref source) = metadata.source_knot {
            header.push_str(&format!(" source={source}"));
        }
        if let Some(ref original_strand) = metadata.original_strand {
            header.push_str(&format!(" original-strand={original_strand}"));
        }
        header
    }

    /// Plan 099: slim the event audit. Each ```markdown event block
    /// in the tie-off content is replaced, in place, by one slim audit
    /// line — `id`, `occurred`, `reason` (the block's `description`),
    /// and `timestamp`. No payload fields, no body: the full event
    /// stays in the commit body and the session narrative.
    ///
    /// Content outside event blocks is preserved verbatim.
    fn slim_event_audit(content: &str) -> String {
        let events =
            crate::domain::tieoff_parser::extract_agent_events(content);
        let re = regex::Regex::new(r"(?s)```markdown.*?(?:```|$)")
            .expect("event-block regex compiles");
        let mut idx = 0usize;
        let replaced = re.replace_all(content, |caps: &regex::Captures| {
            let i = idx;
            idx += 1;
            match events.get(i) {
                Some(e) => {
                    let reason = e
                        .payload
                        .get("description")
                        .cloned()
                        .unwrap_or_default()
                        .replace('\n', " ");
                    let ts = e
                        .payload
                        .get("timestamp")
                        .cloned()
                        .unwrap_or_default();
                    format!(
                        "- event-audit id={} occurred={} reason={} timestamp={}",
                        e.event_id, e.occurred, reason, ts
                    )
                }
                // A ```markdown block that is not an event block (no
                // parsable frontmatter) is dropped from the log — it
                // carries no audit value and the commit keeps it.
                None => String::new(),
            }
        });
        replaced.to_string()
    }
}

impl TieOffSink for FileSystemTieOffSink {
    fn write(&self, tie_off: TieOff) -> Result<(), PortError> {
        let path = &tie_off.path.0;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))?;
        }
        fs::write(path, &tie_off.content)
            .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))?;
        Ok(())
    }

    fn read_content(&self, path: &TieOffPath) -> Result<String, PortError> {
        if path.0.exists() {
            fs::read_to_string(&path.0)
                .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))
        } else {
            Ok(String::new())
        }
    }

    fn append(&self, tie_off: TieOff) -> Result<(), PortError> {
        let path = &tie_off.path.0;

        // Ensure parent directories exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))?;
        }

        let section = {
            // Sole-writer serialisation (plan 099): knots append
            // concurrently but the unified log takes one section at a
            // time. A poisoned lock is recovered — the guard protects
            // only the append ordering, no invariant can corrupt.
            let _guard = TIEOFF_APPEND_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            let header = Self::render_section_header(&tie_off);
            let body = Self::slim_event_audit(&tie_off.content);
            let section = format!("{header}\n{body}");

            let mut file = OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(path)
                .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))?;

            let mut buf = String::new();
            if file.metadata().map(|m| m.len()).unwrap_or(0) > 0 {
                buf.push_str("\n---\n");
            }
            buf.push_str(&section);
            file.write_all(buf.as_bytes())
                .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))?;
            file.flush()
                .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))?;
            section
        };

        let _ = section;
        Ok(())
    }
}



// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{EventMetadata, TieOffStatus};
    use std::sync::Arc;

    #[test]
    fn tieoff_write_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());

        let tie_off = TieOff {
            content: "Generated content".to_string(),
            path: TieOffPath(dir.path().join("review.tie-off.md")),
            status: TieOffStatus::Produced,
            knot_name: None,
            event_type: None,
            strand_path: None,
            timestamp: None,
            agent_events: Vec::new(),
            event_metadata: EventMetadata::default(),
            session_id: None,
        };

        let result = sink.write(tie_off);
        assert!(
            result.is_ok(),
            "write should succeed"
        );

        let file_path = dir.path().join("review.tie-off.md");
        assert!(
            file_path.exists(),
            "tie-off file should be created on disk"
        );

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            content,
            "Generated content",
            "file should contain the tie-off content"
        );
    }

    #[test]
    fn tieoff_overwrite_existing() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());

        let path = TieOffPath(dir.path().join("output.tie-off.md"));

        // First write
        sink.write(TieOff {
            content: "First content".to_string(),
            path: path.clone(),
            status: TieOffStatus::Produced,
            knot_name: None,
            event_type: None,
            strand_path: None,
            timestamp: None,
            agent_events: Vec::new(),
            event_metadata: EventMetadata::default(),
            session_id: None,
        })
        .unwrap();

        // Second write (overwrite)
        sink.write(TieOff {
            content: "Second content".to_string(),
            path: path.clone(),
            status: TieOffStatus::Produced,
            knot_name: None,
            event_type: None,
            strand_path: None,
            timestamp: None,
            agent_events: Vec::new(),
            event_metadata: EventMetadata::default(),
            session_id: None,
        })
        .unwrap();

        let file_path = dir.path().join("output.tie-off.md");
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            content,
            "Second content",
            "file should contain the second write, not the first"
        );
    }

    #[test]
    fn tieoff_create_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();

        let sink = FileSystemTieOffSink::new(base.clone());

        // Tie-off path has nested subdirectories that don't exist yet
        let tie_off = TieOff {
            content: "Deep content".to_string(),
            path: TieOffPath(base.join("sub/dir/deep.tie-off.md")),
            status: TieOffStatus::Produced,
            knot_name: None,
            event_type: None,
            strand_path: None,
            timestamp: None,
            agent_events: Vec::new(),
            event_metadata: EventMetadata::default(),
            session_id: None,
        };

        let sub_dir = dir.path().join("sub/dir");
        assert!(
            !sub_dir.exists(),
            "subdirectory should not exist before write"
        );

        let result = sink.write(tie_off);
        assert!(
            result.is_ok(),
            "write should create parent directories and succeed"
        );

        assert!(
            sub_dir.exists(),
            "parent directories should be created by write()"
        );

        let file_path = dir.path().join("sub/dir/deep.tie-off.md");
        assert!(
            file_path.exists(),
            "tie-off file should exist in newly created directory"
        );

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            content,
            "Deep content",
            "file should contain the tie-off content"
        );
    }

    #[test]
    fn tieoff_sink_trait_object_safe() {
        let sink = FileSystemTieOffSink::new(PathBuf::from("/tmp"));
        // Verify trait is object-safe
        let _obj: &dyn TieOffSink = &sink;
    }

    #[test]
    fn append_mode_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("tie-offs.log");

        let tie_off = make_tie_off(
            &file_path,
            "First section content",
            "review",
            "Created",
            "strand1.md",
            "2026-06-05T00:00:00Z",
            None,
        );

        assert!(
            !file_path.exists(),
            "file should not exist before append"
        );

        let result = sink.append(tie_off);
        assert!(result.is_ok(), "append should succeed");
        assert!(file_path.exists(), "file should be created");

        let content = fs::read_to_string(&file_path).unwrap();
        // Plan 099: the section opens with one structured header line.
        assert_eq!(
            content,
            "## knot=review event=Created strand=strand1.md \
timestamp=2026-06-05T00:00:00Z\nFirst section content",
            "unified-log section shape: {content}"
        );
    }

    #[test]
    fn append_mode_adds_section() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("tie-offs.log");

        sink.append(make_tie_off(
            &file_path,
            "Section one",
            "review",
            "Created",
            "strand.md",
            "2026-06-05T10:00:00Z",
            None,
        ))
        .unwrap();
        sink.append(make_tie_off(
            &file_path,
            "Section two",
            "review",
            "Modified",
            "strand.md",
            "2026-06-05T11:00:00Z",
            None,
        ))
        .unwrap();

        let content = fs::read_to_string(&file_path).unwrap();

        // Sections separated by exactly one `---` line.
        assert_eq!(
            content,
            "## knot=review event=Created strand=strand.md \
timestamp=2026-06-05T10:00:00Z\nSection one\n---\n\
## knot=review event=Modified strand=strand.md \
timestamp=2026-06-05T11:00:00Z\nSection two",
            "two-section unified-log shape: {content}"
        );
        // Section two comes after section one.
        let pos_one = content.find("Section one").unwrap();
        let pos_two = content.find("Section two").unwrap();
        assert!(pos_one < pos_two, "chronological order");
    }

    #[test]
    fn append_mode_preserves_history() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("tie-offs.log");

        let events = [
            ("Created", "Initial content", "2026-06-05T10:00:00Z"),
            ("Modified", "Updated content", "2026-06-05T11:00:00Z"),
            ("Deleted", "Deleted content", "2026-06-05T12:00:00Z"),
        ];
        for (event_type, body, ts) in &events {
            sink.append(make_tie_off(
                &file_path,
                body,
                "review",
                event_type,
                "strand.md",
                ts,
                None,
            ))
            .unwrap();
        }

        let content = fs::read_to_string(&file_path).unwrap();
        for (event_type, body, _) in &events {
            assert!(
                content.contains(&format!(
                    "## knot=review event={event_type} strand=strand.md"
                )),
                "should have {event_type} trigger: {}",
                content
            );
            assert!(
                content.contains(body),
                "should have {body}: {}",
                content
            );
        }
        // Chronological order.
        let pos_created = content.find("Initial content").unwrap();
        let pos_modified = content.find("Updated content").unwrap();
        let pos_deleted = content.find("Deleted content").unwrap();
        assert!(
            pos_created < pos_modified && pos_modified < pos_deleted,
            "sections should be in chronological order"
        );
        // Two separators between three sections.
        assert_eq!(
            content.lines().filter(|l| *l == "---").count(),
            2,
            "three sections, two separators: {content}"
        );
    }

    /// Plan 099: the session rides the structured header line — same
    /// line as the timestamp, after it.
    #[test]
    fn append_session_rides_the_header_line() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("tie-offs.log");

        sink.append(make_tie_off(
            &file_path,
            "Section body",
            "review",
            "Created",
            "strand.md",
            "2026-08-22T12:00:00+00:00",
            Some("sess-123"),
        ))
        .unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        let header = content.lines().next().unwrap();
        assert!(
            header.contains("timestamp=2026-08-22T12:00:00+00:00 session=sess-123"),
            "session rides the header line right after the timestamp: {header}"
        );
    }

    /// Plan 099: `None` session is omitted from the header, never
    /// written empty.
    #[test]
    fn append_omits_session_when_none() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("tie-offs.log");

        sink.append(make_tie_off(
            &file_path,
            "Section body",
            "review",
            "Created",
            "strand.md",
            "2026-08-22T12:00:00+00:00",
            None,
        ))
        .unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(
            content,
            "## knot=review event=Created strand=strand.md \
timestamp=2026-08-22T12:00:00+00:00\nSection body",
            "no-session section keeps the exact unified-log shape"
        );
        assert!(!content.contains("session="));
    }

    /// Plan 099: every trace field rides the one header line, in the
    /// fixed order: knot, event, strand, timestamp, session, source,
    /// original-strand.
    #[test]
    fn append_header_carries_all_trace_fields_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("tie-offs.log");

        let mut tie_off = make_tie_off(
            &file_path,
            "Body",
            "review",
            "Modified",
            "strand.md",
            "2026-08-22T12:00:00+00:00",
            Some("sess-999"),
        );
        tie_off.event_metadata = EventMetadata {
            event_id: Some("PlanCreated".to_string()),
            source_knot: Some("producer-knot".to_string()),
            original_strand: Some("project/plans/x.md".to_string()),
        };
        sink.append(tie_off).unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        let header = content.lines().next().unwrap();
        assert_eq!(
            header,
            "## knot=review event=Modified event-id=PlanCreated \
strand=strand.md timestamp=2026-08-22T12:00:00+00:00 \
session=sess-999 source=producer-knot \
original-strand=project/plans/x.md",
            "full structured header line"
        );
    }

    /// Plan 099: the event audit is slimmed — each ```markdown event
    /// block is replaced by one `event-audit` line (id, occurred,
    /// reason, timestamp). No payload fields, no body.
    #[test]
    fn append_slims_event_blocks_to_audit_lines() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("tie-offs.log");

        let mut tie_off = make_tie_off(
            &file_path,
            concat!(
                "Work summary.\n\n",
                "```markdown\n---\n",
                "event: PlanCreated\noccurred: true\n",
                "description: The plan was approved.\n",
                "timestamp: 2026-10-23T09:00:00Z\nplan: PLAN-9\n",
                "---\n\nLong narrative body that must not reach the log.\n",
                "```\n\nClosing note.",
            ),
            "author",
            "Created",
            "strands/spec.md",
            "2026-10-23T09:00:00Z",
            None,
        );
        tie_off.event_metadata = EventMetadata::default();
        sink.append(tie_off).unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert!(
            content.contains(
                "- event-audit id=PlanCreated occurred=true \
reason=The plan was approved. timestamp=2026-10-23T09:00:00Z"
            ),
            "slim audit line expected: {content}"
        );
        assert!(
            !content.contains("Long narrative body"),
            "event body must NOT reach the unified log: {content}"
        );
        assert!(
            !content.contains("plan: PLAN-9"),
            "payload fields must NOT reach the unified log: {content}"
        );
        assert!(
            content.contains("Work summary.") && content.contains("Closing note."),
            "prose outside event blocks is preserved: {content}"
        );
    }

    /// Plan 099: an `occurred: false` block audits with occurred=false
    /// — the acknowledgement stays visible, nothing dispatches.
    #[test]
    fn append_audit_records_non_occurred_events() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("tie-offs.log");

        sink.append(make_tie_off(
            &file_path,
            concat!(
                "Summary.\n\n",
                "```markdown\n---\n",
                "event: PlanRejected\noccurred: false\n",
                "description: Nothing to reject.\n",
                "---\n",
                "```",
            ),
            "author",
            "Created",
            "strands/spec.md",
            "2026-10-23T09:00:00Z",
            None,
        ))
        .unwrap();

        let content = fs::read_to_string(&file_path).unwrap();
        assert!(
            content.contains(
                "- event-audit id=PlanRejected occurred=false \
reason=Nothing to reject. timestamp="
            ),
            "non-occurred audit line expected: {content}"
        );
    }

    /// Plan 099: sole-writer serialisation — concurrent appends from
    /// many threads land whole, never interleaved.
    #[test]
    fn append_serialises_concurrent_writers() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Arc::new(FileSystemTieOffSink::new(
            dir.path().to_path_buf(),
        ));
        let file_path = dir.path().join("tie-offs.log");

        let mut handles = Vec::new();
        for i in 0..8 {
            let sink = Arc::clone(&sink);
            let path = file_path.clone();
            handles.push(std::thread::spawn(move || {
                sink.append(make_tie_off(
                    &path,
                    &format!("body-{i}"),
                    &format!("knot-{i}"),
                    "Created",
                    "strand.md",
                    "2026-10-23T09:00:00Z",
                    None,
                ))
                .unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let content = fs::read_to_string(&file_path).unwrap();
        // All eight sections present, each header immediately followed
        // by its own body (no interleaving).
        assert_eq!(
            content.lines().filter(|l| l.starts_with("## knot=")).count(),
            8,
            "eight headers: {content}"
        );
        for i in 0..8 {
            let pair = format!("knot-{i} \nplaceholder");
            let _ = pair;
            let header = format!("## knot=knot-{i} ");
            let hpos = content
                .find(&header)
                .unwrap_or_else(|| panic!("header {header} missing"));
            let body_pos = content
                .find(&format!("body-{i}"))
                .unwrap_or_else(|| panic!("body-{i} missing"));
            assert!(
                body_pos > hpos
                    && body_pos - hpos
                        < header.len() + 200,
                "section {i} must be contiguous (header {hpos}, body {body_pos})"
            );
        }
        // Seven separators between eight sections.
        assert_eq!(
            content.lines().filter(|l| *l == "---").count(),
            7,
            "eight sections, seven separators: {content}"
        );
    }

    fn make_tie_off(
        path: &std::path::Path,
        content: &str,
        knot: &str,
        event: &str,
        strand: &str,
        ts: &str,
        session: Option<&str>,
    ) -> TieOff {
        TieOff {
            content: content.to_string(),
            path: TieOffPath(path.to_path_buf()),
            status: TieOffStatus::Produced,
            knot_name: Some(knot.to_string()),
            event_type: Some(event.to_string()),
            strand_path: Some(strand.to_string()),
            timestamp: Some(ts.to_string()),
            agent_events: Vec::new(),
            event_metadata: EventMetadata::default(),
            session_id: session.map(str::to_string),
        }
    }
}
