//! Persistent event format for disk-backed strand event queue.
//!
//! Defines the serialisable event data model (`PendingEvent`), unique
//! identifier (`PendingEventId`), file naming convention, and
//! conversions to/from the domain's [`StrandEvent`].
//!
//! Each pending event is stored as a JSON file in the rig's runtime-root
//! events directory (`tie-offs/<rig-basename>/events/`) with a
//! filename of `{unix_timestamp_ms}-{4-hex-chars}.json`. The timestamp
//! ensures FIFO ordering by filename sort; the random suffix prevents
//! collisions within the same millisecond.
//!
//! ## Two entry kinds (plan 099)
//!
//! A **filesystem entry** (`kind` `Created` / `Modified` / `Deleted`)
//! points at a real strand file — the entry is a pointer, the file holds
//! the message, which is right for a file trigger.
//!
//! An **event entry** (`kind` [`EVENT_ENTRY_KIND`]) *is* the message: it
//! carries the emitted payload in [`EventDelivery`] and no file exists
//! to read. Its `strand_path` is the synthetic
//! [`event_strand_descriptor`] (`event:<producer>/<event-id>`), so
//! every log line and tie-off header keeps a meaningful, greppable
//! `strand=` field.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

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

// ── Event delivery payload (plan 099) ─────────────────────────────────────

/// The queue-entry `kind` for a delivered agent-event emission.
///
/// Distinct from the filesystem kinds: an `Event` entry carries its
/// message in [`EventDelivery`] and its `strand_path` is the synthetic
/// [`event_strand_descriptor`], not a file path.
pub const EVENT_ENTRY_KIND: &str = "Event";

/// Build the synthetic strand descriptor carried by an event-delivered
/// queue entry: `event:<producer-knot>/<event-id>`.
///
/// Not a filesystem path — nothing is ever stat-ed or read at it. It
/// exists so the delivered entry keeps a `strand=` field that names
/// where the message came from, and so `grep 'strand=event:<knot>/'`
/// finds every delivery of one producer's events.
pub fn event_strand_descriptor(producer_knot: &str, event_id: &str) -> String {
    format!("event:{}/{}", producer_knot, event_id)
}

/// Whether a `strand_path` value is an event descriptor rather than a
/// filesystem path.
pub fn is_event_descriptor(strand_path: &str) -> bool {
    strand_path.starts_with("event:")
}

/// Parse an event descriptor (`event:<producer>/<event-id>`) into its
/// parts. `None` for anything else, including a filesystem path.
pub fn parse_event_descriptor(strand_path: &str) -> Option<(String, String)> {
    let rest = strand_path.strip_prefix("event:")?;
    let (producer, event_id) = rest.split_once('/')?;
    if producer.is_empty() || event_id.is_empty() {
        return None;
    }
    Some((producer.to_string(), event_id.to_string()))
}

/// The payload one producer emission carries to one subscriber (plan
/// 099).
///
/// Embedded in the queue entry that delivers the emission, so the
/// consumer receives the message **from the queue** — no dispatched
/// event file exists to read, and the payload is not duplicated on
/// disk. The full agent narrative behind the emission stays reachable
/// through the producer's `session:` trailer (`pi --session-id`).
///
/// `payload` is a [`BTreeMap`] (not the parsed [`super::events::AgentEvent`]'s
/// `HashMap`) so the persisted entry JSON is deterministic — two runs of
/// the same emission write byte-identical entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDelivery {
    /// The emitted event's id (e.g. `PlanCreated`).
    pub event_id: String,
    /// The knot that emitted the event.
    pub producer_knot: String,
    /// The loom the producer belongs to.
    pub producer_loom: String,
    /// The producer's own trigger strand — the file whose change started
    /// the chain that produced this emission. `None` when the producer
    /// had no file trigger (a system event, or a rig-scoped emission).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_strand: Option<String>,
    /// The event's narrative body, verbatim from the producer's tie-off
    /// block (`None` when the producer emitted no body).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The event's key-value payload fields.
    #[serde(default)]
    pub payload: BTreeMap<String, String>,
}

