//! Text/binary file detection using the `content_inspector` crate.
//!
//! Probes the first bytes of a file for null bytes and known binary
//! signatures. Returns `true` for text files and `false` for binary
//! files (images, PDFs, archives, compiled binaries, etc.).

use std::fs;
use std::path::Path;

use crate::application::ports::PortError;

/// Probe the first bytes of a file to determine if it is text.
///
/// Reads up to 8 KB from the start of the file and inspects for
/// null bytes or known binary magic numbers. Files that are empty,
/// unreadable, or inaccessible are treated conservatively:
/// - Empty files (0 bytes) are treated as text (safe default).
/// - Unreadable files return `Err(PortError::RigScanFailed)`.
pub fn is_text_file(path: &Path) -> Result<bool, PortError> {
    let bytes = fs::read(path).map_err(|e| {
        PortError::RigScanFailed(format!(
            "failed to read file for text detection: {path:?}: {e}"
        ))
    })?;

    // Empty files are treated as text (safe default — nothing to parse)
    if bytes.is_empty() {
        return Ok(true);
    }

    let result = content_inspector::inspect(&bytes);

    // `content_inspector` returns `ContentType::BINARY` for files that
    // contain null bytes or known binary signatures (PNG, JPEG,
    // PDF, ZIP, ELF, etc.). Everything else is considered text.
    Ok(!matches!(result, content_inspector::ContentType::BINARY))
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn plain_text_file_detected_as_text() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("hello.txt");
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "Hello, world!").unwrap();
        drop(file);

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn rust_source_file_detected_as_text() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("main.rs");
        fs::write(&file_path, "fn main() { println!(\"hi\"); }").unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn json_file_detected_as_text() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("config.json");
        fs::write(&file_path, r#"{"key": "value"}"#).unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn yaml_file_detected_as_text() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("config.yaml");
        fs::write(&file_path, "key: value\n").unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn md_file_detected_as_text() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("readme.md");
        fs::write(&file_path, "# Title\n\nContent here.\n").unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn binary_with_null_bytes_detected_as_binary() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("data.bin");
        let data: Vec<u8> = vec![0x00, 0x01, 0x02, 0xFF, 0xFE];
        fs::write(&file_path, &data).unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn png_header_detected_as_binary() {
        // PNG magic bytes: 89 50 4E 47 0D 0A 1A 0A
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("image.png");
        let data: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
            0x00, 0x00, 0x00, 0x00, // rest doesn't matter
        ];
        fs::write(&file_path, &data).unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn pdf_header_detected_as_binary() {
        // PDF magic: %PDF-
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("doc.pdf");
        fs::write(&file_path, b"%PDF-1.4 fake pdf content").unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn zip_header_detected_as_binary() {
        // ZIP magic: PK\x03\x04 followed by null bytes (real archives
        // always contain null bytes in their internal structure)
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("archive.zip");
        let data: Vec<u8> = vec![
            0x50, 0x4B, 0x03, 0x04, // PK header
            0x00, 0x00, 0x00, 0x00, // null bytes in archive structure
        ];
        fs::write(&file_path, &data).unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn empty_file_treated_as_text() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("empty.txt");
        fs::write(&file_path, "").unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn non_existent_file_returns_error() {
        let result = is_text_file(Path::new("/nonexistent/path/file.txt"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        // Should be a RigScanFailed error
        match &err {
            PortError::RigScanFailed(msg) => {
                assert!(
                    msg.contains("failed to read file"),
                    "error message should describe the failure"
                );
            }
            _ => panic!("expected RigScanFailed, got {:?}", err),
        }
    }

    #[test]
    fn python_source_file_detected_as_text() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("script.py");
        fs::write(&file_path, "#!/usr/bin/env python3\nprint('hello')\n")
            .unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn javascript_file_detected_as_text() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("app.js");
        fs::write(&file_path, "console.log('hello');\n").unwrap();

        let result = is_text_file(&file_path);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }
}
