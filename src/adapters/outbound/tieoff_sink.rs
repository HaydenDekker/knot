use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::application::ports::{PortError, TieOffSink};
use crate::domain::entities::{StrandPath, TieOff, TieOffPath};

/// Filesystem-backed implementation of `TieOffSink`.
///
/// Writes tie-off content as plain text files. Derives tie-off filenames
/// from strand filenames: `<name>.tie-off.<ext>`.
pub struct FileSystemTieOffSink {
    tie_off_dir: PathBuf,
}

impl FileSystemTieOffSink {
    /// Create a new sink that writes into `tie_off_dir`.
    pub fn new(tie_off_dir: PathBuf) -> Self {
        Self { tie_off_dir }
    }

    /// Derive a tie-off filename from a strand filename.
    ///
    /// Pattern: `<name>.tie-off.<ext>`.
    /// E.g. `input.md` → `input.tie-off.md`
    ///      `report` → `report.tie-off`
    pub fn derive_tieoff_filename(strand_path: &StrandPath) -> String {
        let filename = strand_path.0.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "output".to_string());

        let extension = strand_path.0.extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()));

        match extension {
            Some(ext) => format!("{}.tie-off{}", filename, ext),
            None => format!("{}.tie-off", filename),
        }
    }

    /// Resolve the full tie-off path for a given strand.
    pub fn resolve_path(&self, strand_path: &StrandPath) -> TieOffPath {
        let filename = Self::derive_tieoff_filename(strand_path);
        TieOffPath(self.tie_off_dir.join(filename))
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

        let file_exists = path.exists();

        // Build the section content with metadata header
        let event_label = tie_off
            .event_type
            .clone()
            .unwrap_or_else(|| "Processed".to_string());
        let strand_label = tie_off
            .strand_path
            .clone()
            .unwrap_or_default();
        let timestamp = tie_off.timestamp.clone().unwrap_or_else(|| {
            Self::format_timestamp(SystemTime::now())
        });

        let mut new_content = String::new();
        new_content.push_str(&format!(
            "## Event: {event_label}\n## Strand: {strand_label}\n## Timestamp: {timestamp}\n---\n"
        ));
        new_content.push_str(&tie_off.content);

        if file_exists {
            // Read existing content, prepend delimiter
            let existing = fs::read_to_string(path)
                .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))?;
            let mut full_content = existing;
            full_content.push_str("\n---\n");
            full_content.push_str(&new_content);
            fs::write(path, full_content)
                .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))?;
        } else {
            // Create file with header section (no leading ---)
            fs::write(path, new_content)
                .map_err(|e| PortError::TieOffWriteFailed(e.to_string()))?;
        }

        Ok(())
    }
}

