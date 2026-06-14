use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::application::ports::{PortError, RigLogPort};
use crate::domain::events::RigLogEvent;

/// Filesystem-backed implementation of `RigLogPort`.
///
/// Writes rig-log events as JSONL (one JSON object per line) to
/// `<rig_dir>/.rig-log`. Uses append mode for concurrent write safety.
#[derive(Clone)]
pub struct FileSystemRigLog {
    rig_dir: PathBuf,
}

impl FileSystemRigLog {
    /// Create a new rig-log adapter backed by `rig_dir`.
    ///
    /// The log file lives at `<rig_dir>/.rig-log`.
    pub fn new(rig_dir: PathBuf) -> Self {
        Self { rig_dir }
    }

    /// Resolve the log file path.
    fn log_path(&self) -> PathBuf {
        self.rig_dir.join(".rig-log")
    }

    /// Open the log file for appending, creating parent directories as needed.
    fn open_append(&self) -> Result<std::fs::File, PortError> {
        let path = self.log_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| PortError::RigLogWriteFailed(e.to_string()))?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| PortError::RigLogWriteFailed(e.to_string()))
    }
}

impl RigLogPort for FileSystemRigLog {
    fn append(&self, event: RigLogEvent) -> Result<(), PortError> {
        let line = serde_json::to_string(&event)
            .map_err(|e| PortError::RigLogWriteFailed(e.to_string()))?;

        let mut file = self.open_append()?;
        writeln!(file, "{}", line)
            .map_err(|e| PortError::RigLogWriteFailed(e.to_string()))?;
        file.flush()
            .map_err(|e| PortError::RigLogWriteFailed(e.to_string()))?;
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<RigLogEvent>, PortError> {
        let path = self.log_path();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&path)
            .map_err(|e| PortError::RigLogReadFailed(e.to_string()))?;
        let reader = BufReader::new(file);

        let mut events = Vec::new();
        for line_result in reader.lines() {
            let line = line_result
                .map_err(|e| PortError::RigLogReadFailed(e.to_string()))?;
            if line.is_empty() {
                continue;
            }
            let event: RigLogEvent = serde_json::from_str(&line)
                .map_err(|e| PortError::RigLogReadFailed(e.to_string()))?;
            events.push(event);
        }

        Ok(events)
    }
}

/// Shared wrapper for concurrent append safety.
#[derive(Clone)]
pub struct SharedRigLog {
    inner: FileSystemRigLog,
    file: Arc<Mutex<std::fs::File>>,
}

impl SharedRigLog {
    /// Create a shared rig-log writer.
    pub fn new(rig_dir: PathBuf) -> Result<Self, PortError> {
        let inner = FileSystemRigLog::new(rig_dir.clone());
        let file = inner.open_append()?;
        Ok(Self { inner, file: Arc::new(Mutex::new(file)) })
    }

    /// Append an event using the shared file handle.
    pub fn append(&self, event: RigLogEvent) -> Result<(), PortError> {
        let line = serde_json::to_string(&event)
            .map_err(|e| PortError::RigLogWriteFailed(e.to_string()))?;

        let mut file = self.file.lock().unwrap();
        writeln!(file, "{}", line)
            .map_err(|e| PortError::RigLogWriteFailed(e.to_string()))?;
        file.flush()
            .map_err(|e| PortError::RigLogWriteFailed(e.to_string()))?;
        Ok(())
    }

    /// Read all events (delegates to inner).
    pub fn read_all(&self) -> Result<Vec<RigLogEvent>, PortError> {
        self.inner.read_all()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{KnotId, LoomId, StrandPath};
    use std::thread;

    #[test]
    fn rig_log_create_and_append() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemRigLog::new(dir.path().to_path_buf());

        // append creates the file and writes one line
        let event = RigLogEvent::QueueIdle {
            timestamp: "2026-06-14T10:00:00Z".to_string(),
        };
        let result = log.append(event);
        assert!(result.is_ok(), "append should succeed");

        // Verify the file exists
        let log_path = dir.path().join(".rig-log");
        assert!(log_path.exists(), "log file should exist");

        let content = fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1, "should have exactly one JSONL entry");

