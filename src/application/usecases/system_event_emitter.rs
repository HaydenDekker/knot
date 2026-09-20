//! System event emitter (plan 082).
//!
//! Every `LoomEvent` / `RigLogEvent` the system writes to the loom-log /
//! rig-log is also **dispatched** as a system event to every subscribed
//! consumer, reusing the existing agent-event machinery: a synthesized
//! [`AgentEvent`] (`occurred: true`) is pushed through
//! [`EventDispatcherPort::dispatch`] with the identical event-file format,
//! per-consumer placement, filename contract (070), and watch/debounce/
//! queue pipeline.
//!
//! ## Producer scopes
//!
//! [`EventScope`] selects the dispatch scope. The producer token written
//! to the event file (`target-knot:` frontmatter) is the knot id, loom id,
//! or rig id respectively. Consumer matching resolves the consumer's
//! `strand-dir` event URI per scope:
//!
//! - [`EventScope::Knot`] — `resolve_for_producer` (knot / loom / `*`).
//! - [`EventScope::Loom`] — `resolve_loom_event` (`<loom-id>` / `*`).
//! - [`EventScope::Rig`] — `resolve_rig_event` (`knot` (canonical engine
//!   token, plan 092) / `<rig-id>` (deprecated) / `*`).
//!
//! ## Loop safety
//!
//! A **system** event is never dispatched back to the knot that produced
//! it (knot-scoped self-exclusion) — see [`SystemEventEmitter::emit`].
//!
//! ## Grouping
//!
//! [`dispatch_grouped`] is the shared grouping/sequencing block (extracted
//! from `ProcessStrand::dispatch_events_to_consumers`) so system-event and
//! agent-event dispatch share the identical filename/`seq` contract.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adapters::logging;
use crate::application::ports::{EventDispatcherPort, PortError};
use crate::application::store::LoomStore;
use crate::domain::entities::{Knot, KnotId, LoomId, StrandPath};
use crate::domain::events::AgentEvent;
use crate::domain::value_objects::StrandSource;

/// One dispatch request handed to [`dispatch_grouped`].
///
/// A borrow of the event being dispatched, the matching consumer knot, the
/// consumer's loom, and the producer token written to the event file.
pub struct DispatchRequest<'a> {
    /// The event to dispatch (synthesized system event or agent event).
    pub event: &'a AgentEvent,
    /// The consumer knot that subscribed to this event.
    pub consumer_knot: &'a Knot,
    /// The consumer knot's loom (target directory's loom).
    pub consumer_loom: &'a LoomId,
    /// Producer token written to the event file (knot id, loom id, or rig
    /// id per scope).
    pub producer: &'a str,
}

