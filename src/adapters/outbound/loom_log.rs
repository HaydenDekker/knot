use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::application::ports::{LoomLogPort, PortError};
use crate::domain::entities::LoomId;
use crate::domain::events::LoomEvent;
use crate::domain::knot_file::{derive_loom_log_path, derive_runtime_root};

/// Filesystem-backed implementation of `LoomLogPort`.
///
/// Writes loom events as JSONL (one JSON object per line) to
/// `tie-offs/<rig-basename>/<loom_id>/.loom-log` (under the rig's runtime
/// root). Uses `Arc<Mutex<File>>` for concurrent write safety.
#[derive(Clone)]
pub struct FileSystemLoomLog {
    rig_dir: PathBuf,
}

impl FileSystemLoomLog {
    /// Create a new log adapter backed by `rig_dir`.
    ///
    /// Log files live at the rig's runtime root:
    /// `<project-root>/tie-offs/<rig-basename>/<loom_id>/.loom-log`.
    pub fn new(rig_dir: PathBuf) -> Self {
        Self { rig_dir }
    }

    /// Resolve the log file path for a given loom.
    fn log_path(&self, loom_id: &LoomId) -> PathBuf {
        derive_loom_log_path(&loom_id.0, &self.rig_dir)
    }

    /// Open the log file for appending, creating directories as needed.
    fn open_file(loom_id: &LoomId, rig_dir: &std::path::Path) -> Result<File, PortError> {
        let path = derive_loom_log_path(&loom_id.0, rig_dir);
        let dir = path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&dir)
            .map_err(|e| PortError::LoomLogOpenFailed(e.to_string()))?;
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
        let _file = Self::open_file(loom_id, &self.rig_dir)?;
        Ok(())
    }

    fn append(&self, event: LoomEvent) -> Result<(), PortError> {
        // The loom_id is embedded in the event itself for JSONL storage.
        // We derive it from the event to find the correct log file.
        let loom_id = match &event {
            LoomEvent::KnotRegistered { loom_id, .. } => loom_id.clone(),
            LoomEvent::LoomStarted { loom_id, .. } => loom_id.clone(),
            LoomEvent::LoomStopped { loom_id, .. } => loom_id.clone(),
            LoomEvent::StrandProcessed { loom_id, .. } => loom_id.clone(),
            LoomEvent::KnotProcessing { loom_id, .. } => loom_id.clone(),
            LoomEvent::KnotCompleted { loom_id, .. } => loom_id.clone(),
            LoomEvent::KnotFailed { loom_id, .. } => loom_id.clone(),
            LoomEvent::KnotDeregistered { loom_id, .. } => {
                loom_id.clone()
            }
            LoomEvent::KnotParseWarning { loom_id, .. } => {
                loom_id.clone()
            }
            LoomEvent::DirectoryCreated { loom_id, .. } => {
                loom_id.clone()
            }
            LoomEvent::StrandIgnored { loom_id, .. } => {
                loom_id.clone()
            }
            LoomEvent::StrandSkipped { loom_id, .. } => {
                loom_id.clone()
            }
            LoomEvent::SessionResumed { loom_id, .. } => {
                loom_id.clone()
            }
            LoomEvent::KnotEmptyResponse { loom_id, .. } => {
                loom_id.clone()
            }
            LoomEvent::EventsDispatched { loom_id, .. } => {
                loom_id.clone()
            }
            LoomEvent::KnotEventsMissing { loom_id, .. } => {
                loom_id.clone()
            }
        };

        let line = serde_json::to_string(&event)
            .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))?;

        let mut file = Self::open_file(&loom_id, &self.rig_dir)?;
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
        let mut line_num = 0u64;
        for line_result in reader.lines() {
            line_num += 1;
            let line = line_result
                .map_err(|e| PortError::LoomLogReadFailed(e.to_string()))?;
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<LoomEvent>(&line) {
                Ok(event) => events.push(event),
                Err(e) => {
                    // Skip non-JSONL lines (e.g. agent-written text that
                    // accidentally ended up in the log file) instead of
                    // aborting the entire read. Log a warning so the issue
                    // is visible in output.
                    eprintln!(
                        "WARN: loom-log {} line {}: skipping non-JSONL content: {}",
                        loom_id.0,
                        line_num,
                        e,
                    );
                }
            }
        }

        Ok(events)
    }

    fn clear_all(&self) -> Result<(), PortError> {
        // Enumerate runtime-root subdirectories (one per loom, including
        // orphans) and truncate files named `.loom-log` directly inside
        // them. Never descend: dispatch dirs, tie-off files, and other
        // runtime artifacts are left untouched.
        let runtime_root = derive_runtime_root(&self.rig_dir);
        let entries = match fs::read_dir(&runtime_root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Fresh rig — no runtime root yet, nothing to clear.
                return Ok(());
            }
            Err(e) => {
                return Err(PortError::LoomLogReadFailed(e.to_string()));
            }
        };
        for entry in entries {
            let entry =
                entry.map_err(|e| PortError::LoomLogReadFailed(e.to_string()))?;
            let loom_dir = entry.path();
            if !loom_dir.is_dir() {
                continue;
            }
            let log_file = loom_dir.join(".loom-log");
            if log_file.is_file() {
                // Truncate in place (keeps file identity stable).
                fs::File::create(&log_file)
                    .map_err(|e| PortError::LoomLogAppendFailed(e.to_string()))?;
            }
        }
        Ok(())
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
    pub fn new(rig_dir: PathBuf, loom_id: LoomId) -> Result<Self, PortError> {
        let inner = FileSystemLoomLog::new(rig_dir.clone());
        let file = Self::open_file(&loom_id, &rig_dir)?;
        Ok(Self {
            inner,
            file: Arc::new(Mutex::new(Some(file))),
            loom_id,
        })
    }

    fn open_file(loom_id: &LoomId, rig_dir: &std::path::Path) -> Result<File, PortError> {
        let path = derive_loom_log_path(&loom_id.0, rig_dir);
        let dir = path.parent().unwrap().to_path_buf();
        fs::create_dir_all(&dir)
            .map_err(|e| PortError::LoomLogOpenFailed(e.to_string()))?;
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
        let rig_dir = dir.path().join("rig");
        let log = FileSystemLoomLog::new(rig_dir);
        let loom_id = LoomId("test-loom".to_string());

        // open creates the file
        let result = log.open(&loom_id);
        assert!(result.is_ok(), "open should succeed");

        // append writes one line
        let event = LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        };
        let result = log.append(event);
        assert!(result.is_ok(), "append should succeed");

        // Verify the file has one JSONL entry at the runtime root
        // (tie-offs/<rig-basename>/{loom-id}/.loom-log)
        let log_path = dir.path().join("tie-offs/rig/test-loom/.loom-log");
        assert!(log_path.exists(), "log file should exist");
        let content = fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1, "should have exactly one JSONL entry");

        // Verify it parses back correctly
        let parsed: LoomEvent = serde_json::from_str(lines[0]).unwrap();
        match parsed {
            LoomEvent::LoomStarted { loom_id: ref lid, .. } => {
                assert_eq!(*lid, loom_id);
            }
            _ => panic!("Expected LoomStarted event"),
        }
    }

    #[test]
    fn loom_log_read_all() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        let log = FileSystemLoomLog::new(rig_dir);
        let loom_id = LoomId("read-loom".to_string());

        // Append 3 events
        log.append(LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        })
        .unwrap();
        log.append(LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: KnotId("k1".to_string()),
            timestamp: "2026-06-10T12:00:01Z".to_string(),
        })
        .unwrap();
        log.append(LoomEvent::LoomStopped {
            loom_id: loom_id.clone(),
            timestamp: "2026-06-10T12:00:02Z".to_string(),
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
        let rig_dir = dir.path().join("rig");
        let log = FileSystemLoomLog::new(rig_dir);
        let loom_id = LoomId("multi-loom".to_string());

        // Append events of different types
        let knot_registered = LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: KnotId("review".to_string()),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        };
        let loom_started = LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        };
        let strand_processed = LoomEvent::StrandProcessed {
            loom_id: loom_id.clone(),
            strand_path: StrandPath(PathBuf::from("doc.md")),
            error: None,
            timestamp: "2026-06-10T12:00:00Z".to_string(),
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
        let shared = SharedLoomLog::new(dir.path().join("rig"), loom_id.clone())
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
                    timestamp: "2026-06-10T12:00:00Z".to_string(),
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
        let rig_dir = dir.path().join("rig");
        let log = FileSystemLoomLog::new(rig_dir);
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
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().join("rig"));
        // Verify trait is object-safe
        let _obj: &dyn LoomLogPort = &log;
    }

    #[test]
    fn loom_log_strand_skipped_routes_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        let log = FileSystemLoomLog::new(rig_dir);
        let loom_id = LoomId("skip-loom".to_string());

        let event = LoomEvent::StrandSkipped {
            loom_id: loom_id.clone(),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/sedABC123")),
            reason: "missing file (unknown pattern)".to_string(),
            timestamp: "2026-06-24T10:00:00Z".to_string(),
        };

        // Append — should route to correct loom log
        log.append(event.clone()).unwrap();

        // Read back and verify
        let events = log.read_all(&loom_id).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::StrandSkipped {
                loom_id: lid,
                strand_path,
                reason,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(
                    strand_path.0,
                    PathBuf::from("project/sedABC123")
                );
                assert_eq!(reason, "missing file (unknown pattern)");
            }
            _ => panic!("Expected StrandSkipped event"),
        }
    }

    #[test]
    fn loom_log_knot_empty_response_routes_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        let log = FileSystemLoomLog::new(rig_dir);
        let loom_id = LoomId("empty-loom".to_string());

        let event = LoomEvent::KnotEmptyResponse {
            loom_id: loom_id.clone(),
            knot_id: KnotId("review".to_string()),
            strand_path: StrandPath(PathBuf::from("project/prds/my-prd.md")),
            attempt: 2,
            timestamp: "2026-06-28T15:00:00Z".to_string(),
        };

        // Append — should route to correct loom log
        log.append(event.clone()).unwrap();

        // Read back and verify
        let events = log.read_all(&loom_id).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::KnotEmptyResponse {
                loom_id: lid,
                strand_path,
                attempt,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(
                    strand_path.0,
                    PathBuf::from("project/prds/my-prd.md")
                );
                assert_eq!(*attempt, 2);
            }
            _ => panic!("Expected KnotEmptyResponse event"),
        }
    }

    /// Non-JSONL lines in the log file are skipped gracefully instead
    /// of aborting the entire read.
    #[test]
    fn loom_log_read_all_skips_non_jsonl_lines() {
        let dir = tempfile::tempdir().unwrap();
        let loom_id = LoomId("dirty-loom".to_string());

        // Create a log file with mixed content: valid JSONL lines
        // interleaved with non-JSON text (e.g. agent accident).
        let log_path =
            dir.path().join("tie-offs/rig/dirty-loom/.loom-log");
        fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        let content = concat!(
            "# Tie-off: agent wrote markdown here\n",
            "\n",
            "**This is not JSON**\n",
            "{\"KnotRegistered\":{\"loom_id\":\"dirty-loom\",\"knot_id\":\"k1\",\"timestamp\":\"2026-07-10T10:00:00Z\"}}\n",
            "More non-json text\n",
            "{\"KnotCompleted\":{\"loom_id\":\"dirty-loom\",\"knot_id\":\"k1\",\"strand_path\":\"input.md\",\"tie_off_path\":\"out.md\",\"timestamp\":\"2026-07-10T10:05:00Z\"}}\n",
        );
        fs::write(&log_path, content).unwrap();

        let log = FileSystemLoomLog::new(dir.path().join("rig"));
        let events = log.read_all(&loom_id).unwrap();

        // Should have 2 valid events, non-JSON lines skipped
        assert_eq!(events.len(), 2, "should skip non-JSONL lines");
        match &events[0] {
            LoomEvent::KnotRegistered { knot_id, .. } => {
                assert_eq!(knot_id.0, "k1");
            }
            _ => panic!("first event should be KnotRegistered"),
        }
        match &events[1] {
            LoomEvent::KnotCompleted { knot_id, .. } => {
                assert_eq!(knot_id.0, "k1");
            }
            _ => panic!("second event should be KnotCompleted"),
        }
    }

    /// A legacy 3-tuple `EventsDispatched` line (written by Knot < 0.33.0,
    /// before the created-file-path element) fails to deserialize into the
    /// 4-tuple and is skipped with a warning — while subsequent lines read
    /// fine. Pins the 0.30.1-style graceful degradation: warning noise
    /// only, no data loss.
    #[test]
    fn loom_log_read_all_skips_legacy_3tuple_events_dispatched() {
        let dir = tempfile::tempdir().unwrap();
        let loom_id = LoomId("legacy-loom".to_string());

        let log_path =
            dir.path().join("tie-offs/rig/legacy-loom/.loom-log");
        fs::create_dir_all(log_path.parent().unwrap()).unwrap();

        // Line 1: legacy 3-tuple EventsDispatched (old shape)
        // Line 2: current 4-tuple EventsDispatched
        // Line 3: a normal event after the legacy line
        let content = concat!(
            "{\"EventsDispatched\":{\"loom_id\":\"legacy-loom\",\"knot_id\":\"k1\",\"strand_path\":\"in.md\",\"dispatches\":[[\"PlanCreated\",\"watcher\",\"consumer-loom\"]],\"timestamp\":\"2026-07-10T10:00:00Z\"}}\n",
            "{\"EventsDispatched\":{\"loom_id\":\"legacy-loom\",\"knot_id\":\"k1\",\"strand_path\":\"in.md\",\"dispatches\":[[\"PlanCreated\",\"watcher\",\"consumer-loom\",\"/tie-offs/rig/consumer-loom/PlanCreated/event-2026-07-10T10-00-01Z.md\"]],\"timestamp\":\"2026-08-22T21:54:49+01:00\"}}\n",
            "{\"KnotCompleted\":{\"loom_id\":\"legacy-loom\",\"knot_id\":\"k1\",\"strand_path\":\"in.md\",\"tie_off_path\":\"out.md\",\"timestamp\":\"2026-08-22T21:54:50+01:00\"}}\n",
        );
        fs::write(&log_path, content).unwrap();

        let log = FileSystemLoomLog::new(dir.path().join("rig"));
        let events = log.read_all(&loom_id).unwrap();

        // The legacy line is skipped; the two current-shape lines survive
        assert_eq!(
            events.len(),
            2,
            "legacy 3-tuple line must be skipped, later lines must read"
        );

        match &events[0] {
            LoomEvent::EventsDispatched { dispatches, .. } => {
                assert_eq!(dispatches.len(), 1);
                assert_eq!(dispatches[0].0, "PlanCreated");
                assert!(
                    dispatches[0].3.ends_with("event-2026-07-10T10-00-01Z.md"),
                    "4th element must carry the created file path: {}",
                    dispatches[0].3
                );
            }
            other => panic!("first event should be EventsDispatched, got {other:?}"),
        }
        match &events[1] {
            LoomEvent::KnotCompleted { .. } => {}
            other => panic!(
                "second event should be KnotCompleted (read after the \
                 skipped legacy line), got {other:?}"
            ),
        }
    }

    /// `clear_all()` truncates every `*/.loom-log` under the runtime root
    /// in place: prior-run events are gone from all looms, the files
    /// still exist (empty), and appends after the clear work.
    #[test]
    fn loom_log_clear_all_truncates_every_loom_log() {
        let dir = tempfile::tempdir().unwrap();
        // Adapter is constructed with the rig dir; the runtime root is
        // derived as <parent>/tie-offs/<rig-basename>/.
        let log = FileSystemLoomLog::new(dir.path().join("rig"));
        let runtime_root = dir.path().join("tie-offs").join("rig");

        let loom_a = LoomId("loom-a".to_string());
        let loom_b = LoomId("loom-b".to_string());
        log.append(LoomEvent::LoomStarted {
            loom_id: loom_a.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        })
        .unwrap();
        log.append(LoomEvent::LoomStopped {
            loom_id: loom_a.clone(),
            timestamp: "2026-06-10T12:30:00Z".to_string(),
        })
        .unwrap();
        log.append(LoomEvent::LoomStarted {
            loom_id: loom_b.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        })
        .unwrap();
        assert_eq!(log.read_all(&loom_a).unwrap().len(), 2);
        assert_eq!(log.read_all(&loom_b).unwrap().len(), 1);

        log.clear_all().unwrap();

        // Both loom-logs emptied, files still exist (truncated, not deleted)
        assert!(
            log.read_all(&loom_a).unwrap().is_empty(),
            "loom-a log must be emptied"
        );
        assert!(
            log.read_all(&loom_b).unwrap().is_empty(),
            "loom-b log must be emptied"
        );
        for loom in [&loom_a, &loom_b] {
            let path = runtime_root.join(loom.0.as_str()).join(".loom-log");
            assert!(path.exists(), "clear_all truncates, does not delete");
            assert_eq!(fs::read_to_string(&path).unwrap(), "");
        }

        // Appends after the clear work and read back
        log.append(LoomEvent::LoomStarted {
            loom_id: loom_a.clone(),
            timestamp: "2026-06-11T09:00:00Z".to_string(),
        })
        .unwrap();
        assert_eq!(log.read_all(&loom_a).unwrap().len(), 1);
        assert!(log.read_all(&loom_b).unwrap().is_empty());
    }

    /// `clear_all()` clears loom-logs for looms that no longer exist in
    /// the rig directory (orphaned runtime dirs) — enumeration is over
    /// the runtime root, not over discovered looms.
    #[test]
    fn loom_log_clear_all_includes_orphan_looms() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().join("rig"));
        let runtime_root = dir.path().join("tie-offs").join("rig");

        // An orphaned loom dir with a prior-run log and no knot content
        let orphan_log = runtime_root.join("old-loom").join(".loom-log");
        fs::create_dir_all(orphan_log.parent().unwrap()).unwrap();
        fs::write(&orphan_log, "orphan prior-run line\n").unwrap();

        // A live loom with a prior-run log
        let live = LoomId("live-loom".to_string());
        log.append(LoomEvent::LoomStarted {
            loom_id: live.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        })
        .unwrap();

        log.clear_all().unwrap();

        assert!(
            log.read_all(&live).unwrap().is_empty(),
            "live loom log must be emptied"
        );
        assert_eq!(
            fs::read_to_string(&orphan_log).unwrap(),
            "",
            "orphaned loom log must be emptied too"
        );
    }

    /// `clear_all()` touches only files named `.loom-log` in runtime-root
    /// subdirectories: tie-off files, dispatch-dir files, `state.json`,
    /// and `events/` contents are byte-identical afterwards.
    #[test]
    fn loom_log_clear_all_leaves_other_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let log = FileSystemLoomLog::new(dir.path().join("rig"));
        let runtime_root = dir.path().join("tie-offs").join("rig");

        let loom = LoomId("keep-loom".to_string());
        let loom_dir = runtime_root.join("keep-loom");
        // Prior-run loom-log (will be cleared)
        log.append(LoomEvent::LoomStarted {
            loom_id: loom.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        })
        .unwrap();
        // Files that must survive byte-identical
        let tie_off = loom_dir.join("tie-off.md");
        fs::write(&tie_off, "durable tie-off\n").unwrap();
        let dispatch_dir = loom_dir.join("KnotCompleted");
        fs::create_dir_all(&dispatch_dir).unwrap();
        let dispatch_file = dispatch_dir.join("event-1.md");
        fs::write(&dispatch_file, "pending dispatch\n").unwrap();
        let state = runtime_root.join("state.json");
        fs::write(&state, "{\"looms\":[]}\n").unwrap();
        let events_dir = runtime_root.join("events");
        fs::create_dir_all(&events_dir).unwrap();
        let event_q = events_dir.join("q1.json");
        fs::write(&event_q, "{\"queued\":true}\n").unwrap();
        // A file named `.loom-log` one level deeper (dispatch-dir level)
        // must NOT be touched — only top-level `*/.loom-log`.
        let deep_log = dispatch_dir.join(".loom-log");
        fs::write(&deep_log, "deep\n").unwrap();

        log.clear_all().unwrap();

        assert!(log.read_all(&loom).unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(&tie_off).unwrap(),
            "durable tie-off\n"
        );
        assert_eq!(
            fs::read_to_string(&dispatch_file).unwrap(),
            "pending dispatch\n"
        );
        assert_eq!(
            fs::read_to_string(&state).unwrap(),
            "{\"looms\":[]}\n"
        );
        assert_eq!(
            fs::read_to_string(&event_q).unwrap(),
            "{\"queued\":true}\n"
        );
        assert_eq!(fs::read_to_string(&deep_log).unwrap(), "deep\n");
    }

    /// `clear_all()` with a missing runtime root (fresh rig, nothing
    /// ever written) is a no-op returning `Ok` — first-run startup must
    /// not fail.
    #[test]
    fn loom_log_clear_all_missing_root_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        // Rig dir never created → runtime root tie-offs/<rig> missing
        let log = FileSystemLoomLog::new(dir.path().join("never-created-rig"));
        assert!(
            log.clear_all().is_ok(),
            "clear_all on a missing runtime root must be a no-op"
        );
    }
}