        // Verify it parses back correctly
        let parsed: RigLogEvent = serde_json::from_str(lines[0]).unwrap();
        match parsed {
            RigLogEvent::QueueIdle { timestamp } => {
                assert_eq!(timestamp, "2026-06-14T10:00:00Z");
            }
            _ => panic!("Expected QueueIdle event"),
        }
    }

    #[test]
    fn rig_log_append_timeout_exceeded() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemRigLog::new(dir.path().to_path_buf());

        let event = RigLogEvent::TimeoutExceeded {
            loom_id: LoomId("prds".to_string()),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
            error: "Agent session exceeded 60s deadline".to_string(),
            timestamp: "2026-06-14T10:00:00Z".to_string(),
        };
        log.append(event.clone()).unwrap();

        // Verify it reads back correctly
        let events = log.read_all().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], event);
    }

    #[test]
    fn rig_log_read_all() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemRigLog::new(dir.path().to_path_buf());

        // Append multiple events
        log.append(RigLogEvent::TimeoutExceeded {
            loom_id: LoomId("loom-1".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(PathBuf::from("in.md")),
            error: "timeout".to_string(),
            timestamp: "2026-06-14T10:00:00Z".to_string(),
        })
        .unwrap();
        log.append(RigLogEvent::QueueIdle {
            timestamp: "2026-06-14T10:01:00Z".to_string(),
        })
        .unwrap();
        log.append(RigLogEvent::TimeoutExceeded {
            loom_id: LoomId("loom-2".to_string()),
            knot_id: KnotId("k2".to_string()),
            strand_path: StrandPath(PathBuf::from("data.md")),
            error: "timeout 2".to_string(),
            timestamp: "2026-06-14T10:02:00Z".to_string(),
        })
        .unwrap();

        let events = log.read_all().unwrap();
        assert_eq!(events.len(), 3, "should return 3 events");

        match &events[0] {
            RigLogEvent::TimeoutExceeded { error, .. } => {
                assert_eq!(error, "timeout");
            }
            _ => panic!("first event should be TimeoutExceeded"),
        }
        match &events[1] {
            RigLogEvent::QueueIdle { .. } => {}
            _ => panic!("second event should be QueueIdle"),
        }
        match &events[2] {
            RigLogEvent::TimeoutExceeded { error, .. } => {
                assert_eq!(error, "timeout 2");
            }
            _ => panic!("third event should be TimeoutExceeded"),
        }
    }

    #[test]
    fn rig_log_read_all_empty() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemRigLog::new(dir.path().to_path_buf());

        // No events appended — should return empty vec
        let events = log.read_all().unwrap();
        assert!(
            events.is_empty(),
            "read_all on non-existent log should return empty vec"
        );
    }

    #[test]
    fn rig_log_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("deep").join("nested").join("rig");
        let log = FileSystemRigLog::new(nested);

        let event = RigLogEvent::QueueIdle {
            timestamp: "2026-06-14T10:00:00Z".to_string(),
        };
        let result = log.append(event);
        assert!(result.is_ok(), "append should create parent dirs");

        // Verify the file exists in the nested path
        let log_path = dir.path().join("deep/nested/rig/.rig-log");
        assert!(log_path.exists(), "log file should exist in nested path");
    }

    #[test]
    fn rig_log_concurrent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let shared = SharedRigLog::new(dir.path().to_path_buf()).unwrap();
        let shared = Arc::new(shared);

        let mut handles = Vec::new();
        for i in 0..20 {
            let shared = Arc::clone(&shared);
            let handle = thread::spawn(move || {
                let event = RigLogEvent::TimeoutExceeded {
                    loom_id: LoomId(format!("loom-{}", i % 5)),
                    knot_id: KnotId(format!("knot-{}", i)),
                    strand_path: StrandPath(PathBuf::from(format!("in{}.md", i))),
                    error: format!("timeout-{}", i),
                    timestamp: "2026-06-14T10:00:00Z".to_string(),
                };
                shared.append(event).unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // All 20 entries present — no data loss
        let events = shared.read_all().unwrap();
        assert_eq!(
            events.len(),
            20,
            "all 20 concurrent writes should be present"
        );

        // Each event is a valid TimeoutExceeded
        for event in &events {
            match event {
                RigLogEvent::TimeoutExceeded { knot_id, .. } => {
                    assert!(
                        knot_id.0.starts_with("knot-"),
                        "knot id should start with knot-"
                    );
                }
                _ => panic!("expected TimeoutExceeded event"),
            }
        }
    }

    #[test]
    fn rig_log_trait_object_safe() {
        let log = FileSystemRigLog::new(tempfile::tempdir().unwrap().path().to_path_buf());
        // Verify trait is object-safe
        let _obj: &dyn RigLogPort = &log;
    }
}
