//! Filesystem-backed event store for disk-backed strand event queue.
//!
//! Provides low-level file CRUD operations: atomic write (temp → rename),
//! file removal, directory scanning, and single-file reads. All event
//! files live in a flat `rig/events/` directory as `{id}.json`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::domain::pending_event::{PendingEvent, PendingEventId};

/// Filesystem-backed event store.
///
/// Manages `{id}.json` files in a flat directory. The directory is created
/// on construction if it does not exist.
pub struct FileSystemEventStore {
    events_dir: PathBuf,
}

impl FileSystemEventStore {
    /// Create a new event store pointing at `events_dir`.
    ///
    /// Creates the directory (and any missing parents) if it does not
    /// already exist.
    pub fn new(events_dir: PathBuf) -> Self {
        fs::create_dir_all(&events_dir).expect(
            "unable to create events directory — check permissions",
        );
        Self { events_dir }
    }

    /// Return a reference to the events directory path.
    pub fn events_dir(&self) -> &Path {
        &self.events_dir
    }

    /// Resolve the path for a given event ID.
    pub fn event_path(&self, id: &PendingEventId) -> PathBuf {
        self.events_dir.join(format!("{}.json", id.0))
    }

    /// Resolve the temp file path for atomic writes.
    fn temp_path(&self, id: &PendingEventId) -> PathBuf {
        self.events_dir.join(format!("{}.json.tmp", id.0))
    }

