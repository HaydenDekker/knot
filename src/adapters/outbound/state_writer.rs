//! Filesystem-backed implementation of `StateWriterPort`.
//!
//! Writes `RigState` JSON to `{rig_dir}/state.json` using atomic
//! write (write to `.state.json.tmp`, then rename).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::application::ports::{PortError, StateWriterPort};
use crate::domain::entities::RigState;

/// Filesystem-backed implementation of `StateWriterPort`.
///
/// Writes atomically: serialize to a temp file, then rename into place.
/// This prevents readers from seeing partially-written state.
#[derive(Clone)]
pub struct FileSystemStateWriter {
    /// Directory where `state.json` is written (typically the rig directory).
    rig_dir: PathBuf,
}

impl FileSystemStateWriter {
    /// Create a new filesystem-backed state writer.
    ///
    /// # Arguments
    ///
    /// * `rig_dir` - Path to the rig directory (e.g. `/project/rig`).
    pub fn new(rig_dir: PathBuf) -> Self {
        Self { rig_dir }
    }

    /// Resolve the state file path.
    fn state_path(&self) -> PathBuf {
        self.rig_dir.join("state.json")
    }

    /// Resolve the temp file path for atomic write.
    fn tmp_path(&self) -> PathBuf {
        self.rig_dir.join(".state.json.tmp")
    }
}

impl StateWriterPort for FileSystemStateWriter {
    fn write_state(&self, state: &RigState) -> Result<(), PortError> {
        // Serialize to JSON with pretty printing
        let json = serde_json::to_string_pretty(state).map_err(|e| {
            PortError::StateWriteFailed(format!(
                "failed to serialize state: {e}"
            ))
        })?;

        // Ensure the rig directory exists
        fs::create_dir_all(&self.rig_dir).map_err(|e| {
            PortError::StateWriteFailed(format!(
                "failed to create rig directory {}: {e}",
                self.rig_dir.display()
            ))
        })?;

        // Write to temp file first (atomic write pattern)
        let tmp = self.tmp_path();
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| {
                PortError::StateWriteFailed(format!(
                    "failed to open temp file {}: {e}",
                    tmp.display()
                ))
            })?;

        // Write JSON content
        write!(file, "{}", json).map_err(|e| {
            PortError::StateWriteFailed(format!(
                "failed to write temp file {}: {e}",
                tmp.display()
            ))
        })?;

        // Ensure data is flushed to disk before rename
        file.flush().map_err(|e| {
            PortError::StateWriteFailed(format!(
                "failed to flush temp file {}: {e}",
                tmp.display()
            ))
        })?;

        // Atomic rename: move temp file into final position.
        // Uses `std::fs::rename` which is atomic on POSIX.
        let final_path = self.state_path();
        fs::rename(&tmp, &final_path).map_err(|e| {
            PortError::StateWriteFailed(format!(
                "failed to rename {} to {}: {e}",
                tmp.display(),
                final_path.display()
            ))
        })?;

        Ok(())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{
        RigStateKnot, RigStateLoom, RigStateProfile,
    };

    /// Build a minimal `RigState` for testing.
    fn build_state(rig_path: &str) -> RigState {
        RigState {
            rig_path: rig_path.to_string(),
            looms: vec![RigStateLoom {
                id: "test-loom".to_string(),
                knots: vec![RigStateKnot {
                    id: "k1".to_string(),
                    status: "idle".to_string(),
                    last_strand_path: None,
                    last_tie_off_path: None,
                    last_error: None,
                    last_event_at: None,
                }],
            }],
            profiles: vec![RigStateProfile {
                name: "fast".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                timeout: None,
            }],
            updated_at: "2026-06-18T12:00:00Z".to_string(),
        }
    }

    #[test]
    fn write_state_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemStateWriter::new(dir.path().to_path_buf());
        let state = build_state(dir.path().to_str().unwrap());

        let result = writer.write_state(&state);
        assert!(result.is_ok());

        // Verify the file exists at rig/state.json
        let path = dir.path().join("state.json");
        assert!(path.exists(), "state.json should exist");

