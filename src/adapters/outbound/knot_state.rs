use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::application::ports::{
    KnotEventType, KnotState, ProcessingStatus, PortError,
};
use crate::domain::entities::{KnotId, StrandPath};

/// Filesystem-backed implementation of `KnotStatePort`.
///
/// Stores per-knot state as JSON files in `<loom_dir>/.knots/<knot-name>.state`.
pub struct FileSystemKnotStateStore {
    loom_dir: PathBuf,
}

impl FileSystemKnotStateStore {
    /// Create a new store for the given loom directory.
    pub fn new(loom_dir: PathBuf) -> Self {
        Self { loom_dir }
    }

    /// Return the path to the `.knots` subdirectory.
    fn knots_dir(&self) -> PathBuf {
        self.loom_dir.join(".knots")
    }

    /// Return the `.state` file path for a given knot ID.
    fn state_file_path(&self, knot_id: &KnotId) -> PathBuf {
        self.knots_dir().join(format!("{}.state", knot_id.0))
    }

    /// Write a `KnotState` to disk as pretty-printed JSON.
    fn write_state(&self, state: &KnotState) -> Result<(), PortError> {
        let path = self.state_file_path(&state.knot_id);
        let dir = path.parent().ok_or_else(|| {
            PortError::LoomSaveFailed("invalid state path".to_string())
        })?;
        fs::create_dir_all(dir)
            .map_err(|e| PortError::LoomSaveFailed(e.to_string()))?;
        let json = serde_json::to_string_pretty(state)
            .map_err(|e| PortError::LoomSaveFailed(e.to_string()))?;
        fs::write(&path, json)
            .map_err(|e| PortError::LoomSaveFailed(e.to_string()))?;
        Ok(())
    }

    /// Format the current time as a simple ISO-like string.
    fn now_iso() -> String {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        format!(
            "{}.{:09}Z",
            duration.as_secs(),
            duration.subsec_nanos()
        )
    }
}

// KnotStatePort trait removed in Phase 2 — impl disabled.
// impl KnotStatePort for FileSystemKnotStateStore {
//     fn create(&self, knot_id: &KnotId) -> Result<(), PortError> { ... }
//     fn update(&self, state: KnotState) -> Result<(), PortError> { ... }
//     fn get(&self, knot_id: &KnotId) -> Result<Option<KnotState>, PortError> { ... }
// }

// ── Tests ──────────────────────────────────────────────────────────────────

// #[cfg(test)]
#[cfg(feature = "__disabled_tests")]
mod tests {
    use super::*;
    use crate::domain::entities::TieOffPath;

    #[test]
    fn knot_state_create_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemKnotStateStore::new(dir.path().to_path_buf());
        let knot_id = KnotId("test-knot".to_string());

        let result = store.create(&knot_id);
        assert!(
            result.is_ok(),
            "create should succeed"
        );

        // File exists on disk in .knots subdirectory.
        let state_path = dir.path().join(".knots/test-knot.state");
        assert!(
            state_path.exists(),
            "state file should exist on disk after create"
        );

        // File contains valid JSON with idle status.
        let content = fs::read_to_string(&state_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            parsed["status"], "idle",
            "new state should have idle status"
        );
    }

    #[test]
    fn knot_state_update_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemKnotStateStore::new(dir.path().to_path_buf());
        let knot_id = KnotId("test-knot".to_string());

        // Create initial state.
        store.create(&knot_id).unwrap();

        // Update with a new state.
        let new_state = KnotState {
            knot_id: knot_id.clone(),
            event_type: KnotEventType::Modified,
            strand_path: StrandPath(PathBuf::from("input.md")),
            tie_off_path: None,
            status: ProcessingStatus::Processing,
            error: None,
            last_updated: "2026-01-01T00:00:00Z".to_string(),
        };
        let result = store.update(new_state.clone());
        assert!(
            result.is_ok(),
            "update should succeed"
        );

        // Read the file back and verify it matches the new state.
        let state_path = dir.path().join(".knots/test-knot.state");
        let content = fs::read_to_string(&state_path).unwrap();
        let parsed: KnotState = serde_json::from_str(&content).unwrap();
        assert_eq!(
            parsed.status,
            ProcessingStatus::Processing,
            "status should be updated to processing"
        );
        assert_eq!(
            parsed.event_type,
            KnotEventType::Modified,
            "event_type should be updated"
        );
    }

    #[test]
    fn knot_state_read_current() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemKnotStateStore::new(dir.path().to_path_buf());
        let knot_id = KnotId("read-knot".to_string());

        // Write a state via update.
        let state = KnotState {
            knot_id: knot_id.clone(),
            event_type: KnotEventType::Created,
            strand_path: StrandPath(PathBuf::from("doc.md")),
            tie_off_path: Some(TieOffPath(PathBuf::from("output.md"))),
            status: ProcessingStatus::Completed,
            error: None,
            last_updated: "2026-06-01T12:00:00Z".to_string(),
        };
        store.update(state).unwrap();

        // Read via get().
        let result = store.get(&knot_id);
        assert!(
            result.is_ok(),
            "get should succeed"
        );
        let read_state = result
            .unwrap()
            .expect("state should exist");
        assert_eq!(
            read_state.status,
            ProcessingStatus::Completed,
            "status should round-trip correctly"
        );
        assert_eq!(
            read_state.tie_off_path,
            Some(TieOffPath(PathBuf::from("output.md"))),
            "tie_off_path should round-trip correctly"
        );
        assert_eq!(
            read_state.last_updated,
            "2026-06-01T12:00:00Z",
            "last_updated should round-trip correctly"
        );
    }

    #[test]
    fn knot_state_status_transitions() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemKnotStateStore::new(dir.path().to_path_buf());
        let knot_id = KnotId("transition-knot".to_string());

        // Step 1: create -> idle.
        store.create(&knot_id).unwrap();
        let state = store.get(&knot_id).unwrap().unwrap();
        assert_eq!(
            state.status,
            ProcessingStatus::Idle,
            "initial state should be idle"
        );

        // Step 2: update to processing.
        let processing_state = KnotState {
            knot_id: knot_id.clone(),
            event_type: KnotEventType::Created,
            strand_path: StrandPath(PathBuf::from("input.md")),
            tie_off_path: None,
            status: ProcessingStatus::Processing,
            error: None,
            last_updated: "2026-01-01T00:00:01Z".to_string(),
        };
        store.update(processing_state).unwrap();
        let state = store.get(&knot_id).unwrap().unwrap();
        assert_eq!(
            state.status,
            ProcessingStatus::Processing,
            "state should transition to processing"
        );

        // Step 3: update to completed.
        let completed_state = KnotState {
            knot_id: knot_id.clone(),
            event_type: KnotEventType::Created,
            strand_path: StrandPath(PathBuf::from("input.md")),
            tie_off_path: Some(TieOffPath(PathBuf::from("output.md"))),
            status: ProcessingStatus::Completed,
            error: None,
            last_updated: "2026-01-01T00:00:02Z".to_string(),
        };
        store.update(completed_state).unwrap();
        let state = store.get(&knot_id).unwrap().unwrap();
        assert_eq!(
            state.status,
            ProcessingStatus::Completed,
            "state should transition to completed"
        );
    }

    #[test]
    fn knot_state_get_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemKnotStateStore::new(dir.path().to_path_buf());
        let knot_id = KnotId("does-not-exist".to_string());

        let result = store.get(&knot_id);
        assert!(
            result.is_ok(),
            "get for nonexistent knot should return Ok"
        );
        assert!(
            result.unwrap().is_none(),
            "get for nonexistent knot should return None"
        );
    }
}