impl FileSystemTieOffSink {
    /// Format a SystemTime as ISO 8601 UTC string.
    fn format_timestamp(time: SystemTime) -> String {
        let duration = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs();

        // Compute UTC date/time components from Unix epoch seconds
        let days = secs / 86400;
        let remaining = secs % 86400;
        let hours = remaining / 3600;
        let minutes = (remaining % 3600) / 60;
        let seconds = remaining % 60;

        // Convert days since epoch to year/month/day
        let (year, month, day) = Self::days_to_ymd(days);

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            year, month, day, hours, minutes, seconds
        )
    }

    /// Convert days since Unix epoch (1970-01-01) to (year, month, day).
    fn days_to_ymd(days: u64) -> (u64, u64, u64) {
        let d = days as i64 + 719468; // Adjust for algorithm
        let era = if d >= 0 { d } else { d - 146096 } / 146097;
        let doe = d - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe as u64 + era as u64 * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = mp + if mp < 10 { 3 } else { -9 };
        let year = y + if month <= 2 { 1 } else { 0 };
        (year, month as u64, day as u64)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::TieOffStatus;

    #[test]
    fn tieoff_write_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());

        let tie_off = TieOff {
            content: "Generated content".to_string(),
            path: TieOffPath(dir.path().join("review.tie-off.md")),
            status: TieOffStatus::Produced,
            event_type: None,
            strand_path: None,
            timestamp: None,
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
            event_type: None,
            strand_path: None,
            timestamp: None,
        })
        .unwrap();

        // Second write (overwrite)
        sink.write(TieOff {
            content: "Second content".to_string(),
            path: path.clone(),
            status: TieOffStatus::Produced,
            event_type: None,
            strand_path: None,
            timestamp: None,
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
    fn tieoff_filename_derived_from_strand() {
        let strand = StrandPath(PathBuf::from("input.md"));
        let filename = FileSystemTieOffSink::derive_tieoff_filename(&strand);

        assert_eq!(
            filename,
            "input.tie-off.md",
            "strand input.md should produce tie-off input.tie-off.md"
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
            event_type: None,
            strand_path: None,
            timestamp: None,
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
    fn tieoff_filename_no_extension() {
        let strand = StrandPath(PathBuf::from("report"));
        let filename = FileSystemTieOffSink::derive_tieoff_filename(&strand);

        assert_eq!(
            filename,
            "report.tie-off",
            "strand without extension should get .tie-off suffix"
        );
    }

    #[test]
    fn tieoff_filename_complex_extension() {
        let strand = StrandPath(PathBuf::from("document.markdown"));
        let filename = FileSystemTieOffSink::derive_tieoff_filename(&strand);

        assert_eq!(
            filename,
            "document.tie-off.markdown",
            "complex extension should be preserved"
        );
    }

    #[test]
    fn tieoff_resolve_path() {
        let sink = FileSystemTieOffSink::new(PathBuf::from("output/reviews"));
        let strand = StrandPath(PathBuf::from("input.md"));
        let path = sink.resolve_path(&strand);

        assert_eq!(
            path.0,
            PathBuf::from("output/reviews/input.tie-off.md"),
            "resolve_path should join tie_off_dir with derived filename"
        );
    }

    #[test]
    fn append_mode_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("first.tie-off");

        let tie_off = TieOff {
            content: "First section content".to_string(),
            path: TieOffPath(file_path.clone()),
            status: TieOffStatus::Produced,
            event_type: Some("Created".to_string()),
            strand_path: Some("strand1.md".to_string()),
            timestamp: Some("2026-06-05T00:00:00Z".to_string()),
        };

        assert!(
            !file_path.exists(),
            "file should not exist before append"
        );

        let result = sink.append(tie_off);
        assert!(result.is_ok(), "append should succeed");
        assert!(file_path.exists(), "file should be created");

        let content = fs::read_to_string(&file_path).unwrap();
        assert!(
            content.contains("## Event: Created"),
            "content should have event header: {}", content
        );
        assert!(
            content.contains("## Strand: strand1.md"),
            "content should have strand header: {}", content
        );
        assert!(
            content.contains("## Timestamp: 2026-06-05T00:00:00Z"),
            "content should have timestamp header: {}", content
        );
        assert!(
            content.contains("First section content"),
            "content should include body: {}", content
        );
        // First section should NOT have a leading ---
        assert!(
            !content.starts_with("---"),
            "first section should not start with ---: {}", content
        );
    }

    #[test]
    fn append_mode_adds_section() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("history.tie-off");

        // First append
        let tie_off_1 = TieOff {
            content: "Section one".to_string(),
            path: TieOffPath(file_path.clone()),
            status: TieOffStatus::Produced,
            event_type: Some("Created".to_string()),
            strand_path: Some("strand.md".to_string()),
            timestamp: Some("2026-06-05T10:00:00Z".to_string()),
        };
        sink.append(tie_off_1).unwrap();

        // Second append
        let tie_off_2 = TieOff {
            content: "Section two".to_string(),
            path: TieOffPath(file_path.clone()),
            status: TieOffStatus::Produced,
            event_type: Some("Modified".to_string()),
            strand_path: Some("strand.md".to_string()),
            timestamp: Some("2026-06-05T11:00:00Z".to_string()),
        };
        sink.append(tie_off_2).unwrap();

        let content = fs::read_to_string(&file_path).unwrap();

        // Should have two --- delimiters (one after first header, one between sections)
        let delimiter_count = content.matches("---").count();
        assert!(
            delimiter_count >= 2,
            "should have at least 2 delimiter sections, found {}: {}",
            delimiter_count, content
        );
        assert!(
            content.contains("Section one"),
            "should preserve first section: {}", content
        );
        assert!(
            content.contains("Section two"),
            "should have second section: {}", content
        );
        assert!(
            content.contains("Event: Created"),
            "should have first event type: {}", content
        );
        assert!(
            content.contains("Event: Modified"),
            "should have second event type: {}", content
        );
        // Section two should come after section one
        let pos_one = content.find("Section one").unwrap();
        let pos_two = content.find("Section two").unwrap();
        assert!(
            pos_one < pos_two,
            "sections should be in chronological order"
        );
    }

    #[test]
    fn append_mode_preserves_history() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("full-history.tie-off");

        let events = vec![
            (
                "Created".to_string(),
                "Initial content".to_string(),
                "2026-06-05T10:00:00Z".to_string(),
            ),
            (
                "Modified".to_string(),
                "Updated content".to_string(),
                "2026-06-05T11:00:00Z".to_string(),
            ),
            (
                "Deleted".to_string(),
                "Deleted content".to_string(),
                "2026-06-05T12:00:00Z".to_string(),
            ),
        ];

        for (event_type, body, ts) in &events {
            let tie_off = TieOff {
                content: body.clone(),
                path: TieOffPath(file_path.clone()),
                status: TieOffStatus::Produced,
                event_type: Some(event_type.clone()),
                strand_path: Some("strand.md".to_string()),
                timestamp: Some(ts.clone()),
            };
            sink.append(tie_off).unwrap();
        }

        let content = fs::read_to_string(&file_path).unwrap();

        // All three sections should be present
        assert!(
            content.contains("## Event: Created"),
            "should have Created event: {}", content
        );
        assert!(
            content.contains("Initial content"),
            "should have initial content: {}", content
        );
        assert!(
            content.contains("## Event: Modified"),
            "should have Modified event: {}", content
        );
        assert!(
            content.contains("Updated content"),
            "should have updated content: {}", content
        );
        assert!(
            content.contains("## Event: Deleted"),
            "should have Deleted event: {}", content
        );
        assert!(
            content.contains("Deleted content"),
            "should have deleted content: {}", content
        );

        // Three sections in chronological order
        let pos_created = content.find("Initial content").unwrap();
        let pos_modified = content.find("Updated content").unwrap();
        let pos_deleted = content.find("Deleted content").unwrap();
        assert!(
            pos_created < pos_modified && pos_modified < pos_deleted,
            "sections should be in chronological order"
        );

        // Should have --- delimiters between sections
        let delimiter_count = content.matches("---").count();
        assert!(
            delimiter_count >= 4,
            "should have multiple delimiters for 3 sections, found {}: {}",
            delimiter_count, content
        );
    }
}
