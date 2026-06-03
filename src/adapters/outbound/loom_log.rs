use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::application::ports::{LoomLogPort, PortError};
use crate::domain::entities::LoomId;
use crate::domain::events::LoomEvent;

/// Filesystem-backed implementation of `LoomLogPort`.
///
/// Writes loom events as JSONL (one JSON object per line) to
/// `<base_dir>/<loom_id>/.loom-log`. Uses `Arc<Mutex<File>>` for
/// concurrent write safety.
#[derive(Clone)]
pub struct FileSystemLoomLog {
    base_dir: PathBuf,
}

impl FileSystemLoomLog {
    /// Create a new log adapter backed by `base_dir`.
    ///
    /// Log files live at `<base_dir>/<loom_id>/.loom-log`.
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Resolve the log file path for a given loom.
    fn log_path(&self, loom_id: &LoomId) -> PathBuf {
        self.base_dir.join(&loom_id.0).join(".loom-log")
    }

    /// Open the log file for appending, creating directories as needed.
    fn open_file(loom_id: &LoomId, base_dir: &PathBuf) -> Result<File, PortError> {
        let dir = base_dir.join(&loom_id.0);
        fs::create_dir_all(&dir)
            .map_err(|e| PortError::LoomLogOpenFailed(e.to_string()))?;
        let path = dir.join(".loom-log");
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))
    }
}

impl LoomLogPort for FileSystemLoomLog {
    fn open(&self, loom_id: &LoomId) -> Result<(), PortError> {
        // Ensure the loom directory and log file exist.
        let _file = Self::open_file(loom_id, &self.base_dir)?;
        Ok(())
    }

    fn append(&self, event: LoomEvent) -> Result<(), PortError> {
        // The loom_id is embedded in the event itself for JSONL storage.
        // We derive it from the event to find the correct log file.
        let loom_id = match &event {
            LoomEvent::KnotRegistered { loom_id, .. } => loom_id.clone(),
            LoomEvent::LoomStarted { loom_id } => loom_id.clone(),
            LoomEvent::LoomStopped { loom_id } => loom_id.clone(),
            LoomEvent::StrandProcessed { loom_id, .. } => loom_id.clone(),
        };

        let line = serde_json::to_string(&event)
            .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))?;

        let mut file = Self::open_file(&loom_id, &self.base_dir)?;
        writeln!(file, "{}", line)
            .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))?;
        file.flush()
            .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))?;
        Ok(())
    }

    fn read_all(&self, loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
        let path = self.log_path(loom_id);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&path)
            .map_err(|e| PortError::LoomLogReadFailed(e.to_string()))?;
        let reader = BufReader::new(file);

        let mut events = Vec::new();
        for line_result in reader.lines() {
            let line = line_result
                .map_err(|e| PortError::LoomLogReadFailed(e.to_string()))?;
            if line.is_empty() {
                continue;
            }
            let event: LoomEvent = serde_json::from_str(&line)
                .map_err(|e| PortError::LoomLogReadFailed(e.to_string()))?;
            events.push(event);
        }

        Ok(events)
    }
}

/// Shared wrapper for concurrent append safety.
#[derive(Clone)]
pub struct SharedLoomLog {
    inner: FileSystemLoomLog,
    file: Arc<Mutex<Option<File>>>,
    loom_id: LoomId,
}

impl SharedLoomLog {
    /// Create a shared log writer for a specific loom.
    pub fn new(base_dir: PathBuf, loom_id: LoomId) -> Result<Self, PortError> {
        let inner = FileSystemLoomLog::new(base_dir.clone());
        let file = Self::open_file(&loom_id, &base_dir)?;
        Ok(Self {
            inner,
            file: Arc::new(Mutex::new(Some(file))),
            loom_id,
        })
    }