impl EventDelivery {
    /// The synthetic strand descriptor this delivery's queue entry
    /// carries.
    pub fn strand_descriptor(&self) -> String {
        event_strand_descriptor(&self.producer_knot, &self.event_id)
    }

    /// Render the delivery as the block a consumer agent is given in
    /// place of a file it cannot read (plan 099).
    ///
    /// Carries the producer identity, the event id, the producer's
    /// original strand, the payload fields, and the body — everything
    /// the consumer previously got from the dispatched `.md`'s
    /// frontmatter and body, and nothing it did not.
    pub fn render_for_consumer(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "## Event: {} from {} (loom {})\n",
            self.event_id, self.producer_knot, self.producer_loom
        ));
        if let Some(ref original) = self.original_strand {
            out.push_str(&format!("original_strand: {}\n", original));
        }
        if !self.payload.is_empty() {
            out.push_str("\npayload:\n");
            for (key, value) in &self.payload {
                out.push_str(&format!("- **{}**: {}\n", key, value));
            }
        }
        if let Some(ref body) = self.body {
            out.push('\n');
            out.push_str(body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }
}

// ── PendingEvent ──────────────────────────────────────────────────────────

/// A serialisable strand event ready for disk persistence.
///
/// Stored as JSON in `rig/events/` as `{id}.json`. The disk file IS the
/// queue — there is no separate in-memory index.
///
/// Filesystem kinds carry `delivery: None` and `strand_path` points at a
/// real file. An [`EVENT_ENTRY_KIND`] entry carries the message in
/// `delivery` and a synthetic descriptor in `strand_path` — see the
/// module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEvent {
    /// Unique event identifier (matches the `.json` filename stem).
    pub id: PendingEventId,
    /// Event kind: `"Created"`, `"Modified"`, `"Deleted"`, or
    /// [`EVENT_ENTRY_KIND`].
    pub kind: String,
    /// The loom this event targets.
    pub loom_id: String,
    /// The knot this event targets.
    pub knot_id: String,
    /// Filesystem entries: the absolute path to the strand file.
    /// Event entries: the synthetic [`event_strand_descriptor`].
    pub strand_path: String,
    /// ISO 8601 timestamp (local time) when the event entered the queue.
    pub queued_at: String,
    /// The embedded emission payload — `Some` only for
    /// [`EVENT_ENTRY_KIND`] entries. Absent (and never serialised) for
    /// the filesystem kinds, so pre-099 entries on disk load unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<EventDelivery>,
}

impl PendingEvent {
    /// Whether this entry delivers an agent-event emission rather than a
    /// filesystem change.
    pub fn is_event_delivery(&self) -> bool {
        self.kind == EVENT_ENTRY_KIND
    }

    /// Build the queue entry that delivers `delivery` to one subscriber
    /// (`consumer_loom` / `consumer_knot`).
    ///
    /// One call per subscriber — fan-out is the caller's job, and each
    /// subscriber gets its own entry with its own id and `queued_at` so
    /// removal is per-delivery.
    pub fn from_delivery(
        delivery: EventDelivery,
        consumer_loom: &LoomId,
        consumer_knot: &KnotId,
    ) -> Self {
        let strand_path = delivery.strand_descriptor();
        PendingEvent {
            id: generate_event_id(),
            kind: EVENT_ENTRY_KIND.to_string(),
            loom_id: consumer_loom.0.clone(),
            knot_id: consumer_knot.0.clone(),
            strand_path,
            queued_at: crate::adapters::logging::format_timestamp(),
            delivery: Some(delivery),
        }
    }
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
            StrandEvent::EventDelivered { .. } => EVENT_ENTRY_KIND,
        };
        match event {
            StrandEvent::EventDelivered {
                loom_id,
                knot_id,
                delivery,
                ..
            } => PendingEvent::from_delivery(*delivery, &loom_id, &knot_id),
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
                delivery: None,
            },
        }
    }
}

