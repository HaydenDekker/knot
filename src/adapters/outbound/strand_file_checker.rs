//! Strand file checker adapter.
//!
//! Implements `StrandFileChecker` using `content_inspector::is_text_file()`.

use crate::adapters::outbound::content_inspector::is_text_file;
use crate::domain::entities::StrandFileChecker;
use std::path::Path;

/// Adapter that bridges the domain `StrandFileChecker` trait to the
/// `content_inspector` adapter.
pub struct ContentInspectorChecker;

impl StrandFileChecker for ContentInspectorChecker {
    fn is_text_file(&self, path: &Path) -> Result<bool, std::io::Error> {
        is_text_file(path).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("content_inspector failed: {e}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn checker_detects_text_file() {
        let dir = TempDir::new().unwrap();
        let text_path = dir.path().join("hello.txt");
        std::fs::write(&text_path, "hello world").unwrap();

        let result = ContentInspectorChecker.is_text_file(&text_path);
        assert!(result.unwrap(), "should be text");
    }

    #[test]
    fn checker_detects_binary_file() {
        let dir = TempDir::new().unwrap();
        let binary_path = dir.path().join("data.bin");
        std::fs::write(&binary_path, vec![0x00, 0x01, 0x02, 0xFF])
            .unwrap();

        let result = ContentInspectorChecker.is_text_file(&binary_path);
        assert!(!result.unwrap(), "should be binary");
    }

    #[test]
    fn checker_empty_file_is_text() {
        let dir = TempDir::new().unwrap();
        let empty_path = dir.path().join("empty.txt");
        std::fs::write(&empty_path, "").unwrap();

        let result = ContentInspectorChecker.is_text_file(&empty_path);
        assert!(result.unwrap(), "empty file should be text");
    }

    #[test]
    fn checker_nonexistent_file_returns_error() {
        let path = Path::new("/nonexistent/file.txt");
        let result = ContentInspectorChecker.is_text_file(path);
        assert!(result.is_err());
    }
}