    fn open_file(loom_id: &LoomId, base_dir: &PathBuf) -> Result<File, PortError> {
        let dir = base_dir.join(&loom_id.0);
        fs::create_dir_all(&dir)
            .map_err(|e| PortError::LoomLogOpenFailed(e.to_string()))?;
        let path = dir.join(".loom-log");
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))
    }

    /// Append an event using the shared file handle.
    pub fn append(&self, event: LoomEvent) -> Result<(), PortError> {
        let line = serde_json::to_string(&event)
            .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))?;

        let mut file = self.file.lock().unwrap();
        let file = file.as_mut().expect("file should be open");
        writeln!(file, "{}", line)
            .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))?;
        file.flush()
            .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))?;
        Ok(())
    }

    /// Read all events (delegates to inner).
    pub fn read_all(&self) -> Result<Vec<LoomEvent>, PortError> {
        self.inner.read_all(&self.loom_id)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{KnotId, StrandPath};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn loom_log_create_and_append() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().to_path_buf());
        let loom_id = LoomId("test-loom".to_string());

        // open creates the file
        let result = log.open(&loom_id);
        assert!(result.is_ok(), "open should succeed");

        // append writes one line
        let event = LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
        };
        let result = log.append(event);
        assert!(result.is_ok(), "append should succeed");

        // Verify the file has one JSONL entry
        let log_path = dir.path().join("test-loom/.loom-log");
        assert!(log_path.exists(), "log file should exist");
        let content = fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1, "should have exactly one JSONL entry");

        // Verify it parses back correctly
        let parsed: LoomEvent = serde_json::from_str(lines[0]).unwrap();
        match parsed {
            LoomEvent::LoomStarted { loom_id: ref lid } => {
                assert_eq!(*lid, loom_id);
            }
            _ => panic!("Expected LoomStarted event"),
        }
    }

    #[test]
    fn loom_log_read_all() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().to_path_buf());
        let loom_id = LoomId("read-loom".to_string());

        // Append 3 events
        log.append(LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
        })
        .unwrap();
        log.append(LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: KnotId("k1".to_string()),
        })
        .unwrap();
        log.append(LoomEvent::LoomStopped {
            loom_id: loom_id.clone(),
        })
        .unwrap();

        // read_all returns all 3 in order
        let events = log.read_all(&loom_id).unwrap();
        assert_eq!(events.len(), 3, "should return 3 events");

        match &events[0] {
            LoomEvent::LoomStarted { .. } => {}
            _ => panic!("first event should be LoomStarted"),
        }
        match &events[1] {
            LoomEvent::KnotRegistered {
                knot_id, ..
            } => {
                assert_eq!(knot_id, &KnotId("k1".to_string()));
            }
            _ => panic!("second event should be KnotRegistered"),
        }
        match &events[2] {
            LoomEvent::LoomStopped { .. } => {}
            _ => panic!("third event should be LoomStopped"),
        }
    }

    #[test]
    fn loom_log_multiple_events() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().to_path_buf());
        let loom_id = LoomId("multi-loom".to_string());

        // Append events of different types
        let knot_registered = LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: KnotId("review".to_string()),
        };
        let loom_started = LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
        };
        let strand_processed = LoomEvent::StrandProcessed {
            loom_id: loom_id.clone(),
            strand_path: StrandPath(PathBuf::from("doc.md")),
        };

        log.append(knot_registered.clone()).unwrap();
        log.append(loom_started.clone()).unwrap();
        log.append(strand_processed.clone()).unwrap();

        // All events preserved
        let events = log.read_all(&loom_id).unwrap();
        assert_eq!(events.len(), 3, "all 3 event types should be preserved");

        assert_eq!(events[0], knot_registered);
        assert_eq!(events[1], loom_started);
        assert_eq!(events[2], strand_processed);
    }

    #[test]
    fn loom_log_concurrent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let loom_id = LoomId("concurrent-loom".to_string());

        // Shared writer for concurrent access
        let shared = SharedLoomLog::new(dir.path().to_path_buf(), loom_id.clone())
            .unwrap();
        let shared = Arc::new(shared);

        let loom_id_clone = loom_id.clone();
        let mut handles = Vec::new();
        for i in 0..10 {
            let shared = Arc::clone(&shared);
            let loom_id = loom_id_clone.clone();
            let handle = thread::spawn(move || {
                let knot_id = KnotId(format!("knot-{}", i));
                let event = LoomEvent::KnotRegistered {
                    loom_id,
                    knot_id,
                };
                shared.append(event).unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // All 10 entries present — no data loss
        let events = shared.read_all().unwrap();
        assert_eq!(
            events.len(),
            10,
            "all 10 concurrent writes should be present"
        );

        // Each event is a valid KnotRegistered
        for event in &events {
            match event {
                LoomEvent::KnotRegistered { knot_id, .. } => {
                    assert!(
                        knot_id.0.starts_with("knot-"),
                        "knot id should start with knot-"
                    );
                }
                _ => panic!("expected KnotRegistered event"),
            }
        }
    }

    #[test]
    fn loom_log_read_all_empty() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().to_path_buf());
        let loom_id = LoomId("empty-loom".to_string());

        // No events appended — should return empty vec
        let events = log.read_all(&loom_id).unwrap();
        assert!(
            events.is_empty(),
            "read_all on non-existent log should return empty vec"
        );
    }

    #[test]
    fn loom_log_trait_object_safe() {
        let log = FileSystemLoomLog::new(tempfile::tempdir().unwrap().path().to_path_buf());
        // Verify trait is object-safe
        let _obj: &dyn LoomLogPort = &log;
    }
}
