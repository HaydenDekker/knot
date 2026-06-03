use std::fs;
use std::path::PathBuf;

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
        })
        .unwrap();

        // Second write (overwrite)
        sink.write(TieOff {
            content: "Second content".to_string(),
            path: path.clone(),
            status: TieOffStatus::Produced,
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
}
