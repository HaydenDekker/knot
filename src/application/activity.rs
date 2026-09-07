//! In-memory run activity — plan 083 (consolidated service log).
//!
//! Replaces the per-run `.loom-log` / `.rig-log` JSONL files: the
//! current run's events live in memory (one [`RunActivity`] per
//! process, built inside `build_app_context`), and the only persistent
//! observability file is `state.json` (written only when state
//! changes).
//!
//! [`InMemoryLoomLog`] and [`InMemoryRigLog`] implement the existing
//! log ports — `read_all` keeps working for the query use cases
//! (`GetLoomActivity`, `GetKnotStatus`) — and log the `[KNOT][EVENT]`
//! line on append.

use std::collections::HashMap;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::application::ports::{LoomLogPort, PortError, RigLogPort};
use crate::domain::entities::LoomId;
#[cfg(test)]
use crate::domain::entities::{KnotId, StrandPath};
use crate::domain::events::{LoomEvent, RigLogEvent};

/// In-memory store for the current run's activity.
///
/// Loom events are keyed by loom id (one vec per loom, in append
/// order); rig events are a single vec in append order. Nothing is
/// persisted — the store dies with the process, by design.
#[derive(Debug, Default)]
pub struct RunActivity {
    loom_events: Mutex<HashMap<String, Vec<LoomEvent>>>,
    rig_events: Mutex<Vec<RigLogEvent>>,
}

impl RunActivity {
    /// Create an empty activity store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a loom event (in-memory only — no log line here; the
    /// log adapters own the `[EVENT]` rendering).
    pub fn append_loom(&self, loom_id: &str, event: LoomEvent) {
        self.loom_events
            .lock()
            .unwrap()
            .entry(loom_id.to_string())
            .or_default()
            .push(event);
    }

    /// Append a rig event.
    pub fn append_rig(&self, event: RigLogEvent) {
        self.rig_events.lock().unwrap().push(event);
    }

    /// All events recorded for one loom, in append order (empty if
    /// the loom was never seen this run).
    pub fn loom_events(&self, loom_id: &str) -> Vec<LoomEvent> {
        self.loom_events
            .lock()
            .unwrap()
            .get(loom_id)
            .cloned()
            .unwrap_or_default()
    }

    /// All rig events, in append order.
    pub fn rig_events(&self) -> Vec<RigLogEvent> {
        self.rig_events.lock().unwrap().clone()
    }
}

/// Extract the loom id from any [`LoomEvent`] variant (all variants
/// carry one).
fn loom_id_of(event: &LoomEvent) -> &LoomId {
    match event {
        LoomEvent::LoomStarted { loom_id, .. }
        | LoomEvent::LoomStopped { loom_id, .. }
        | LoomEvent::KnotRegistered { loom_id, .. }
        | LoomEvent::KnotDeregistered { loom_id, .. }
        | LoomEvent::KnotProcessing { loom_id, .. }
        | LoomEvent::KnotCompleted { loom_id, .. }
        | LoomEvent::KnotFailed { loom_id, .. }
        | LoomEvent::StrandProcessed { loom_id, .. }
        | LoomEvent::StrandIgnored { loom_id, .. }
        | LoomEvent::StrandSkipped { loom_id, .. }
        | LoomEvent::AgentInactivity { loom_id, .. }
        | LoomEvent::SessionResumed { loom_id, .. }
        | LoomEvent::KnotEmptyResponse { loom_id, .. }
        | LoomEvent::ContextCompacted { loom_id, .. }
        | LoomEvent::EventsDispatched { loom_id, .. }
        | LoomEvent::KnotEventsMissing { loom_id, .. }
        | LoomEvent::KnotParseWarning { loom_id, .. }
        | LoomEvent::DirectoryCreated { loom_id, .. } => loom_id,
    }
}

/// [`LoomLogPort`] backed by an in-memory [`RunActivity`] — no file is
/// created. `open` is a no-op; `append` logs the `[KNOT][EVENT]` line
/// and stores the event; `read_all` returns the current in-memory
/// events for that loom.
#[derive(Clone)]
pub struct InMemoryLoomLog {
    activity: Arc<RunActivity>,
}

impl InMemoryLoomLog {
    /// Create the adapter around a shared activity store.
    pub fn new(activity: Arc<RunActivity>) -> Self {
        Self { activity }
    }

    /// The shared activity store (for tests and composition).
    pub fn activity(&self) -> Arc<RunActivity> {
        self.activity.clone()
    }
}

impl LoomLogPort for InMemoryLoomLog {
    fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
        Ok(())
    }

    fn append(&self, event: LoomEvent) -> Result<(), PortError> {
        crate::adapters::service_log::log_loom_event_line(&event);
        let loom_id = loom_id_of(&event).0.clone();
        self.activity.append_loom(&loom_id, event);
        Ok(())
    }

    fn read_all(&self, loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
        Ok(self.activity.loom_events(&loom_id.0))
    }
}

