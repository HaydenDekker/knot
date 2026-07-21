//! Persistent event format for disk-backed strand event queue.
//!
//! Defines the serialisable event data model (`PendingEvent`), unique
//! identifier (`PendingEventId`), file naming convention, and
//! conversions to/from the domain's [`StrandEvent`].
//!
//! Each pending event is stored as a JSON file in `rig/events/` with a
//! filename of `{unix_timestamp_ms}-{4-hex-chars}.json`. The timestamp
//! ensures FIFO ordering by filename sort; the random suffix prevents
//! collisions within the same millisecond.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::entities::{KnotId, LoomId, StrandPath};
use super::events::StrandEvent;

// ── PendingEventId ────────────────────────────────────────────────────────

/// Unique identifier for a pending event.
///
/// Formatted as `{unix_timestamp_ms}-{4-hex-chars}`. The timestamp prefix
/// ensures sortable, FIFO-ordered filenames. The random suffix prevents
/// collisions when multiple events are generated within the same millisecond.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PendingEventId(pub String);

impl PendingEventId {
    /// Return the inner ID string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PendingEventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── ID Generation ─────────────────────────────────────────────────────────

/// Generate a unique, sortable event ID.
///
/// Format: `{unix_timestamp_ms}-{4-hex-chars}`.
/// The 4 hex chars (16 bits of randomness) prevent collisions within the
/// same millisecond while keeping IDs short and human-readable.
pub fn generate_event_id() -> PendingEventId {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let rand_part = rand_hex(4);
    PendingEventId(format!("{}-{}", timestamp, rand_part))
}

/// Generate `n` lowercase hex characters from OS randomness.
fn rand_hex(n: usize) -> String {
    use std::io::Read;
    let mut file = match std::fs::File::open("/dev/urandom") {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let mut buf = vec![0u8; n / 2];
    if file.read_exact(&mut buf).is_err() {
        return String::new();
    }
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

// ── PendingEvent ──────────────────────────────────────────────────────────

/// A serialisable strand event ready for disk persistence.
///
/// Stored as JSON in `rig/events/` as `{id}.json`. The disk file IS the
/// queue — there is no separate in-memory index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEvent {
    /// Unique event identifier (matches the `.json` filename stem).
    pub id: PendingEventId,
    /// Event kind: `"Created"`, `"Modified"`, or `"Deleted"`.
    pub kind: String,
    /// The loom this event targets.
    pub loom_id: String,
    /// The knot this event targets.
    pub knot_id: String,
    /// Absolute path to the strand file.
    pub strand_path: String,
    /// ISO 8601 timestamp (local time) when the event entered the queue.
    pub queued_at: String,
}

// ── PendingEventOrShutdown ────────────────────────────────────────────────

/// The value type produced by the queue.
///
/// Replaces `Option<StrandEvent>` so the shutdown sentinel
/// is an explicit variant rather than `None`.
#[derive(Debug, Clone)]
pub enum PendingEventOrShutdown {
    /// A real strand event from the queue.
    Event(PendingEvent),
    /// The debounce engine has shut down and the queue is drained.
    Shutdown,
}

// ── Deduplication ─────────────────────────────────────────────────────────

/// Deduplication key: `(strand_path, loom_id, knot_id, kind)`.
///
/// Events with the same key are considered duplicates — `push_or_replace`
/// replaces the existing entry rather than queuing a second one.
pub type DedupKey = (String, String, String, String);

/// Derive the deduplication key from a [`PendingEvent`].
pub fn dedup_key(event: &PendingEvent) -> DedupKey {
    (
        event.strand_path.clone(),
        event.loom_id.clone(),
        event.knot_id.clone(),
        event.kind.clone(),
    )
}

// ── Conversions: StrandEvent ↔ PendingEvent ──────────────────────────────

/// Convert a domain [`StrandEvent`] into a persistable [`PendingEvent`].
///
/// Generates a new [`PendingEventId`] and captures the current timestamp
/// as `queued_at`.
impl From<StrandEvent> for PendingEvent {
    fn from(event: StrandEvent) -> Self {
        let kind = match &event {
            StrandEvent::Created { .. } => "Created",
            StrandEvent::Modified { .. } => "Modified",
            StrandEvent::Deleted { .. } => "Deleted",
        };
        match event {
            StrandEvent::Created {
                loom_id,
                knot_id,
                strand_path,
            }
            | StrandEvent::Modified {
                loom_id,
                knot_id,
                strand_path,
            }
            | StrandEvent::Deleted {
                loom_id,
                knot_id,
                strand_path,
            } => PendingEvent {
                id: generate_event_id(),
                kind: kind.to_string(),
                loom_id: loom_id.0,
                knot_id: knot_id.0,
                strand_path: strand_path.0.to_string_lossy().into_owned(),
                queued_at: crate::adapters::logging::format_timestamp(),
            },
        }
    }
}

/// Convert a [`PendingEvent`] back into a domain [`StrandEvent`].
///
/// Returns an error if `kind` is not one of `"Created"`, `"Modified"`,
/// or `"Deleted"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownEventKind(String);

impl std::fmt::Display for UnknownEventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown event kind: {}", self.0)
    }
}