    /// Write an event file atomically (temp file → rename).
    ///
    /// Serialises `event` to pretty-printed JSON, writes to a `.tmp` file,
    /// then renames it into the final `{id}.json` position. This prevents
    /// readers from seeing partially-written content.
    ///
    /// Returns the path of the written file on success.
    pub fn write_event(
        &self,
        event: &PendingEvent,
    ) -> std::io::Result<PathBuf> {
        let json = serde_json::to_string_pretty(event).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("failed to serialize event: {e}"),
            )
        })?;

        // Write to temp file first
        let tmp = self.temp_path(&event.id);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        write!(file, "{}", json)?;
        file.flush()?;

        // Atomic rename into final position
        let final_path = self.event_path(&event.id);
        fs::rename(&tmp, &final_path)?;

        Ok(final_path)
    }

    /// Remove the file for the given event ID.
    ///
    /// No-op if the file does not already exist.
    pub fn remove_event(&self, id: &PendingEventId) -> std::io::Result<()> {
        let path = self.event_path(id);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Scan the events directory and return all valid events, sorted by
    /// filename (which gives FIFO order due to the timestamp prefix).
    ///
    /// Malformed JSON files are skipped with a warning logged to stderr.
    /// Non-`.json` files are silently ignored.
    pub fn scan_events(&self) -> std::io::Result<Vec<PendingEvent>> {
        let mut events: Vec<(String, PendingEvent)> = Vec::new();

        for entry in fs::read_dir(&self.events_dir)? {
            let entry = entry?;
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();

            // Only process .json files (skip .tmp and everything else)
            if !filename_str.ends_with(".json") {
                continue;
            }

            let path = entry.path();
            let content = fs::read_to_string(&path)?;

            match serde_json::from_str::<PendingEvent>(&content) {
                Ok(event) => {
                    events.push((filename_str.to_string(), event));
                }
                Err(e) => {
                    eprintln!(
                        "[WARN] skipping malformed event file {}: {e}",
                        path.display()
                    );
                }
            }
        }

        // Sort by filename (timestamp prefix ensures FIFO order)
        events.sort_by(|(a, _), (b, _)| a.cmp(b));

        Ok(events.into_iter().map(|(_, event)| event).collect())
    }

    /// Read and parse a single event file by ID.
    ///
    /// Returns an `io::Error` with `NotFound` kind if the file does not
    /// exist. Returns a generic error if the file exists but is malformed.
    pub fn read_event(&self, id: &PendingEventId) -> std::io::Result<PendingEvent> {
        let path = self.event_path(id);
        if !path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("event file not found: {}", path.display()),
            ));
        }

        let content = fs::read_to_string(&path)?;
        serde_json::from_str(&content).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to parse event file {}: {e}", path.display()),
            )
        })
    }

    /// Return the number of `.json` files in the events directory.
    pub fn event_count(&self) -> usize {
        fs::read_dir(&self.events_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .ends_with(".json")
                    })
                    .count()
            })
            .unwrap_or(0)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `PendingEvent` for testing.
    fn make_event(id: &str, strand_path: &str) -> PendingEvent {
        PendingEvent {
            id: PendingEventId(id.to_string()),
            kind: "Created".to_string(),
            loom_id: "test-loom".to_string(),
            knot_id: "test-knot".to_string(),
            strand_path: strand_path.to_string(),
            queued_at: "2026-01-01T00:00:00+00:00".to_string(),
        }
    }

    /// `write_event` creates a file with correct JSON content using
    /// atomic write (temp then rename).
    #[test]
    fn write_event_creates_file_with_correct_json() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());
        let event = make_event("1000-aaaa", "/project/file.md");

        let result = store.write_event(&event);
        assert!(result.is_ok());

        // File should exist at the correct path
        let path = dir.path().join("1000-aaaa.json");
        assert!(path.exists());

        // Content should be valid JSON matching the event
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: PendingEvent = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.id.0, "1000-aaaa");
        assert_eq!(parsed.kind, "Created");
        assert_eq!(parsed.strand_path, "/project/file.md");

        // No temp file should remain
        let tmp_path = dir.path().join("1000-aaaa.json.tmp");
        assert!(!tmp_path.exists());
    }

    /// `write_event` creates parent directory if missing.
    #[test]
    fn write_event_creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("sub").join("events");
        assert!(!nested.exists());

        let store = FileSystemEventStore::new(nested.clone());
        let event = make_event("1000-aaaa", "/file.md");

        let result = store.write_event(&event);
        assert!(result.is_ok());
        assert!(nested.join("1000-aaaa.json").exists());
    }

    /// `remove_event` deletes the file.
    #[test]
    fn remove_event_deletes_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());
        let event = make_event("1000-aaaa", "/file.md");
        store.write_event(&event).unwrap();

        let id = PendingEventId("1000-aaaa".to_string());
        let result = store.remove_event(&id);
        assert!(result.is_ok());

        assert!(!dir.path().join("1000-aaaa.json").exists());
    }

    /// `remove_event` is a no-op for a missing file.
    #[test]
    fn remove_event_no_op_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());
        let id = PendingEventId("9999-zzzz".to_string());

        let result = store.remove_event(&id);
        assert!(result.is_ok(), "should not error on missing file");
    }

    /// `scan_events` returns an empty vec for an empty directory.
    #[test]
    fn scan_events_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());

        let events = store.scan_events().unwrap();
        assert!(events.is_empty());
    }

    /// `scan_events` returns events sorted by filename (FIFO order).
    #[test]
    fn scan_events_returns_sorted_events() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());

        // Write events out of order (by filename / timestamp)
        let e3 = make_event("1003-ccc", "/c.md");
        let e1 = make_event("1001-aaa", "/a.md");
        let e2 = make_event("1002-bbb", "/b.md");

        store.write_event(&e3).unwrap();
        store.write_event(&e1).unwrap();
        store.write_event(&e2).unwrap();

        let events = store.scan_events().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].id.0, "1001-aaa");
        assert_eq!(events[1].id.0, "1002-bbb");
        assert_eq!(events[2].id.0, "1003-ccc");
    }

    /// `scan_events` skips malformed JSON files (logs a warning).
    #[test]
    fn scan_events_skips_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());

        // Write a valid event
        let event = make_event("1001-aaa", "/a.md");
        store.write_event(&event).unwrap();

        // Write a malformed JSON file
        std::fs::write(
            dir.path().join("1002-bbb.json"),
            "not valid json {{{",
        )
        .unwrap();

        // Write another valid event
        let e3 = make_event("1003-ccc", "/c.md");
        store.write_event(&e3).unwrap();

        // scan_events should skip the malformed file and return valid ones
        let events = store.scan_events().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id.0, "1001-aaa");
        assert_eq!(events[1].id.0, "1003-ccc");
    }

    /// `scan_events` silently ignores non-JSON files.
    #[test]
    fn scan_events_skips_non_json_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());

        // Write a .md file (should be ignored)
        std::fs::write(dir.path().join("notes.md"), "some notes").unwrap();
        // Write a .tmp file (should be ignored)
        std::fs::write(dir.path().join("1000-aaaa.json.tmp"), "{}").unwrap();

        // Write a valid event
        let event = make_event("1001-aaa", "/a.md");
        store.write_event(&event).unwrap();

        let events = store.scan_events().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.0, "1001-aaa");
    }

    /// `read_event` reads and parses a single file correctly.
    #[test]
    fn read_event_parses_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());
        let event = make_event("1000-aaaa", "/project/file.md");
        store.write_event(&event).unwrap();

        let id = PendingEventId("1000-aaaa".to_string());
        let result = store.read_event(&id);
        assert!(result.is_ok());

        let parsed = result.unwrap();
        assert_eq!(parsed.id.0, "1000-aaaa");
        assert_eq!(parsed.strand_path, "/project/file.md");
        assert_eq!(parsed.kind, "Created");
    }

    /// `read_event` returns an error for a missing file.
    #[test]
    fn read_event_error_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());
        let id = PendingEventId("9999-zzzz".to_string());

        let result = store.read_event(&id);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }

    /// `event_count` returns the correct count of `.json` files.
    #[test]
    fn event_count_returns_correct_count() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());

        assert_eq!(store.event_count(), 0);

        store
            .write_event(&make_event("1001-aaa", "/a.md"))
            .unwrap();
        assert_eq!(store.event_count(), 1);

        store
            .write_event(&make_event("1002-bbb", "/b.md"))
            .unwrap();
        assert_eq!(store.event_count(), 2);

        // Add a non-json file — should not affect count
        std::fs::write(dir.path().join("notes.md"), "hi").unwrap();
        assert_eq!(store.event_count(), 2);
    }

    /// Round-trip: write 3 events, scan, verify all 3 returned in order.
    #[test]
    fn roundtrip_write_three_then_scan() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileSystemEventStore::new(dir.path().to_path_buf());

        let events = vec![
            make_event("1001-aaa", "/first.md"),
            make_event("1002-bbb", "/second.md"),
            make_event("1003-ccc", "/third.md"),
        ];

        for event in &events {
            store.write_event(event).unwrap();
        }

        let scanned = store.scan_events().unwrap();
        assert_eq!(scanned.len(), 3);
        assert_eq!(scanned[0].id.0, "1001-aaa");
        assert_eq!(scanned[0].strand_path, "/first.md");
        assert_eq!(scanned[1].id.0, "1002-bbb");
        assert_eq!(scanned[1].strand_path, "/second.md");
        assert_eq!(scanned[2].id.0, "1003-ccc");
        assert_eq!(scanned[2].strand_path, "/third.md");
    }
}