/// Convert a [`PendingEvent`] back into a domain [`StrandEvent`].
///
/// An [`EVENT_ENTRY_KIND`] entry becomes [`StrandEvent::EventDelivered`]
/// (carrying its embedded payload); the filesystem kinds become their
/// matching variants. Returns an error for any other `kind`, and for an
/// `Event` kind with no payload.
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
            EVENT_ENTRY_KIND => {
                let id = event.id.clone();
                let delivery = event.delivery.ok_or_else(|| {
                    UnknownEventKind(format!(
                        "{} entry {} carries no delivery payload",
                        EVENT_ENTRY_KIND, id
                    ))
                })?;
                Ok(StrandEvent::EventDelivered {
                    loom_id,
                    knot_id,
                    strand_path,
                    delivery: Box::new(delivery),
                })
            }
            other => Err(UnknownEventKind(other.to_string())),
        }
    }
}

// ── Queued Event Index (plan 097) ─────────────────────────────────────────

/// Cap on the number of newest entries listed per event type in a
/// producer's queued-events index. A backlog of N queued entries must not
/// become N prompt entries (the read-bloat anti-pattern); the per-type
/// `count` always carries the full total.
pub const PENDING_INDEX_CAP: usize = 10;

/// One queued event as observed by the pending-index builder: the event
/// file path, when it entered the queue (`queued_at` from the queue entry —
/// empty when the source has no timestamp, e.g. the directory-scan
/// fallback), the producer identity (`None` when unknown), and — for a
/// plan 099 payload-embedded entry — the event id carried by the
/// delivery.
///
/// `event_id` is `Some` only when the queue entry itself names the event
/// type (an `Event`-kind entry). When it is `None` the builder falls back
/// to deriving the type from the path, which is how a legacy dispatched
/// file (or a producer's own frontmatter) is read.
///
/// No event body is ever carried here — the index is metadata-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedEventMeta {
    pub strand_path: PathBuf,
    pub queued_at: String,
    pub producer: Option<String>,
    /// The event id from the queue entry's delivery payload (plan 099).
    /// `None` → derive from the path.
    pub event_id: Option<String>,
}

/// Per-event-type summary in a producer's queued-events index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedEventType {
    /// The event type (the dispatch path's parent directory name).
    pub event_id: String,
    /// Total count of queued entries of this type from this producer.
    pub count: usize,
    /// The newest [`PENDING_INDEX_CAP`] entries (newest first by
    /// `queued_at`), as `(path, queued_at)`.
    pub entries: Vec<(PathBuf, String)>,
}

/// Derive the event type from a queued-entry path.
///
/// Two shapes are recognised:
///
/// - The **event descriptor** (plan 099) —
///   `event:<producer>/<event-id>` → the event id.
/// - A **legacy dispatched file** —
///   `…/review-loom/PlanCreated/event-2026-07-14T10-00-00Z.md` → the
///   parent directory name (`PlanCreated`). Kept so entries queued by
///   an older binary still index during the cutover.
///
/// `None` for anything that is neither.
pub fn event_type_from_path(path: &Path) -> Option<String> {
    // Plan 099: the descriptor is a whole string, not a path with a
    // final component — check the full value first.
    if let Some((_, event_id)) =
        parse_event_descriptor(&path.to_string_lossy())
    {
        return Some(event_id);
    }
    let file_name = path.file_name()?.to_str()?;
    if !file_name.starts_with("event-") {
        return None;
    }
    path.parent()?.file_name()?.to_str().map(str::to_string)
}

/// Count queued entries per event type (from dispatch paths only — no
/// file reads). Used for the pre-session / pre-dispatch `queue-busy`
/// comparison at tie-off dispatch.
pub fn count_event_types(paths: &[PathBuf]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for path in paths {
        if let Some(event_id) = event_type_from_path(path) {
            *counts.entry(event_id).or_insert(0) += 1;
        }
    }
    counts
}