impl std::error::Error for UnknownEventKind {}

impl TryFrom<PendingEvent> for StrandEvent {
    type Error = UnknownEventKind;

    fn try_from(event: PendingEvent) -> Result<Self, Self::Error> {
        let loom_id = LoomId(event.loom_id);
        let knot_id = KnotId(event.knot_id);
        let strand_path = StrandPath(PathBuf::from(&event.strand_path));

        match event.kind.as_str() {
            "Created" => Ok(StrandEvent::Created {
                loom_id,
                knot_id,
                strand_path,
            }),
            "Modified" => Ok(StrandEvent::Modified {
                loom_id,
                knot_id,
                strand_path,
            }),
            "Deleted" => Ok(StrandEvent::Deleted {
                loom_id,
                knot_id,
                strand_path,
            }),
            other => Err(UnknownEventKind(other.to_string())),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── generate_event_id ────────────────────────────────────────────

    /// 10 rapid IDs are all unique.
    #[test]
    fn generate_event_id_produces_unique_ids() {
        let mut ids = Vec::new();
        for _ in 0..10 {
            ids.push(generate_event_id());
        }
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(
            unique.len(),
            10,
            "all IDs should be unique: {:?}",
            ids
        );
    }

    /// IDs generated sequentially are sortable (timestamp prefix
    /// ensures creation order matches lexicographic order).
    #[test]
    fn generate_event_id_produces_sortable_ids() {
        let mut ids = Vec::new();
        for _ in 0..10 {
            ids.push(generate_event_id());
            // Tiny sleep to ensure monotonic timestamps.
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut sorted = ids.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            ids, sorted,
            "IDs should be in creation order (sortable by string): {:?}",
            ids
        );
    }

    /// ID format is `{digits}-{4-hex-chars}`.
    #[test]
    fn generate_event_id_format() {
        let id = generate_event_id();
        let parts: Vec<&str> = id.0.split('-').collect();
        assert_eq!(parts.len(), 2, "ID should have exactly one hyphen");
        // Timestamp portion should be all digits
        assert!(
            parts[0].chars().all(|c| c.is_ascii_digit()),
            "timestamp part should be all digits: {}",
            parts[0]
        );
        // Random portion should be exactly 4 hex chars
        assert_eq!(
            parts[1].len(),
            4,
            "random part should be 4 chars: {}",
            parts[1]
        );
        assert!(
            parts[1].chars().all(|c| c.is_ascii_hexdigit()),
            "random part should be hex: {}",
            parts[1]
        );
    }

    // ── From<StrandEvent> for PendingEvent ───────────────────────────

    /// `Created` StrandEvent converts correctly.
    #[test]
    fn from_strand_event_created() {
        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("test-knot".to_string()),
            strand_path: StrandPath(PathBuf::from("/project/file.md")),
        };
        let pending: PendingEvent = event.into();

        assert_eq!(pending.kind, "Created");
        assert_eq!(pending.loom_id, "test-loom");
        assert_eq!(pending.knot_id, "test-knot");
        assert_eq!(pending.strand_path, "/project/file.md");
        assert!(!pending.queued_at.is_empty());
    }

    /// `Modified` StrandEvent converts correctly.
    #[test]
    fn from_strand_event_modified() {
        let event = StrandEvent::Modified {
            loom_id: LoomId("loom-a".to_string()),
            knot_id: KnotId("knot-x".to_string()),
            strand_path: StrandPath(PathBuf::from("/project/doc.md")),
        };
        let pending: PendingEvent = event.into();

        assert_eq!(pending.kind, "Modified");
        assert_eq!(pending.loom_id, "loom-a");
        assert_eq!(pending.knot_id, "knot-x");
        assert_eq!(pending.strand_path, "/project/doc.md");
    }

    /// `Deleted` StrandEvent converts correctly.
    #[test]
    fn from_strand_event_deleted() {
        let event = StrandEvent::Deleted {
            loom_id: LoomId("loom-b".to_string()),
            knot_id: KnotId("knot-y".to_string()),
            strand_path: StrandPath(PathBuf::from("/project/gone.md")),
        };
        let pending: PendingEvent = event.into();

        assert_eq!(pending.kind, "Deleted");
        assert_eq!(pending.loom_id, "loom-b");
        assert_eq!(pending.knot_id, "knot-y");
        assert_eq!(pending.strand_path, "/project/gone.md");
    }

    // ── TryFrom<PendingEvent> for StrandEvent ────────────────────────

    /// Valid event round-trips through PendingEvent → StrandEvent.
    #[test]
    fn try_from_pending_event_created_roundtrip() {
        let original = StrandEvent::Created {
            loom_id: LoomId("loom".to_string()),
            knot_id: KnotId("knot".to_string()),
            strand_path: StrandPath(PathBuf::from("/f.md")),
        };
        let pending: PendingEvent = original.clone().into();
        let restored: StrandEvent = pending.try_into().unwrap();
        assert_eq!(restored, original);
    }

    /// Modified round-trips correctly.
    #[test]
    fn try_from_pending_event_modified_roundtrip() {
        let original = StrandEvent::Modified {
            loom_id: LoomId("loom".to_string()),
            knot_id: KnotId("knot".to_string()),
            strand_path: StrandPath(PathBuf::from("/f.md")),
        };
        let pending: PendingEvent = original.clone().into();
        let restored: StrandEvent = pending.try_into().unwrap();
        assert_eq!(restored, original);
    }

    /// Deleted round-trips correctly.
    #[test]
    fn try_from_pending_event_deleted_roundtrip() {
        let original = StrandEvent::Deleted {
            loom_id: LoomId("loom".to_string()),
            knot_id: KnotId("knot".to_string()),
            strand_path: StrandPath(PathBuf::from("/f.md")),
        };
        let pending: PendingEvent = original.clone().into();
        let restored: StrandEvent = pending.try_into().unwrap();
        assert_eq!(restored, original);
    }

    /// Unknown kind returns an error.
    #[test]
    fn try_from_pending_event_unknown_kind_returns_error() {
        let pending = PendingEvent {
            id: PendingEventId("123-abcd".to_string()),
            kind: "Unknown".to_string(),
            loom_id: "loom".to_string(),
            knot_id: "knot".to_string(),
            strand_path: "/f.md".to_string(),
            queued_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let result: Result<StrandEvent, _> = pending.try_into();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.to_string(), "unknown event kind: Unknown");
    }

    // ── JSON Serialization ───────────────────────────────────────────

    /// All three variants survive JSON round-trip.
    #[test]
    fn json_roundtrip_created() {
        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("test-knot".to_string()),
            strand_path: StrandPath(PathBuf::from("/project/file.md")),
        };
        let pending: PendingEvent = event.into();

        let json = serde_json::to_string(&pending).unwrap();
        let deserialized: PendingEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, pending);
        // Verify JSON schema matches plan
        assert!(json.contains(r#""kind":"Created""#));
        assert!(json.contains(r#""loom_id":"test-loom""#));
        assert!(json.contains(r#""knot_id":"test-knot""#));
        assert!(json.contains(r#""strand_path":"/project/file.md""#));
        assert!(json.contains(r#""queued_at":"#));
    }

    #[test]
    fn json_roundtrip_modified() {
        let event = StrandEvent::Modified {
            loom_id: LoomId("loom".to_string()),
            knot_id: KnotId("k".to_string()),
            strand_path: StrandPath(PathBuf::from("/p.md")),
        };
        let pending: PendingEvent = event.into();
        let json = serde_json::to_string(&pending).unwrap();
        let deserialized: PendingEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.kind, "Modified");
        assert_eq!(deserialized, pending);
    }

    #[test]
    fn json_roundtrip_deleted() {
        let event = StrandEvent::Deleted {
            loom_id: LoomId("loom".to_string()),
            knot_id: KnotId("k".to_string()),
            strand_path: StrandPath(PathBuf::from("/p.md")),
        };
        let pending: PendingEvent = event.into();
        let json = serde_json::to_string(&pending).unwrap();
        let deserialized: PendingEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.kind, "Deleted");
        assert_eq!(deserialized, pending);
    }

    // ── Dedup Key ────────────────────────────────────────────────────

    /// Same (path, loom, knot, kind) produces the same key.
    #[test]
    fn dedup_key_same_fields_same_key() {
        let e1 = PendingEvent {
            id: PendingEventId("100-a".to_string()),
            kind: "Created".to_string(),
            loom_id: "loom".to_string(),
            knot_id: "knot".to_string(),
            strand_path: "/file.md".to_string(),
            queued_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let e2 = PendingEvent {
            id: PendingEventId("200-b".to_string()), // different ID
            kind: "Created".to_string(),
            loom_id: "loom".to_string(),
            knot_id: "knot".to_string(),
            strand_path: "/file.md".to_string(),
            queued_at: "2026-01-01T00:00:01Z".to_string(), // different time
        };
        assert_eq!(
            dedup_key(&e1),
            dedup_key(&e2),
            "same dedup fields should produce same key"
        );
    }

    /// Same path but different kind produces different keys.
    #[test]
    fn dedup_key_different_kind_different_key() {
        let created = PendingEvent {
            id: PendingEventId("100-a".to_string()),
            kind: "Created".to_string(),
            loom_id: "loom".to_string(),
            knot_id: "knot".to_string(),
            strand_path: "/file.md".to_string(),
            queued_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let modified = PendingEvent {
            id: PendingEventId("100-b".to_string()),
            kind: "Modified".to_string(),
            loom_id: "loom".to_string(),
            knot_id: "knot".to_string(),
            strand_path: "/file.md".to_string(),
            queued_at: "2026-01-01T00:00:00Z".to_string(),
        };
        assert_ne!(
            dedup_key(&created),
            dedup_key(&modified),
            "different kinds should produce different keys"
        );
    }

    // ── PendingEventOrShutdown ───────────────────────────────────────

    /// Event variant pattern-matches correctly.
    #[test]
    fn pending_event_or_shutdown_event_variant() {
        let event = PendingEvent {
            id: PendingEventId("1-a".to_string()),
            kind: "Created".to_string(),
            loom_id: "loom".to_string(),
            knot_id: "knot".to_string(),
            strand_path: "/f.md".to_string(),
            queued_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let eos = PendingEventOrShutdown::Event(event.clone());
        match eos {
            PendingEventOrShutdown::Event(e) => {
                assert_eq!(e.id.0, "1-a");
                assert_eq!(e.kind, "Created");
            }
            PendingEventOrShutdown::Shutdown => {
                panic!("expected Event variant");
            }
        }
    }

    /// Shutdown variant pattern-matches correctly.
    #[test]
    fn pending_event_or_shutdown_shutdown_variant() {
        let eos = PendingEventOrShutdown::Shutdown;
        match eos {
            PendingEventOrShutdown::Shutdown => {
                // correct
            }
            PendingEventOrShutdown::Event(_) => {
                panic!("expected Shutdown variant");
            }
        }
    }
}
