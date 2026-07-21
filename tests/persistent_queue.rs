//! Integration tests for the persistent (disk-backed) event queue.
//!
//! Verifies the full persistence lifecycle: events are written as JSON files
//! in `rig/events/`, survive queue recreation (restart simulation), and are
//! removed after processing. Uses `DiskBackedEventQueue` directly with
//! `tempfile` for isolated filesystem state.
//!
//! The disk **is** the queue — `.json` files in the events directory are the
//! source of truth. These tests verify that file-level operations match the
//! expected persistence semantics.

use knot::adapters::outbound::DiskBackedEventQueue;
use knot::application::ports::StrandEventQueue;
use knot::domain::pending_event::{
    PendingEvent, PendingEventId, PendingEventOrShutdown,
};
use std::fs;
use std::path::PathBuf;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Build a `PendingEvent` with a fixed ID for deterministic ordering.
fn make_pending_event(
    id: &str,
    kind: &str,
    loom_id: &str,
    knot_id: &str,
    strand_path: &str,
) -> PendingEvent {
    PendingEvent {
        id: PendingEventId(id.to_string()),
        kind: kind.to_string(),
        loom_id: loom_id.to_string(),
        knot_id: knot_id.to_string(),
        strand_path: strand_path.to_string(),
        queued_at: "2026-07-21T10:00:00+00:00".to_string(),
    }
}

/// Return the events directory for a queue (via debug formatting).
///
/// `DiskBackedEventQueue` stores the path internally. We derive it from
/// the temp dir passed at construction — tests own the temp dir directly.
fn event_file_exists(events_dir: &PathBuf, id: &PendingEventId) -> bool {
    events_dir.join(format!("{}.json", id.0)).exists()
}

/// Count `.json` files in the events directory.
fn count_event_files(events_dir: &PathBuf) -> usize {
    fs::read_dir(events_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
                .count()
        })
        .unwrap_or(0)
}

// ── Full Persistence Cycle ──────────────────────────────────────────────

/// Full cycle: push event → verify file exists on disk → pop event → verify
/// file removed from disk.
///
/// This verifies the core persistence contract: every push writes a `.json`
/// file, and every pop removes it.
#[test]
fn full_cycle_push_pop_disk_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let events_dir = tmp.path().to_path_buf();
    let queue = DiskBackedEventQueue::new(events_dir.clone());

    // Push an event
    let event = make_pending_event(
        "1000-aaaa",
        "Created",
        "review-loom",
        "review",
        "/project/strands/feature.md",
    );
    let id = queue.push(event.clone());

    // Verify file exists on disk
    assert!(
        event_file_exists(&events_dir, &id),
        "event file should exist on disk after push"
    );
    assert_eq!(
        count_event_files(&events_dir),
        1,
        "should have exactly 1 event file"
    );

    // Pop the event
    let result = queue.pop();
    assert!(
        matches!(result, Some(PendingEventOrShutdown::Event(_))),
        "pop should return the event"
    );

    // Verify file removed from disk
    assert!(
        !event_file_exists(&events_dir, &id),
        "event file should be removed from disk after pop"
    );
    assert_eq!(
        count_event_files(&events_dir),
        0,
        "should have 0 event files after pop"
    );

    // Queue is empty
    assert!(queue.is_empty());
    assert!(queue.pop().is_none());
}

// ── Restart Survival ────────────────────────────────────────────────────