/// Build a producer's queued-events index from queue metadata (plan
/// 097). Pure domain logic — no file I/O: the event type is derived from
/// the dispatch path, `queued_at` comes from the queue entry, and the
/// producer identity arrives as frontmatter data (`target-knot`) that the
/// caller extracted.
///
/// Entries are scoped to `emittable_types` (the event types this knot may
/// emit) and to entries produced by `producer_id`. Per type: the full
/// `count` plus the newest [`PENDING_INDEX_CAP`] entries (newest first by
/// `queued_at`; empty timestamps sort oldest, ties keep input order).
pub fn build_pending_index(
    metas: &[QueuedEventMeta],
    producer_id: &str,
    emittable_types: &HashSet<String>,
) -> Vec<QueuedEventType> {
    let mut by_type: HashMap<String, Vec<QueuedEventMeta>> = HashMap::new();
    for meta in metas {
        // Plan 099: a payload-embedded entry names its own event type;
        // only a legacy dispatched file needs path derivation.
        let Some(event_id) = meta
            .event_id
            .clone()
            .or_else(|| event_type_from_path(&meta.strand_path))
        else {
            continue;
        };
        if !emittable_types.contains(&event_id) {
            continue;
        }
        if meta.producer.as_deref() != Some(producer_id) {
            continue;
        }
        by_type.entry(event_id).or_default().push(meta.clone());
    }

    let mut result: Vec<QueuedEventType> = by_type
        .into_iter()
        .map(|(event_id, mut entries)| {
            let count = entries.len();
            // Newest first by `queued_at` (ISO 8601 — lexicographic
            // order); stable, so ties keep input order.
            entries.sort_by(|a, b| b.queued_at.cmp(&a.queued_at));
            let entries = entries
                .into_iter()
                .take(PENDING_INDEX_CAP)
                .map(|e| (e.strand_path, e.queued_at))
                .collect();
            QueuedEventType { event_id, count, entries }
        })
        .collect();
    result.sort_by(|a, b| a.event_id.cmp(&b.event_id));
    result
}