/// Group requests by target directory `(consumer_loom_id, event_id)` and
/// dispatch each with a per-group sequence, preserving the 070 filename
/// contract.
///
/// A group with a single member dispatches with `seq = 0` (plain
/// filename); a group of `N > 1` members dispatches with `seq = 1..N` in
/// request order. Requests are assumed to be already matched and filtered
/// (event-ID equality, `occurred`, self-exclusion) by the caller — this
/// function only owns grouping and sequencing, so system-event and
/// agent-event dispatch share the identical filename/`seq` behaviour.
///
/// Returns the list of
/// `(event_id, consumer_knot_id, consumer_loom_id, created_file_path)`
/// dispatches performed.
pub fn dispatch_grouped(
    dispatcher: &dyn EventDispatcherPort,
    rig_dir: &Path,
    requests: &[DispatchRequest<'_>],
) -> Result<Vec<(String, String, String, String)>, PortError> {
    // Pass 1 — count group sizes. Group key = the target directory:
    // `(consumer_loom_id, event_id)`.
    let mut group_sizes: HashMap<(&str, &str), u32> = HashMap::new();
    for r in requests {
        *group_sizes
            .entry((r.consumer_loom.0.as_str(), r.event.event_id.as_str()))
            .or_insert(0) += 1;
    }

    // Pass 2 — dispatch in request order with per-group sequences:
    // singleton group → seq 0 (plain name); group of N > 1 → seq 1..N.
    let mut dispatches: Vec<(String, String, String, String)> = Vec::new();
    let mut group_counts: HashMap<(&str, &str), u32> = HashMap::new();
    for r in requests {
        let key = (r.consumer_loom.0.as_str(), r.event.event_id.as_str());
        let group_size = group_sizes[&key];
        let count = group_counts.entry(key).or_insert(0);
        *count += 1;
        let seq = if group_size == 1 { 0 } else { *count };

        let path = dispatcher.dispatch(
            r.event,
            r.consumer_knot,
            r.producer,
            r.consumer_loom,
            rig_dir,
            seq,
        )?;
        dispatches.push((
            r.event.event_id.clone(),
            r.consumer_knot.id.0.clone(),
            r.consumer_loom.0.clone(),
            path.display().to_string(),
        ));
    }

    Ok(dispatches)
}

/// The dispatch scope of a system event — which producer position the
/// event's producer occupies, and therefore which consumers may match.
#[derive(Debug, Clone)]
pub enum EventScope {
    /// Knot-scoped: the producer is a specific knot (its loom is the
    /// loom-level match). Covers run-lifecycle and failure events.
    Knot {
        loom_id: LoomId,
        knot_id: KnotId,
        /// The strand being processed, when applicable (informational —
        /// callers already place `strand-path` in the payload).
        strand_path: Option<StrandPath>,
    },
    /// Loom-scoped: the producer is a loom itself (no knot) —
    /// `LoomStarted`, `LoomStopped`, `KnotParseWarning`.
    Loom { loom_id: LoomId },
    /// Rig-scoped: the producer is the rig — `QueueIdle`. The rig id is
    /// taken from the emitter's `rig_dir`.
    Rig,
}

impl EventScope {
    /// Convenience constructor for a knot-scoped scope with a strand path.
    pub fn knot(
        loom_id: LoomId,
        knot_id: KnotId,
        strand_path: &StrandPath,
    ) -> Self {
        EventScope::Knot {
            loom_id,
            knot_id,
            strand_path: Some(strand_path.clone()),
        }
    }

    /// Convenience constructor for a knot-scoped scope without a strand
    /// path (knot registration / deregistration).
    pub fn knot_no_strand(loom_id: LoomId, knot_id: KnotId) -> Self {
        EventScope::Knot {
            loom_id,
            knot_id,
            strand_path: None,
        }
    }
}

/// Emits system events to subscribed consumer knots.
///
/// Built in the composition root (`server.rs`) and shared via
/// `AppContext`. Cheap to clone (the store is an `Arc`, the dispatcher and
/// rig dir are shared handles).
#[derive(Clone)]
pub struct SystemEventEmitter {
    store: LoomStore,
    event_dispatcher: Arc<dyn EventDispatcherPort>,
    rig_dir: PathBuf,
}

impl SystemEventEmitter {
    pub fn new(
        store: LoomStore,
        event_dispatcher: Arc<dyn EventDispatcherPort>,
        rig_dir: PathBuf,
    ) -> Self {
        Self {
            store,
            event_dispatcher,
            rig_dir,
        }
    }

    /// The rig id — the rig directory's basename (`rig_dir.file_name()`),
    /// the value a rig-scoped subscription matches (e.g. `dev-rig`).
    pub fn rig_id(&self) -> String {
        self.rig_dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Dispatch a system event to every subscribed consumer.
    ///
    /// Synthesizes an [`AgentEvent`] (`occurred: true`, the given `payload`
    /// map and `body`), scans the store for consumers whose `strand_source`
    /// is an event URI, resolves it for the scope (excluding the producing
    /// knot for knot-scoped dispatches), filters by exact event-ID
    /// equality, and dispatches through the shared [`dispatch_grouped`]
    /// pipeline.
    ///
    /// Returns the list of
    /// `(event_id, consumer_knot_id, consumer_loom_id, created_file_path)`
    /// dispatches performed. Emission is **best-effort and non-fatal**:
    /// callers log a returned `Err` to the console and never let it change
    /// a run outcome.
    pub fn emit(
        &self,
        scope: &EventScope,
        event_id: &str,
        payload: HashMap<String, String>,
        body: Option<String>,
    ) -> Result<Vec<(String, String, String, String)>, PortError> {
        let event = AgentEvent {
            event_id: event_id.to_string(),
            occurred: true,
            payload,
            body,
        };

        let all_looms = self.store.list();
        // Knot ids for knot-scoped resolve disambiguation (knot vs loom).
        let all_knot_ids: Vec<&str> = all_looms
            .iter()
            .flat_map(|l| l.knots.iter())
            .map(|k| k.id.0.as_str())
            .collect();

        // Rig-scoped only (plan 092 D5): event URIs subscribed to this
        // event id whose producer token did not resolve — the
        // rename-mismatch signature reported when matches come back
        // empty.
        let is_rig_scope = matches!(scope, EventScope::Rig);
        let mut rig_near_misses: Vec<String> = Vec::new();

        // (producer token, matched (consumer loom, consumer knot) pairs)
        let (producer, matches): (String, Vec<(&LoomId, &Knot)>) = match scope {
            EventScope::Knot { loom_id, knot_id, .. } => {
                let mut ms: Vec<(&LoomId, &Knot)> = Vec::new();
                for loom in &all_looms {
                    for k in &loom.knots {
                        // Self-exclusion: never dispatch a system event
                        // back to its producing knot.
                        if loom.id == *loom_id && k.id == *knot_id {
                            continue;
                        }
                        if let Some(sub) = k.strand_source.resolve_for_producer(
                            &knot_id.0,
                            &loom_id.0,
                            &all_knot_ids,
                        ) && sub.event_id() == event_id
                        {
                            ms.push((&loom.id, k));
                        }
                    }
                }
                (knot_id.0.clone(), ms)
            }
            EventScope::Loom { loom_id } => {
                let mut ms: Vec<(&LoomId, &Knot)> = Vec::new();
                for loom in &all_looms {
                    for k in &loom.knots {
                        if let Some(sub) =
                            k.strand_source.resolve_loom_event(&loom_id.0)
                            && sub.event_id() == event_id
                        {
                            ms.push((&loom.id, k));
                        }
                    }
                }
                (loom_id.0.clone(), ms)
            }
            EventScope::Rig => {
                let rig = self.rig_id();
                let mut ms: Vec<(&LoomId, &Knot)> = Vec::new();
                for loom in &all_looms {
                    for k in &loom.knots {
                        match k.strand_source.resolve_rig_event(&rig) {
                            Some(sub) if sub.event_id() == event_id => {
                                ms.push((&loom.id, k));
                            }
                            Some(_) => {}
                            None => {
                                // Near-miss (plan 092 D6): an event URI
                                // subscribed to this very event id with a
                                // non-matching producer token (e.g. the
                                // rig directory was renamed after the
                                // subscription was written).
                                if let StrandSource::EventUri {
                                    producer_knot,
                                    event_id: sub_event,
                                } = &k.strand_source
                                    && sub_event.as_str() == event_id
                                {
                                    rig_near_misses.push(format!(
                                        "{} (event:{}:{})",
                                        k.id.0, producer_knot, sub_event
                                    ));
                                }
                            }
                        }
                    }
                }
                (rig, ms)
            }
        };

        // Zero-consumer diagnostic (plan 092 D5/D6): rig-scoped emits
        // only — zero consumers is the *normal* state for knot- and
        // loom-scoped events (most rigs subscribe to none), so logging
        // those would be per-run noise. Rig-scoped volume is bounded
        // (one `QueueIdle` per burst) and the line appears only when
        // something is wrong. Near-misses name the rename mismatch
        // directly; a plain line covers the no-subscription case.
        if is_rig_scope && matches.is_empty() {
            let detail = if rig_near_misses.is_empty() {
                "0 consumers matched".to_string()
            } else {
                format!(
                    "0 consumers matched; near-miss subscription(s): {}",
                    rig_near_misses.join("; ")
                )
            };
            logging::log_system_event(event_id, &producer, &detail);
        }

        let requests: Vec<DispatchRequest<'_>> = matches
            .into_iter()
            .map(|(consumer_loom, consumer_knot)| DispatchRequest {
                event: &event,
                consumer_knot,
                consumer_loom,
                producer: producer.as_str(),
            })
            .collect();

        dispatch_grouped(
            &*self.event_dispatcher,
            &self.rig_dir,
            &requests,
        )
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::usecases::test_fixtures::KnotBuilder;
    use crate::application::usecases::test_fixtures::MockEventDispatcher;
    use crate::domain::entities::Loom;
    use crate::domain::value_objects::StrandSource;
    use std::sync::{Arc, Mutex};

    /// Recorded dispatch: `(event_id, producer, consumer_knot, consumer_loom, seq)`.
    type Calls = Arc<Mutex<Vec<(String, String, String, String, u32)>>>;

    // A producer-recording dispatcher (the shared MockEventDispatcher drops
    // the producer token, so emitter tests capture it here).
    #[derive(Default)]
    struct RecordingDispatcher {
        calls: Calls,
    }

    impl RecordingDispatcher {
        fn new() -> (Self, Calls) {
            let calls: Calls = Arc::new(Mutex::new(vec![]));
            (Self { calls: calls.clone() }, calls)
        }
    }

    impl EventDispatcherPort for RecordingDispatcher {
        fn dispatch(
            &self,
            event: &AgentEvent,
            consumer_knot: &Knot,
            producer_knot: &str,
            consumer_loom_id: &LoomId,
            rig_dir: &Path,
            seq: u32,
        ) -> Result<PathBuf, PortError> {
            self.calls.lock().unwrap().push((
                event.event_id.clone(),
                producer_knot.to_string(),
                consumer_knot.id.0.clone(),
                consumer_loom_id.0.clone(),
                seq,
            ));
            Ok(crate::domain::knot_file::derive_runtime_root(rig_dir)
                .join(&consumer_loom_id.0)
                .join(&event.event_id)
                .join("event.md"))
        }
    }

    fn event_knot(id: &str, strand_dir: &str) -> Knot {
        KnotBuilder::new(id)
            .with_instructions("react")
            .with_strand_source(StrandSource::from_str(strand_dir).unwrap())
            .build()
    }

    fn fs_knot(id: &str) -> Knot {
        KnotBuilder::new(id).with_instructions("work").build()
    }

    fn store_with(looms: Vec<Loom>) -> LoomStore {
        let store = LoomStore::new();
        for l in looms {
            store.register(l);
        }
        store
    }

    fn loom(id: &str, knots: Vec<Knot>) -> Loom {
        Loom { id: LoomId(id.to_string()), knots }
    }

    fn rig_dir() -> PathBuf {
        PathBuf::from("/project/dev-rig")
    }

    // ── Knot-scoped ────────────────────────────────────────────────────

    #[test]
    fn knot_emit_reaches_knot_loom_and_wildcard_consumers() {
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![
            loom("writing-loom", vec![
                fs_knot("writer"),
                event_knot("knot_consumer", "event:writer:KnotFailed"),
            ]),
            loom("watch-loom", vec![
                event_knot("loom_consumer", "event:writing-loom:KnotFailed"),
                event_knot("wild_consumer", "event:*:KnotFailed"),
                event_knot("other_event", "event:*:KnotCompleted"),
            ]),
        ]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());

        let result = emitter
            .emit(
                &EventScope::knot(
                    LoomId("writing-loom".into()),
                    KnotId("writer".into()),
                    &StrandPath(PathBuf::from("s.md")),
                ),
                "KnotFailed",
                HashMap::new(),
                None,
            )
            .unwrap();

        // 3 consumers matched (knot-level, loom-level, wildcard); the
        // other_event subscription (KnotCompleted) did not.
        assert_eq!(result.len(), 3, "got {:?}", calls.lock().unwrap());
        let names: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.2.clone())
            .collect();
        assert!(names.contains(&"knot_consumer".to_string()));
        assert!(names.contains(&"loom_consumer".to_string()));
        assert!(names.contains(&"wild_consumer".to_string()));
        assert!(!names.contains(&"other_event".to_string()));
    }

    #[test]
    fn knot_emit_non_matching_producer_token_does_not_fire() {
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![loom("writing-loom", vec![
            fs_knot("writer"),
            event_knot("other", "event:some-other-knot:KnotFailed"),
        ])]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        emitter
            .emit(
                &EventScope::knot(
                    LoomId("writing-loom".into()),
                    KnotId("writer".into()),
                    &StrandPath(PathBuf::from("s.md")),
                ),
                "KnotFailed",
                HashMap::new(),
                None,
            )
            .unwrap();
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn knot_emit_excludes_producer_even_for_wildcard() {
        let (dispatcher, calls) = RecordingDispatcher::new();
        // The producer knot `writer` subscribes to its OWN failure both by
        // wildcard and explicitly — neither may re-trigger it.
        let store = store_with(vec![loom("writing-loom", vec![
            event_knot("writer", "event:*:KnotFailed"),
            event_knot("writer2", "event:writer:KnotFailed"),
        ])]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        emitter
            .emit(
                &EventScope::knot(
                    LoomId("writing-loom".into()),
                    KnotId("writer".into()),
                    &StrandPath(PathBuf::from("s.md")),
                ),
                "KnotFailed",
                HashMap::new(),
                None,
            )
            .unwrap();
        let names: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.2.clone())
            .collect();
        assert!(!names.contains(&"writer".to_string()), "self must be excluded");
        // writer2 is a different knot in the same loom → it still fires.
        assert!(names.contains(&"writer2".to_string()));
    }

    // ── Loom-scoped ────────────────────────────────────────────────────

    #[test]
    fn loom_emit_reaches_loom_and_wildcard() {
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![
            loom("writing-loom", vec![fs_knot("writer")]),
            loom("watch-loom", vec![
                event_knot("loom_consumer", "event:writing-loom:LoomStarted"),
                event_knot("wild", "event:*:LoomStarted"),
                event_knot("knot_sub", "event:writer:LoomStarted"),
            ]),
        ]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        emitter
            .emit(
                &EventScope::Loom { loom_id: LoomId("writing-loom".into()) },
                "LoomStarted",
                HashMap::new(),
                None,
            )
            .unwrap();
        let names: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.2.clone())
            .collect();
        assert!(names.contains(&"loom_consumer".to_string()));
        assert!(names.contains(&"wild".to_string()));
        // Knot-level subscription does not match a loom-scoped dispatch.
        assert!(!names.contains(&"knot_sub".to_string()));
    }

    // ── Rig-scoped ─────────────────────────────────────────────────────

    #[test]
    fn rig_emit_reaches_rig_and_wildcard() {
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![loom("watch-loom", vec![
            event_knot("rig_consumer", "event:dev-rig:QueueIdle"),
            event_knot("wild", "event:*:QueueIdle"),
            event_knot("knot_sub", "event:some-knot:QueueIdle"),
        ])]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        emitter
            .emit(&EventScope::Rig, "QueueIdle", HashMap::new(), None)
            .unwrap();
        let names: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.2.clone())
            .collect();
        assert!(names.contains(&"rig_consumer".to_string()));
        assert!(names.contains(&"wild".to_string()));
        assert!(!names.contains(&"knot_sub".to_string()));
    }

    #[test]
    fn rig_emit_producer_token_is_rig_id() {
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![loom("watch-loom", vec![
            event_knot("wild", "event:*:QueueIdle"),
        ])]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        emitter
            .emit(&EventScope::Rig, "QueueIdle", HashMap::new(), None)
            .unwrap();
        let c = calls.lock().unwrap();
        let call = c.iter().find(|c| c.2 == "wild").unwrap();
        assert_eq!(call.1, "dev-rig", "producer frontmatter is the rig id");
    }

    #[test]
    fn rig_emit_static_engine_token_consumer() {
        // Plan 092 D1: the canonical form `event:knot:QueueIdle` matches
        // any rig — the token is the engine, not the rig basename. The
        // deprecated rig-name form keeps dispatching alongside it (D2).
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![loom("watch-loom", vec![
            event_knot("static_consumer", "event:knot:QueueIdle"),
            event_knot("dep_consumer", "event:dev-rig:QueueIdle"),
        ])]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        let result = emitter
            .emit(&EventScope::Rig, "QueueIdle", HashMap::new(), None)
            .unwrap();
        assert_eq!(
            result.len(),
            2,
            "both forms must dispatch: {:?}",
            calls.lock().unwrap()
        );
        let names: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.2.clone())
            .collect();
        assert!(names.contains(&"static_consumer".to_string()));
        assert!(names.contains(&"dep_consumer".to_string()));
    }

    #[test]
    fn rig_emit_producer_token_is_rig_id_for_static_consumer() {
        // Plan 092 D4: the event-file frontmatter keeps carrying the
        // actual rig id regardless of the subscription form.
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![loom("watch-loom", vec![
            event_knot("static_consumer", "event:knot:QueueIdle"),
        ])]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        emitter
            .emit(&EventScope::Rig, "QueueIdle", HashMap::new(), None)
            .unwrap();
        let c = calls.lock().unwrap();
        let call = c.iter().find(|c| c.2 == "static_consumer").unwrap();
        assert_eq!(
            call.1, "dev-rig",
            "producer frontmatter is the rig id, not the subscription token"
        );
    }

    #[test]
    fn rig_emit_near_miss_does_not_dispatch() {
        // Plan 092 D6: a subscription with the right event id but a
        // non-matching producer token (the rename-mismatch signature)
        // matches nothing. The diagnostic is a stderr line (not
        // observable here); the behaviour is the unchanged zero
        // dispatch.
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![loom("watch-loom", vec![
            event_knot("stale", "event:old-rig-name:QueueIdle"),
        ])]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        let result = emitter
            .emit(&EventScope::Rig, "QueueIdle", HashMap::new(), None)
            .unwrap();
        assert!(result.is_empty(), "near-miss must not dispatch");
        assert!(calls.lock().unwrap().is_empty());
    }

    // ── Grouping / seq ─────────────────────────────────────────────────

    #[test]
    fn same_dir_fanout_gets_sequences_singleton_plain() {
        // Two consumers in the SAME loom subscribing to the same event id
        // share one target directory → seq 1, 2. A third in a different
        // loom is a singleton → seq 0.
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![
            loom("watch-loom", vec![
                event_knot("c1", "event:*:KnotFailed"),
                event_knot("c2", "event:*:KnotFailed"),
            ]),
            loom("solo-loom", vec![event_knot("solo", "event:*:KnotFailed")]),
        ]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        emitter
            .emit(
                &EventScope::knot(
                    LoomId("writing-loom".into()),
                    KnotId("writer".into()),
                    &StrandPath(PathBuf::from("s.md")),
                ),
                "KnotFailed",
                HashMap::new(),
                None,
            )
            .unwrap();
        let c = calls.lock().unwrap();
        let seq_of = |name: &str| c.iter().find(|x| x.2 == name).unwrap().4;
        let watch: Vec<u32> = c
            .iter()
            .filter(|x| x.3 == "watch-loom")
            .map(|x| x.4)
            .collect();
        assert_eq!(watch, vec![1, 2], "same-directory fan-out must use seq 1..N");
        assert_eq!(seq_of("c1"), 1);
        assert_eq!(seq_of("c2"), 2);
        assert_eq!(seq_of("solo"), 0, "singleton keeps the plain name");
    }

    #[test]
    fn emitter_has_no_events_dispatched_path() {
        // The emitter dispatches only the event-id it is handed; it is never
        // given EventsDispatched. This test pins that EventsDispatched is
        // simply a string like any other — there is no special-casing, and
        // the caller controls the id (see Notes: never emitted).
        let (dispatcher, calls) = RecordingDispatcher::new();
        let store = store_with(vec![loom("w", vec![
            event_knot("c", "event:*:EventsDispatched"),
        ])]);
        let emitter =
            SystemEventEmitter::new(store, Arc::new(dispatcher), rig_dir());
        // We never call it this way in production; the guard is that no
        // emission site passes "EventsDispatched".
        let n = emitter
            .emit(
                &EventScope::Loom { loom_id: LoomId("w".into()) },
                "SomeRealEvent",
                HashMap::new(),
                None,
            )
            .unwrap();
        assert!(n.is_empty(), "EventsDispatched is not the dispatched id here");
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn dispatch_grouped_preserves_singleton_vs_batch_with_mock() {
        let (dispatcher, _calls) = MockEventDispatcher::new();
        let event = AgentEvent {
            event_id: "E".into(),
            occurred: true,
            payload: HashMap::new(),
            body: None,
        };
        let solo = fs_knot("solo");
        let loom_id = LoomId("l-loom".to_string());
        let requests = vec![DispatchRequest {
            event: &event,
            consumer_knot: &solo,
            consumer_loom: &loom_id,
            producer: "p",
        }];
        let out =
            dispatch_grouped(&dispatcher, Path::new("/p/rig"), &requests)
                .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "E");
        assert_eq!(out[0].1, "solo");
    }
}