/// Push events, drop the queue (simulate crash), recreate queue → verify
/// persisted events are loaded and processed.
///
/// Simulates: Knot pushes events → process stops (Ctrl+C/crash) → Knot
/// restarts → events are recovered from `rig/events/` and processed.
#[test]
fn restart_survival_events_load_after_recreation() {
    let tmp = tempfile::tempdir().unwrap();
    let events_dir = tmp.path().to_path_buf();

    // First "run": push events but don't pop (simulate crash before processing)
    {
        let queue = DiskBackedEventQueue::new(events_dir.clone());

        let e1 = make_pending_event(
            "1001-aaaa",
            "Created",
            "review-loom",
            "review",
            "/project/strands/feature-a.md",
        );
        let e2 = make_pending_event(
            "1002-bbbb",
            "Modified",
            "review-loom",
            "review",
            "/project/strands/feature-b.md",
        );
        let e3 = make_pending_event(
            "1003-cccc",
            "Created",
            "build-loom",
            "checker",
            "/project/src/lib.rs",
        );

        queue.push(e1);
        queue.push(e2);
        queue.push(e3);

        assert_eq!(queue.len(), 3, "should have 3 events before crash");
        assert_eq!(count_event_files(&events_dir), 3, "should have 3 files on disk");
        // Drop the queue — simulates process death
    }

    // Verify files still exist after "crash"
    assert_eq!(
        count_event_files(&events_dir),
        3,
        "files should still exist on disk after queue drop"
    );

    // Second "run": recreate queue pointing at same directory
    {
        let queue = DiskBackedEventQueue::new(events_dir.clone());

        // Events should be visible on disk (queue reads from disk, not memory)
        assert_eq!(
            queue.len(),
            3,
            "new queue should see 3 events on disk"
        );

        // load_persisted deduplicates from disk
        let loaded = queue.load_persisted();
        // Since files are already on disk, load_persisted scans them and
        // pushes through push_or_replace. Since they already exist with
        // the same IDs and dedup keys, they get replaced but the count
        // reflects what was found on disk before dedup.
        assert_eq!(
            loaded, 3,
            "should have loaded 3 persisted events"
        );

        // After load_persisted, files may have been deduped (same IDs
        // replaced). The queue still has 3 unique events.
        assert_eq!(queue.len(), 3, "should still have 3 events");

        // Process all 3 events
        for i in 1..=3 {
            let result = queue.pop().unwrap();
            match result {
                PendingEventOrShutdown::Event(e) => {
                    assert!(
                        !e.strand_path.is_empty(),
                        "event {} should have strand path",
                        i
                    );
                }
                PendingEventOrShutdown::Shutdown => {
                    panic!("unexpected shutdown sentinel");
                }
            }
        }

        // All events processed — disk is empty
        assert!(
            queue.is_empty(),
            "queue should be empty after processing all events"
        );
        assert_eq!(
            count_event_files(&events_dir),
            0,
            "disk should be empty after processing all events"
        );
    }
}

// ── Malformed File Handling ──────────────────────────────────────────────

/// Write invalid JSON to `rig/events/`, scan → verify warning logged,
/// processing continues with valid events.
///
/// Malformed files (corrupted JSON, partial writes) must not crash the
/// queue. They should be skipped with a warning, and valid events should
/// still be processed.
#[test]
fn malformed_file_handling_skips_bad_json() {
    let tmp = tempfile::tempdir().unwrap();
    let events_dir = tmp.path().to_path_buf();

    // Write a malformed JSON file directly to disk
    fs::write(
        events_dir.join("9990-bad1.json"),
        "not valid json {{{",
    )
    .unwrap();

    // Write another malformed file (truncated)
    fs::write(
        events_dir.join("9991-bad2.json"),
        r#"{"id":"9991-bad2","kind":"Created"#,
    )
    .unwrap();

    // Write a valid event
    let queue = DiskBackedEventQueue::new(events_dir.clone());
    let good_event = make_pending_event(
        "9992-good",
        "Created",
        "review-loom",
        "review",
        "/project/strands/feature.md",
    );
    queue.push(good_event.clone());

    // Scan should return only the valid event
    let snapshot = queue.snapshot();
    assert_eq!(
        snapshot.len(),
        1,
        "should have 1 valid event (malformed files skipped)"
    );
    assert_eq!(snapshot[0].id.0, "9992-good");
    assert_eq!(snapshot[0].strand_path, "/project/strands/feature.md");

    // Pop should return the valid event
    let result = queue.pop();
    match result {
        Some(PendingEventOrShutdown::Event(e)) => {
            assert_eq!(e.id.0, "9992-good");
        }
        _ => panic!("expected valid event, got {:?}", result),
    }

    // Malformed files still exist on disk (not auto-cleaned)
    assert!(
        events_dir.join("9990-bad1.json").exists(),
        "malformed files are not auto-deleted"
    );
    assert!(
        events_dir.join("9991-bad2.json").exists(),
        "malformed files are not auto-deleted"
    );
}

// ── Empty Events Directory ───────────────────────────────────────────────