        // Verify no temp file remains
        let tmp = dir.path().join(".state.json.tmp");
        assert!(!tmp.exists(), "temp file should be cleaned up");
    }

    #[test]
    fn write_state_writes_correct_json() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemStateWriter::new(dir.path().to_path_buf());
        let state = build_state("/test/rig");

        writer.write_state(&state).unwrap();

        let content = fs::read_to_string(dir.path().join("state.json")).unwrap();

        // Parse back and verify structure
        let parsed: RigState = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.rig_path, "/test/rig");
        assert_eq!(parsed.looms.len(), 1);
        assert_eq!(parsed.looms[0].id, "test-loom");
        assert_eq!(parsed.looms[0].knots.len(), 1);
        assert_eq!(parsed.looms[0].knots[0].id, "k1");
        assert_eq!(parsed.profiles.len(), 1);
        assert_eq!(parsed.profiles[0].name, "fast");
    }

    #[test]
    fn write_state_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemStateWriter::new(dir.path().to_path_buf());

        // Write first state
        let mut state1 = build_state("/first");
        state1.updated_at = "2026-01-01T00:00:00Z".to_string();
        writer.write_state(&state1).unwrap();

        // Write second state
        let mut state2 = build_state("/second");
        state2.updated_at = "2026-12-31T23:59:59Z".to_string();
        writer.write_state(&state2).unwrap();

        // Read back and verify it's the second state
        let content = fs::read_to_string(dir.path().join("state.json")).unwrap();
        let parsed: RigState = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.rig_path, "/second");
        assert_eq!(parsed.updated_at, "2026-12-31T23:59:59Z");
    }

    #[test]
    fn write_state_creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("sub").join("rig");
        let writer = FileSystemStateWriter::new(nested.clone());
        let state = build_state("/nested/rig");

        // sub/rig doesn't exist yet
        assert!(!nested.exists());

        let result = writer.write_state(&state);
        assert!(result.is_ok(), "should create parent dirs");

        assert!(nested.join("state.json").exists());
    }

    #[test]
    fn write_state_atomic_no_partial_on_temp_rename() {
        // Verify the atomic write pattern: after write_state,
        // only the final file exists, no temp file.
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemStateWriter::new(dir.path().to_path_buf());
        let state = build_state("/atomic");

        writer.write_state(&state).unwrap();

        // List all files in the directory
        let entries: Vec<_> = fs::read_dir(&dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();

        // Should contain state.json but not .state.json.tmp
        assert!(entries.contains(&"state.json".to_string()));
        assert!(!entries.iter().any(|n| n.contains(".tmp")));
    }

    #[test]
    fn write_state_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemStateWriter::new(dir.path().to_path_buf());
        let state = RigState {
            rig_path: "/empty".to_string(),
            looms: vec![],
            profiles: vec![],
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        writer.write_state(&state).unwrap();

        let content = fs::read_to_string(dir.path().join("state.json")).unwrap();
        let parsed: RigState = serde_json::from_str(&content).unwrap();
        assert!(parsed.looms.is_empty());
        assert!(parsed.profiles.is_empty());
    }

    #[test]
    fn write_state_is_pretty_printed() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemStateWriter::new(dir.path().to_path_buf());
        let state = build_state("/pretty");

        writer.write_state(&state).unwrap();

        let content = fs::read_to_string(dir.path().join("state.json")).unwrap();

        // Pretty-printed JSON contains newlines and indentation
        assert!(content.contains('\n'), "should be pretty-printed");
        assert!(
            content.contains("  "),
            "should contain indentation"
        );
    }

    #[test]
    fn write_state_trait_object_safe() {
        let writer = FileSystemStateWriter::new(PathBuf::from("/tmp"));
        // Verify trait is object-safe
        let _obj: &dyn StateWriterPort = &writer;
    }

    #[test]
    fn write_state_is_clone() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileSystemStateWriter::new(dir.path().to_path_buf());

        // Should be cloneable (Clone derive)
        let _writer2 = writer.clone();
    }

    #[test]
    fn write_state_concurrent_writes() {
        // In production the state writer runs on a single task, so writes
        // are naturally serialised. This test verifies that serialised
        // writes from multiple threads produce valid output.
        let dir = tempfile::tempdir().unwrap();
        let writer = std::sync::Arc::new(std::sync::Mutex::new(
            FileSystemStateWriter::new(dir.path().to_path_buf()),
        ));
        let mut handles = Vec::new();

        for i in 0..10 {
            let writer = std::sync::Arc::clone(&writer);
            let handle = std::thread::spawn(move || {
                let state = RigState {
                    rig_path: "/concurrent".to_string(),
                    looms: vec![],
                    profiles: vec![RigStateProfile {
                        name: format!("profile-{i}"),
                        provider: "openai".to_string(),
                        model: "gpt-4o".to_string(),
                        timeout: None,
                    }],
                    updated_at: format!("2026-06-18T00:00:0{i}Z"),
                };
                writer.lock().unwrap().write_state(&state).unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Final state.json should be valid JSON
        let content =
            fs::read_to_string(dir.path().join("state.json")).unwrap();
        let _parsed: RigState = serde_json::from_str(&content).unwrap();
    }
}