/// [`RigLogPort`] backed by an in-memory [`RunActivity`] — no file is
/// created. `append` logs the `[KNOT][EVENT]` line and stores the
/// event; `read_all` returns the current in-memory events.
#[derive(Clone)]
pub struct InMemoryRigLog {
    activity: Arc<RunActivity>,
}

impl InMemoryRigLog {
    /// Create the adapter around a shared activity store.
    pub fn new(activity: Arc<RunActivity>) -> Self {
        Self { activity }
    }
}

impl RigLogPort for InMemoryRigLog {
    fn append(&self, event: RigLogEvent) -> Result<(), PortError> {
        crate::adapters::service_log::log_rig_event_line(&event);
        self.activity.append_rig(event);
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<RigLogEvent>, PortError> {
        Ok(self.activity.rig_events())
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> String {
        "2026-09-07T14:03:25+01:00".to_string()
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn loom_started(loom: &str) -> LoomEvent {
        LoomEvent::LoomStarted {
            loom_id: LoomId(loom.to_string()),
            timestamp: ts(),
        }
    }

    fn knot_completed(loom: &str, knot: &str) -> LoomEvent {
        LoomEvent::KnotCompleted {
            loom_id: LoomId(loom.to_string()),
            knot_id: KnotId(knot.to_string()),
            strand_path: StrandPath(p("strands/prd.md")),
            tie_off_path: crate::domain::entities::TieOffPath(p("tie-offs/rig/prd.md")),
            timestamp: ts(),
        }
    }

    #[test]
    fn loom_events_are_keyed_per_loom() {
        let activity = RunActivity::new();
        activity.append_loom("a-loom", loom_started("a-loom"));
        activity.append_loom("a-loom", knot_completed("a-loom", "k"));
        activity.append_loom("b-loom", loom_started("b-loom"));

        assert_eq!(activity.loom_events("a-loom").len(), 2);
        assert_eq!(activity.loom_events("b-loom").len(), 1);
        assert!(activity.loom_events("c-loom").is_empty());
        // Order preserved within a loom.
        let events = activity.loom_events("a-loom");
        assert!(matches!(events[0], LoomEvent::LoomStarted { .. }));
        assert!(matches!(events[1], LoomEvent::KnotCompleted { .. }));
    }

    #[test]
    fn rig_events_keep_append_order() {
        let activity = RunActivity::new();
        activity.append_rig(RigLogEvent::QueueIdle { timestamp: ts() });
        activity.append_rig(RigLogEvent::TimeoutExceeded {
            loom_id: LoomId("a-loom".into()),
            knot_id: KnotId("k".into()),
            strand_path: StrandPath(p("strands/prd.md")),
            error: "timed out".into(),
            timestamp: ts(),
        });
        let events = activity.rig_events();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], RigLogEvent::QueueIdle { .. }));
        assert!(matches!(events[1], RigLogEvent::TimeoutExceeded { .. }));
    }

    #[test]
    fn in_memory_loom_log_roundtrip() {
        let activity = Arc::new(RunActivity::new());
        let log = InMemoryLoomLog::new(activity.clone());
        let loom_id = LoomId("a-loom".into());

        log.open(&loom_id).unwrap();
        log.append(loom_started("a-loom")).unwrap();
        log.append(knot_completed("a-loom", "k")).unwrap();

        let events = log.read_all(&loom_id).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[1], LoomEvent::KnotCompleted { .. }));

        // A second loom id reads only its own events.
        let other = LoomId("b-loom".into());
        assert!(log.read_all(&other).unwrap().is_empty());
    }

    #[test]
    fn in_memory_rig_log_roundtrip() {
        let activity = Arc::new(RunActivity::new());
        let log = InMemoryRigLog::new(activity.clone());
        log.append(RigLogEvent::QueueIdle { timestamp: ts() })
            .unwrap();
        let events = log.read_all().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], RigLogEvent::QueueIdle { .. }));
    }

    #[test]
    fn adapters_share_one_activity_store() {
        let activity = Arc::new(RunActivity::new());
        let loom_log = InMemoryLoomLog::new(activity.clone());
        let rig_log = InMemoryRigLog::new(activity.clone());

        loom_log
            .append(knot_completed("a-loom", "k"))
            .unwrap();
        rig_log.append(RigLogEvent::QueueIdle { timestamp: ts() }).unwrap();

        assert_eq!(activity.loom_events("a-loom").len(), 1);
        assert_eq!(activity.rig_events().len(), 1);
    }
}