/// Clean restart with no events → no events loaded, clean startup.
///
/// Verifies that starting with an empty (or freshly created) events
/// directory does not cause errors — the queue starts clean.
#[test]
fn empty_events_directory_clean_startup() {
    let tmp = tempfile::tempdir().unwrap();
    let events_dir = tmp.path().to_path_buf();

    // Create queue pointing at empty directory
    let queue = DiskBackedEventQueue::new(events_dir.clone());

    // Directory should exist (created by constructor)
    assert!(
        events_dir.exists(),
        "events directory should be created by constructor"
    );
    assert!(
        events_dir.is_dir(),
        "events directory should be a directory"
    );

    // Queue should be empty
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
    assert!(queue.pop().is_none());
    assert!(queue.snapshot().is_empty());

    // load_persisted should return 0
    let loaded = queue.load_persisted();
    assert_eq!(loaded, 0, "should load 0 events from empty directory");

    // push_shutdown on empty queue returns Shutdown
    queue.push_shutdown();
    let result = queue.pop();
    assert!(
        matches!(result, Some(PendingEventOrShutdown::Shutdown)),
        "shutdown on empty queue should return Shutdown"
    );
}

// ── Multiple Events ──────────────────────────────────────────────────────

/// Push 5 events → verify 5 files exist → process all → verify 0 files
/// remain.
///
/// Verifies that the queue handles multiple concurrent events correctly,
/// maintaining FIFO order and cleaning up all files after processing.
#[test]
fn multiple_events_five_pushed_processed() {
    let tmp = tempfile::tempdir().unwrap();
    let events_dir = tmp.path().to_path_buf();
    let queue = DiskBackedEventQueue::new(events_dir.clone());

    // Push 5 events with fixed IDs for deterministic ordering
    let events: Vec<PendingEvent> = (1..=5)
        .map(|i| {
            make_pending_event(
                format!("100{}-aaaa", i).as_str(),
                "Created",
                "review-loom",
                "review",
                &format!("/project/strands/feature-{}.md", i),
            )
        })
        .collect();

    for event in &events {
        queue.push(event.clone());
    }

    // Verify 5 files exist
    assert_eq!(
        queue.len(),
        5,
        "should have 5 events in queue"
    );
    assert_eq!(
        count_event_files(&events_dir),
        5,
        "should have 5 files on disk"
    );

    // Process all 5 events (FIFO order)
    for i in 0..5 {
        let result = queue.pop();
        match result {
            Some(PendingEventOrShutdown::Event(e)) => {
                assert_eq!(
                    e.strand_path,
                    events[i].strand_path,
                    "event {} should match expected path",
                    i + 1
                );
            }
            _ => panic!(
                "expected event {}, got {:?}",
                i + 1,
                result
            ),
        }
    }

    // All processed — disk is empty
    assert_eq!(
        queue.len(),
        0,
        "queue should be empty after processing 5 events"
    );
    assert_eq!(
        count_event_files(&events_dir),
        0,
        "should have 0 files on disk after processing all events"
    );
}

// ── Delete From Queue ────────────────────────────────────────────────────

/// Push event, call `queue.delete(id)` → verify event removed from queue
/// and disk, not processed.
///
/// Verifies that events can be cancelled before processing — both the
/// queue entry and its disk file are removed.
#[test]
fn delete_from_queue_removes_event() {
    let tmp = tempfile::tempdir().unwrap();
    let events_dir = tmp.path().to_path_buf();
    let queue = DiskBackedEventQueue::new(events_dir.clone());

    // Push an event
    let event = make_pending_event(
        "1000-aaaa",
        "Created",
        "review-loom",
        "review",
        "/project/strands/feature.md",
    );
    let id = queue.push(event);

    // Verify event exists
    assert_eq!(queue.len(), 1);
    assert!(event_file_exists(&events_dir, &id));

    // Delete the event
    let deleted = queue.delete(&id);
    assert!(deleted, "delete should return true for existing event");

    // Verify removed from queue
    assert_eq!(queue.len(), 0);
    assert!(queue.is_empty());

    // Verify removed from disk
    assert!(
        !event_file_exists(&events_dir, &id),
        "event file should be removed from disk"
    );
    assert_eq!(
        count_event_files(&events_dir),
        0,
        "should have 0 files after delete"
    );

    // pop() returns None (event was deleted, not processed)
    assert!(queue.pop().is_none());

    // Delete again returns false
    let deleted_again = queue.delete(&id);
    assert!(
        !deleted_again,
        "delete should return false for already-deleted event"
    );
}