/// View of a single queue entry for the pending-index builder (path +
/// `queued_at` + the identity the entry itself carries). Defined here
/// (domain) so the accessor trait in [`super::events`] can reference it.
///
/// Plan 099: `event_id` and `producer` come from the entry's
/// [`EventDelivery`] when it has one, so the index no longer has to
/// open a dispatched file to know what is queued. Filesystem-strand
/// entries leave both `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntryView {
    pub strand_path: PathBuf,
    pub queued_at: String,
    /// The delivered event's id (`None` for a filesystem-strand entry).
    pub event_id: Option<String>,
    /// The delivering producer's knot id (`None` for a filesystem entry).
    pub producer: Option<String>,
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;
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
        
            delivery: None,
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
        
            delivery: None,
        };
        let e2 = PendingEvent {
            id: PendingEventId("200-b".to_string()), // different ID
            kind: "Created".to_string(),
            loom_id: "loom".to_string(),
            knot_id: "knot".to_string(),
            strand_path: "/file.md".to_string(),
            queued_at: "2026-01-01T00:00:01Z".to_string(), // different time
        
            delivery: None,
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
        
            delivery: None,
        };
        let modified = PendingEvent {
            id: PendingEventId("100-b".to_string()),
            kind: "Modified".to_string(),
            loom_id: "loom".to_string(),
            knot_id: "knot".to_string(),
            strand_path: "/file.md".to_string(),
            queued_at: "2026-01-01T00:00:00Z".to_string(),
        
            delivery: None,
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
        
            delivery: None,
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

    // ── Queued event index (plan 097) ─────────────────────────────────

    /// One `QueuedEventMeta` for a **legacy** dispatched event file
    /// under `<base>/<consumer-loom>/<EventId>/event-<n>.md` — the
    /// type is derived from the path (`event_id: None`).
    fn meta(event_id: &str, producer: &str, queued_at: &str) -> QueuedEventMeta {
        QueuedEventMeta {
            strand_path: PathBuf::from(format!(
                "/rig/review-loom/{event_id}/event-{queued_at}.md"
            )),
            queued_at: queued_at.to_string(),
            producer: Some(producer.to_string()),
            event_id: None,
        }
    }

    /// One `QueuedEventMeta` for a plan 099 **payload-embedded** queue
    /// entry — the entry names its own type and producer, no file read.
    fn carried_meta(event_id: &str, producer: &str, queued_at: &str) -> QueuedEventMeta {
        QueuedEventMeta {
            strand_path: PathBuf::from(format!("event:{producer}/{event_id}")),
            queued_at: queued_at.to_string(),
            producer: Some(producer.to_string()),
            event_id: Some(event_id.to_string()),
        }
    }

    fn one_type(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn event_type_from_path_derives_type_and_rejects_non_event_files() {
        assert_eq!(
            event_type_from_path(
                Path::new("/tie-offs/rig/review-loom/PlanCreated/event-2026-07-14T10-00-00Z.md"),
            ),
            Some("PlanCreated".to_string())
        );
        // Plan 099: the event descriptor names the type directly.
        assert_eq!(
            event_type_from_path(Path::new("event:plan-creator/PlanCreated")),
            Some("PlanCreated".to_string())
        );
        // Not an `event-*` file.
        assert_eq!(
            event_type_from_path(Path::new("/tie-offs/rig/loom/feature.md")),
            None
        );
        // No parent directory.
        assert_eq!(event_type_from_path(Path::new("event-x.md")), None);
    }

    #[test]
    fn parse_event_descriptor_splits_producer_and_id() {
        assert_eq!(
            parse_event_descriptor("event:plan-creator/PlanCreated"),
            Some(("plan-creator".to_string(), "PlanCreated".to_string()))
        );
        assert_eq!(parse_event_descriptor("/a/b.md"), None);
        assert_eq!(parse_event_descriptor("event:no-slash"), None);
        assert_eq!(parse_event_descriptor("event:/PlanCreated"), None);
        assert_eq!(parse_event_descriptor("event:plan-creator/"), None);
    }

    #[test]
    fn build_pending_index_reads_the_type_from_the_entry_not_the_path() {
        // A carried entry whose path would NOT derive the type still
        // indexes, because the delivery names it (plan 099).
        let metas = vec![carried_meta("PlanCreated", "plan-creator", "2026-10-18T10:00:00Z")];
        let index = build_pending_index(
            &metas,
            "plan-creator",
            &one_type(&["PlanCreated"]),
        );
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].event_id, "PlanCreated");
        assert_eq!(index[0].count, 1);
    }

    #[test]
    fn build_pending_index_caps_at_newest_n_and_keeps_full_count() {
        // 50 queued entries of one type → count 50, listed entries 10,
        // the 10 newest by `queued_at` (newest first).
        let metas: Vec<QueuedEventMeta> = (0..50)
            .map(|i| {
                meta(
                    "PlanCreated",
                    "plan-creator",
                    &format!("2026-10-18T{0:02}:00:00Z", i),
                )
            })
            .collect();
        let index = build_pending_index(
            &metas,
            "plan-creator",
            &one_type(&["PlanCreated"]),
        );
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].event_id, "PlanCreated");
        assert_eq!(index[0].count, 50);
        assert_eq!(index[0].entries.len(), PENDING_INDEX_CAP);
        // Newest first: the highest timestamp leads.
        assert!(index[0].entries[0].1 > index[0].entries[9].1);
    }

    #[test]
    fn build_pending_index_scopes_to_emittable_types_only() {
        let metas = vec![
            meta("PlanCreated", "plan-creator", "2026-10-18T10:00:00Z"),
            meta("PlanScoped", "plan-creator", "2026-10-18T11:00:00Z"),
        ];
        // Only PlanCreated is emittable for this knot.
        let index = build_pending_index(&metas, "plan-creator", &one_type(&["PlanCreated"]));
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].event_id, "PlanCreated");
        assert!(index.iter().all(|t| t.event_id != "PlanScoped"));
    }

    #[test]
    fn build_pending_index_scopes_to_the_producer() {
        let metas = vec![
            meta("PlanCreated", "plan-creator", "2026-10-18T10:00:00Z"),
            meta("PlanCreated", "other-producer", "2026-10-18T11:00:00Z"),
            // Missing frontmatter producer — never matched.
            QueuedEventMeta {
                strand_path: PathBuf::from(
                    "/rig/loom/PlanCreated/event-x.md",
                ),
                queued_at: "2026-10-18T12:00:00Z".to_string(),
                producer: None,
                event_id: None,
            },
        ];
        let index = build_pending_index(&metas, "plan-creator", &one_type(&["PlanCreated"]));
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].count, 1);
    }

    #[test]
    fn build_pending_index_empty_when_nothing_emittable_is_queued() {
        let metas = vec![meta("PlanScoped", "plan-creator", "2026-10-18T11:00:00Z")];
        let index = build_pending_index(&metas, "plan-creator", &one_type(&["PlanCreated"]));
        assert!(index.is_empty());
        // Empty input → empty index.
        assert!(build_pending_index(&[], "plan-creator", &one_type(&["PlanCreated"])).is_empty());
    }

    #[test]
    fn build_pending_index_orders_newest_first_and_types_stable() {
        let metas = vec![
            meta("PlanCreated", "plan-creator", "2026-10-18T12:00:00Z"),
            meta("PlanCreated", "plan-creator", "2026-10-18T10:00:00Z"),
            meta("PlanCreated", "plan-creator", "2026-10-18T11:00:00Z"),
        ];
        let index = build_pending_index(
            &metas,
            "plan-creator",
            &one_type(&["PlanCreated", "PlanScoped"]),
        );
        let entries = &index[0].entries;
        assert_eq!(
            entries
                .iter()
                .map(|(_, ts)| ts.as_str())
                .collect::<Vec<_>>(),
            vec![
                "2026-10-18T12:00:00Z",
                "2026-10-18T11:00:00Z",
                "2026-10-18T10:00:00Z",
            ]
        );
    }

    #[test]
    fn build_pending_index_is_metadata_only() {
        // The builder's input carries only (path, queued_at, producer) —
        // no description, no body — and the output carries only
        // (path, queued_at) pairs. This test pins the shape: nothing
        // beyond path + queued_at can leak into the index.
        let metas = vec![meta("PlanCreated", "plan-creator", "2026-10-18T10:00:00Z")];
        let index = build_pending_index(&metas, "plan-creator", &one_type(&["PlanCreated"]));
        assert_eq!(
            index[0].entries,
            vec![(
                PathBuf::from("/rig/review-loom/PlanCreated/event-2026-10-18T10:00:00Z.md"),
                "2026-10-18T10:00:00Z".to_string(),
            )]
        );
    }

    #[test]
    fn count_event_types_counts_by_path_only() {
        let paths = vec![
            PathBuf::from("/rig/loom/PlanCreated/event-a.md"),
            PathBuf::from("/rig/loom/PlanCreated/event-b.md"),
            PathBuf::from("/rig/loom/PlanScoped/event-c.md"),
            PathBuf::from("/rig/loom/feature.md"), // not an event file
        ];
        let counts = count_event_types(&paths);
        assert_eq!(counts.get("PlanCreated"), Some(&2));
        assert_eq!(counts.get("PlanScoped"), Some(&1));
        assert_eq!(counts.len(), 2);
        assert!(count_event_types(&[]).is_empty());
    }
}