// ── Modify On Disk ──────────────────────────────────────────────────────

/// Push event, edit file on disk, process event → verify event processed
/// with updated content.
///
/// Verifies that the disk is the source of truth: when a user modifies
/// an event file on disk, the updated content is used when the event
/// is processed (not the original push content).
#[test]
fn modify_on_disk_processed_with_updated_content() {
    let tmp = tempfile::tempdir().unwrap();
    let events_dir = tmp.path().to_path_buf();
    let queue = DiskBackedEventQueue::new(events_dir.clone());

    // Push an event
    let event = make_pending_event(
        "1000-aaaa",
        "Created",
        "review-loom",
        "review",
        "/project/strands/feature.md",
    );
    queue.push(event.clone());

    // Modify the file on disk: change the kind to "Modified" and update
    // the strand path
    let mut modified_event = event.clone();
    modified_event.kind = "Modified".to_string();
    modified_event.strand_path = "/project/strands/feature-updated.md"
        .to_string();

    let file_path = events_dir.join(format!("{}.json", event.id.0));
    fs::write(
        &file_path,
        serde_json::to_string_pretty(&modified_event).unwrap(),
    )
    .unwrap();

    // Verify the file was modified
    let disk_content = fs::read_to_string(&file_path).unwrap();
    assert!(
        disk_content.contains("Modified"),
        "disk file should contain updated kind"
    );

    // Pop should return the updated content
    let result = queue.pop();
    match result {
        Some(PendingEventOrShutdown::Event(e)) => {
            assert_eq!(
                e.kind, "Modified",
                "pop should return updated kind from disk"
            );
            assert_eq!(
                e.strand_path,
                "/project/strands/feature-updated.md",
                "pop should return updated strand path from disk"
            );
        }
        _ => panic!("expected Event, got {:?}", result),
    };

    // File removed after pop
    assert!(
        !event_file_exists(&events_dir, &event.id),
        "file should be removed after pop"
    );

    // Also verify snapshot() reads fresh from disk
    let event2 = make_pending_event(
        "1001-bbbb",
        "Created",
        "review-loom",
        "review",
        "/project/strands/other.md",
    );
    queue.push(event2.clone());

    // Modify again
    let mut event2_modified = event2.clone();
    event2_modified.kind = "Deleted".to_string();
    let file_path2 = events_dir.join(format!("{}.json", event2.id.0));
    fs::write(
        &file_path2,
        serde_json::to_string_pretty(&event2_modified).unwrap(),
    )
    .unwrap();

    // Snapshot should reflect the modification
    let snapshot = queue.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(
        snapshot[0].kind,
        "Deleted",
        "snapshot should reflect on-disk modification"
    );
}

// ── Non-JSON Files Ignored ───────────────────────────────────────────────

/// Non-`.json` files in the events directory are silently ignored by
/// the queue (not treated as events).
#[test]
fn non_json_files_ignored_in_events_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let events_dir = tmp.path().to_path_buf();

    // Write non-JSON files that should be ignored
    fs::write(events_dir.join("notes.md"), "some notes").unwrap();
    fs::write(
        events_dir.join("1000-aaaa.json.tmp"),
        "{}",
    )
    .unwrap();
    fs::write(events_dir.join(".gitignore"), "*.json.tmp").unwrap();

    // Create queue — should handle these files gracefully
    let queue = DiskBackedEventQueue::new(events_dir.clone());

    // Queue should be empty (non-JSON files are ignored)
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);

    // Push a real event — should work alongside non-JSON files
    let event = make_pending_event(
        "1001-bbbb",
        "Created",
        "review-loom",
        "review",
        "/project/strands/feature.md",
    );
    queue.push(event);

    assert_eq!(queue.len(), 1);
    // Count includes our new file only (non-JSON files excluded)
    assert_eq!(count_event_files(&events_dir), 1);
}
