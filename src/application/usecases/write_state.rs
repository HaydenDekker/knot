//! Use case: snapshot the rig's current state to
//! `tie-offs/<rig-basename>/state.json` (the rig's runtime root).

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::ports::{
    AgentProfileRepository, LoomLogPort, ModelRegistryPort, PortError,
    StateWriterPort, StrandEventQueue,
};
use crate::application::store::LoomStore;
use crate::domain::entities::{
    KnotId, LoomId, RigState, RigStateKnot, RigStateLoom, RigStateProfile,
    RigStateStrandQueueEntry,
};
use crate::domain::events::LoomEvent;
use crate::domain::knot_file::derive_runtime_root;
#[cfg(test)]
use crate::domain::pending_event::PendingEvent;
use crate::domain::state_change::diff_state;

use super::types::format_timestamp;

/// Use case: snapshot the rig's current state to the runtime root's
/// `state.json` (`tie-offs/<rig-basename>/state.json`).
///
/// Reads from `LoomStore` (looms + knots), `AgentProfileRepository`
/// (profiles), and `LoomLogPort` (knot processing status from logs),
/// then serialises everything into a `RigState` and delegates to
/// `StateWriterPort` for atomic write.
///
/// Type alias for the strand queue held by WriteState.
type StrandQueueRef = Arc<std::sync::Mutex<Option<Arc<dyn StrandEventQueue>>>>;

/// This is the core logic called by the background state writer task.
pub struct WriteState {
    store: LoomStore,
    log_port: Arc<dyn LoomLogPort>,
    profile_repo: Arc<dyn AgentProfileRepository>,
    /// Model registry — loaded fresh on each state build so that
    /// `model-ref` profiles show their resolved provider/model and a
    /// registry edit is reflected in the next state write (no restart).
    model_registry: Arc<dyn ModelRegistryPort>,
    state_writer: Arc<dyn StateWriterPort>,
    rig_dir: PathBuf,
    strand_queue: Option<StrandQueueRef>,
    /// The last state actually written to disk (`None` before the first
    /// write of this run). Change-driven writes (plan 083): a tick whose
    /// built state is content-equal to this (ignoring `updated_at`) and
    /// whose `state.json` still exists on disk is skipped — no write,
    /// no mtime churn, no `[STATE]` line.
    last_written: Option<RigState>,
}

impl WriteState {
    /// Create a new `WriteState` use case.
    ///
    /// `strand_queue` is an `Arc<Mutex<Option<Arc<dyn StrandEventQueue>>>>`
    /// shared with the event pipeline. When `Some`, `build_state()` takes
    /// a fresh snapshot of the queue on each call.
    pub fn new(
        store: LoomStore,
        log_port: Arc<dyn LoomLogPort>,
        profile_repo: Arc<dyn AgentProfileRepository>,
        model_registry: Arc<dyn ModelRegistryPort>,
        state_writer: Arc<dyn StateWriterPort>,
        rig_dir: PathBuf,
        strand_queue: StrandQueueRef,
    ) -> Self {
        Self {
            store,
            log_port,
            profile_repo,
            model_registry,
            state_writer,
            rig_dir,
            strand_queue: Some(strand_queue),
            last_written: None,
        }
    }

    /// Build a `RigState` snapshot from current in-memory state.
    ///
    /// Returns the `RigState` — caller is responsible for writing it.
    pub fn build_state(&self) -> Result<RigState, PortError> {
        let looms = self.store.list();
        let profiles = self.profile_repo.list()?;

        let rig_state_looms: Vec<RigStateLoom> = looms
            .into_iter()
            .map(|loom| {
                let knots: Vec<RigStateKnot> = loom
                    .knots
                    .into_iter()
                    .map(|knot| self.derive_knot_state(&loom.id, &knot.id))
                    .collect();
                RigStateLoom {
                    id: loom.id.0,
                    knots,
                }
            })
            .collect();

        // Fresh registry read per state build — resolved models track
        // live edits to rig/models.yml.
        let registry = self.model_registry.load()?;

        let rig_state_profiles: Vec<RigStateProfile> = profiles
            .into_iter()
            .map(|p| {
                let (model_ref, provider, model, thinking_level) =
                    match p.model_ref.as_deref() {
                        Some(alias) => match registry.resolve(alias) {
                            Some(resolved) => (
                                Some(alias.to_string()),
                                Some(resolved.provider.clone()),
                                Some(resolved.model.clone()),
                                // Effective level: the profile's own level
                                // (override) wins over the alias default.
                                p.thinking_level.or(resolved.thinking_level),
                            ),
                            None => {
                                eprintln!(
                                    "WARNING: profile '{}' references unknown model alias '{}' — provider/model shown as null in state (check rig/models.yml)",
                                    p.name, alias
                                );
                                // Alias unresolvable — only the profile's
                                // own level can apply.
                                (
                                    Some(alias.to_string()),
                                    None,
                                    None,
                                    p.thinking_level,
                                )
                            }
                        },
                        // Direct-spec profiles use their own level only —
                        // the registry is not consulted.
                        None => (None, p.provider, p.model, p.thinking_level),
                    };
                RigStateProfile {
                    name: p.name,
                    model_ref,
                    provider,
                    model,
                    thinking_level,
                    timeout: p.timeout,
                }
            })
            .collect();

        let rig_path = self.rig_dir.to_string_lossy().to_string();

        let strand_queue_entries = self.build_queue_entries();

        Ok(RigState {
            rig_path,
            looms: rig_state_looms,
            profiles: rig_state_profiles,
            strand_queue: strand_queue_entries,
            updated_at: format_timestamp(),
        })
    }

    /// Execute: build state and write to disk atomically — but only
    /// when the content actually changed (plan 083, change-driven
    /// state writes).
    ///
    /// Skip rule: when the built state is equal to the last written
    /// state (every field except `updated_at`, which is the write
    /// tick) and `state.json` still exists on disk, nothing is written
    /// and no `[STATE]` line is emitted. When the content changed, the
    /// state is written and the delta is logged as `[KNOT][STATE]`
    /// lines (the first write of the run is the baseline line). A
    /// content-equal state whose `state.json` was deleted externally
    /// is rewritten (no delta lines — the content did not change).
    pub fn execute(&mut self) -> Result<(), PortError> {
        let state = self.build_state()?;

        let state_path = derive_runtime_root(&self.rig_dir).join("state.json");
        let content_changed = match &self.last_written {
            Some(prev) => !Self::equal_ignoring_write_tick(prev, &state),
            None => true,
        };
        let file_missing = !state_path.exists();

        if !content_changed && !file_missing {
            // Nothing new to say and the file is still there — skip
            // the write entirely (no mtime churn, no log line).
            return Ok(());
        }

        let change = diff_state(self.last_written.as_ref(), &state);
        let first_write = self.last_written.is_none();

        self.state_writer.write_state(&state)?;
        self.last_written = Some(state.clone());
        crate::adapters::service_log::log_state_write_lines(
            first_write,
            &change,
            &state,
        );
        Ok(())
    }

    /// Compare two states on everything except `updated_at` (the write
    /// tick, which changes on every build and is never "the change").
    fn equal_ignoring_write_tick(a: &RigState, b: &RigState) -> bool {
        let mut a = a.clone();
        let mut b = b.clone();
        a.updated_at.clear();
        b.updated_at.clear();
        a == b
    }

    /// Build the strand queue entries from the current queue snapshot.
    ///
    /// Locks the outer mutex to get the inner `Arc<dyn StrandEventQueue>`,
    /// then takes a snapshot of the queue contents and maps each
    /// `PendingEvent` to a `RigStateStrandQueueEntry`.
    fn build_queue_entries(&self) -> Vec<RigStateStrandQueueEntry> {
        let Some(queue_ref) = &self.strand_queue else {
            return Vec::new();
        };

        let inner = queue_ref.lock().unwrap();
        let Some(queue) = inner.as_ref() else {
            return Vec::new();
        };

        queue
            .snapshot()
            .into_iter()
            .map(|pending| {
                let kind = match pending.kind.as_str() {
                    "Created" => "created",
                    "Modified" => "modified",
                    "Deleted" => "deleted",
                    other => other,
                };

                RigStateStrandQueueEntry {
                    strand_path: pending.strand_path,
                    loom_id: pending.loom_id,
                    knot_id: pending.knot_id,
                    event_kind: kind.to_string(),
                    queued_at: pending.queued_at,
                }
            })
            .collect()
    }

    /// Derive the processing status for a knot from its loom-log.
    ///
    /// Walks the loom-log events for the given loom and finds the
    /// latest event referencing the given knot, then maps it to a
    /// status string.
    fn derive_knot_state(&self, loom_id: &LoomId, knot_id: &KnotId) -> RigStateKnot {
        let events = match self.log_port.read_all(loom_id) {
            Ok(e) => e,
            Err(e) => {
                eprintln!(
                    "WARN: could not read loom-log for {} (knot {}): {}",
                    loom_id.0, knot_id.0, e,
                );
                return RigStateKnot {
                    id: knot_id.0.clone(),
                    status: "idle".to_string(),
                    last_strand_path: None,
                    last_tie_off_path: None,
                    last_error: None,
                    last_event_at: None,
                };
            }
        };

        // Find the latest event referencing this knot
        let latest = events.iter().rev().find(|event| match event {
            LoomEvent::KnotRegistered { knot_id: kid, .. }
            | LoomEvent::KnotProcessing { knot_id: kid, .. }
            | LoomEvent::KnotCompleted { knot_id: kid, .. }
            | LoomEvent::KnotFailed { knot_id: kid, .. } => kid == knot_id,
            _ => false,
        });

        match latest {
            Some(LoomEvent::KnotRegistered { timestamp, .. }) => RigStateKnot {
                id: knot_id.0.clone(),
                status: "idle".to_string(),
                last_strand_path: None,
                last_tie_off_path: None,
                last_error: None,
                last_event_at: Some(timestamp.clone()),
            },
            Some(LoomEvent::KnotProcessing {
                strand_path, timestamp, ..
            }) => RigStateKnot {
                id: knot_id.0.clone(),
                status: "processing".to_string(),
                last_strand_path: Some(strand_path.0.display().to_string()),
                last_tie_off_path: None,
                last_error: None,
                last_event_at: Some(timestamp.clone()),
            },
            Some(LoomEvent::KnotCompleted {
                strand_path,
                tie_off_path,
                timestamp,
                ..
            }) => RigStateKnot {
                id: knot_id.0.clone(),
                status: "completed".to_string(),
                last_strand_path: Some(strand_path.0.display().to_string()),
                last_tie_off_path: Some(tie_off_path.0.display().to_string()),
                last_error: None,
                last_event_at: Some(timestamp.clone()),
            },
            Some(LoomEvent::KnotFailed {
                strand_path,
                error,
                timestamp,
                ..
            }) => RigStateKnot {
                id: knot_id.0.clone(),
                status: "failed".to_string(),
                last_strand_path: Some(strand_path.0.display().to_string()),
                last_tie_off_path: None,
                last_error: Some(error.clone()),
                last_event_at: Some(timestamp.clone()),
            },
            // No events for this knot yet — idle
            _ => RigStateKnot {
                id: knot_id.0.clone(),
                status: "idle".to_string(),
                last_strand_path: None,
                last_tie_off_path: None,
                last_error: None,
                last_event_at: None,
            },
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod write_state_tests {
    use super::*;
    use crate::domain::entities::KnotId;
    use crate::domain::value_objects::AgentProfile;
    use crate::application::store::LoomStore;
    use crate::domain::entities::{Knot, Loom, LoomId, StrandPath, TieOffPath};
    use crate::domain::value_objects::StrandSource;
    use crate::domain::value_objects::{ModelRef, ModelRegistry, ThinkingLevel};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    /// In-memory mock of `LoomLogPort` for WriteState tests.
    #[derive(Default)]
    struct MockLoomLogForState {
        events: Arc<RwLock<HashMap<String, Vec<LoomEvent>>>>,
    }

    impl Clone for MockLoomLogForState {
        fn clone(&self) -> Self {
            Self {
                events: Arc::clone(&self.events),
            }
        }
    }

    impl MockLoomLogForState {
        fn add_events(&self, loom_id: &str, events: Vec<LoomEvent>) {
            self.events.write().unwrap().insert(loom_id.to_string(), events);
        }
    }

    impl LoomLogPort for MockLoomLogForState {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(
            &self,
            loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(self
                .events
                .read()
                .unwrap()
                .get(&loom_id.0)
                .cloned()
                .unwrap_or_default())
        }
    }

    /// In-memory mock of `AgentProfileRepository` for WriteState tests.
    #[derive(Default)]
    struct MockProfileRepoForState {
        profiles: Arc<RwLock<Vec<AgentProfile>>>,
    }

    impl Clone for MockProfileRepoForState {
        fn clone(&self) -> Self {
            Self {
                profiles: Arc::clone(&self.profiles),
            }
        }
    }

    impl MockProfileRepoForState {
        fn add_profile(&self, profile: AgentProfile) {
            self.profiles.write().unwrap().push(profile);
        }
    }

    impl AgentProfileRepository for MockProfileRepoForState {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .read()
                .unwrap()
                .iter()
                .find(|p| p.name == name)
                .cloned())
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(self.profiles.read().unwrap().clone())
        }

    }

    /// In-memory mock of `StateWriterPort` for WriteState tests.
    #[derive(Default)]
    struct MockStateWriterForState {
        writes: Arc<RwLock<Vec<RigState>>>,
    }

    impl Clone for MockStateWriterForState {
        fn clone(&self) -> Self {
            Self {
                writes: Arc::clone(&self.writes),
            }
        }
    }

    impl MockStateWriterForState {
        fn last_write(&self) -> Option<RigState> {
            self.writes.read().unwrap().last().cloned()
        }
    }

    impl StateWriterPort for MockStateWriterForState {
        fn write_state(&self, state: &RigState) -> Result<(), PortError> {
            self.writes.write().unwrap().push(state.clone());
            Ok(())
        }
    }

    /// Build a loom for testing.
    fn test_loom(id: &str) -> Loom {
        Loom {
            id: LoomId(id.to_string()),
            knots: vec![
                Knot {
                    id: KnotId("k1".to_string()),
                    agent_profile_ref: "fast".to_string(),
                    prompt_template: crate::domain::value_objects::PromptTemplate {
                        instructions: "Review.".to_string(),
                    },
                    git_versioned: true,
                    strand_source: StrandSource::Filesystem(PathBuf::from("strands")),
                    event_description: None,
                },
            ],
        }
    }

    fn build_use_case() -> (
        WriteState,
        LoomStore,
        Arc<MockLoomLogForState>,
        Arc<MockProfileRepoForState>,
        Arc<MockStateWriterForState>,
    ) {
        let store = LoomStore::new();
        let log_port = Arc::new(MockLoomLogForState::default());
        let profile_repo = Arc::new(MockProfileRepoForState::default());
        let model_registry = Arc::new(
            crate::application::usecases::test_fixtures::MockModelRegistry::default(),
        );
        let state_writer = Arc::new(MockStateWriterForState::default());
        let rig_dir = PathBuf::from("/test/rig");

        let strand_queue: StrandQueueRef =
            Arc::new(std::sync::Mutex::new(None));

        let use_case = WriteState::new(
            store.clone(),
            log_port.clone(),
            profile_repo.clone(),
            model_registry,
            state_writer.clone(),
            rig_dir,
            strand_queue,
        );

        (use_case, store, log_port, profile_repo, state_writer)
    }

    #[test]
    fn build_state_empty_rig() {
        let (uc, _, _, _, _) = build_use_case();

        let state = uc.build_state().unwrap();
        assert_eq!(state.rig_path, "/test/rig");
        assert!(state.looms.is_empty());
        assert!(state.profiles.is_empty());
    }

    #[test]
    fn build_state_with_looms_and_profiles() {
        let (uc, store, _, profile_repo, _) = build_use_case();

        store.register(test_loom("prds"));
        profile_repo.add_profile(
            AgentProfile::new(
                "fast".to_string(),
                "openai".to_string(),
                "gpt-4o".to_string(),
                "You are fast.".to_string(),
            )
            .unwrap(),
        );

        let state = uc.build_state().unwrap();
        assert_eq!(state.looms.len(), 1);
        assert_eq!(state.looms[0].id, "prds");
        assert_eq!(state.looms[0].knots.len(), 1);
        assert_eq!(state.looms[0].knots[0].id, "k1");
        assert_eq!(state.looms[0].knots[0].status, "idle");
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.profiles[0].name, "fast");
        assert_eq!(state.profiles[0].model_ref, None);
        assert_eq!(state.profiles[0].provider.as_deref(), Some("openai"));
        assert_eq!(state.profiles[0].model.as_deref(), Some("gpt-4o"));
    }

    /// Alias profile: state shows `model-ref` plus the resolved
    /// provider/model from the registry.
    #[test]
    fn build_state_alias_profile_shows_resolved_model() {
        let store = LoomStore::new();
        let log_port: Arc<dyn LoomLogPort> = Arc::new(MockLoomLogForState::default());
        let concrete_repo = Arc::new(MockProfileRepoForState::default());
        concrete_repo.add_profile(
            AgentProfile::with_model_ref(
                "fast".to_string(),
                "fast".to_string(),
                "You are fast.".to_string(),
            )
            .unwrap(),
        );
        let profile_repo: Arc<dyn AgentProfileRepository> = concrete_repo;
        let registry = Arc::new(
            crate::application::usecases::test_fixtures::MockModelRegistry::default(),
        );
        use crate::domain::value_objects::{ModelRef, ModelRegistry};
        let mut reg = ModelRegistry::new();
        reg.entries.insert(
            "fast".to_string(),
            ModelRef {
                provider: "anthropic".to_string(),
                model: "claude-sonnet".to_string(),
                thinking_level: None,
            },
        );
        registry.set_registry(reg);
        let state_writer: Arc<dyn StateWriterPort> =
            Arc::new(MockStateWriterForState::default());
        let strand_queue: StrandQueueRef =
            Arc::new(std::sync::Mutex::new(None));

        let uc = WriteState::new(
            store.clone(),
            log_port,
            profile_repo,
            registry,
            state_writer,
            PathBuf::from("/test/rig"),
            strand_queue,
        );

        let state = uc.build_state().unwrap();
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.profiles[0].name, "fast");
        assert_eq!(state.profiles[0].model_ref.as_deref(), Some("fast"));
        assert_eq!(state.profiles[0].provider.as_deref(), Some("anthropic"));
        assert_eq!(state.profiles[0].model.as_deref(), Some("claude-sonnet"));

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"model-ref\":\"fast\""));
    }

    /// Direct-spec profile: state shows `model_ref` null and the
    /// profile's own provider/model.
    #[test]
    fn build_state_direct_profile_has_null_model_ref() {
        let (uc, _store, _log_port, profile_repo, _writer) = build_use_case();

        profile_repo.add_profile(
            AgentProfile::new(
                "legacy".to_string(),
                "openai".to_string(),
                "gpt-4o".to_string(),
                "You are legacy.".to_string(),
            )
            .unwrap(),
        );

        let state = uc.build_state().unwrap();
        assert_eq!(state.profiles[0].model_ref, None);
        assert_eq!(state.profiles[0].provider.as_deref(), Some("openai"));
        assert_eq!(state.profiles[0].model.as_deref(), Some("gpt-4o"));
    }

    /// Unknown alias: provider/model are null in state (warning is
    /// logged); the profile entry still carries the `model-ref`.
    #[test]
    fn build_state_unknown_alias_shows_null_provider_and_model() {
        let (uc, _store, _log_port, profile_repo, _writer) = build_use_case();
        // Registry stays empty (from build_use_case) — alias unresolvable.

        profile_repo.add_profile(
            AgentProfile::with_model_ref(
                "ghostly".to_string(),
                "ghost".to_string(),
                "You are ghostly.".to_string(),
            )
            .unwrap(),
        );

        let state = uc.build_state().unwrap();
        assert_eq!(state.profiles[0].model_ref.as_deref(), Some("ghost"));
        assert_eq!(state.profiles[0].provider, None);
        assert_eq!(state.profiles[0].model, None);

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"provider\":null"));
        assert!(json.contains("\"model\":null"));
    }

    /// Build a `WriteState` use case with a single profile and a given model
    /// registry (for testing effective thinking-level resolution).
    fn build_uc_with_profile_and_registry(
        profile: AgentProfile,
        registry: ModelRegistry,
    ) -> WriteState {
        let store = LoomStore::new();
        let log_port: Arc<dyn LoomLogPort> = Arc::new(MockLoomLogForState::default());
        let concrete_repo = Arc::new(MockProfileRepoForState::default());
        concrete_repo.add_profile(profile);
        let profile_repo: Arc<dyn AgentProfileRepository> = concrete_repo;
        let model_registry = Arc::new(
            crate::application::usecases::test_fixtures::MockModelRegistry::default(),
        );
        model_registry.set_registry(registry);
        let state_writer: Arc<dyn StateWriterPort> =
            Arc::new(MockStateWriterForState::default());
        let strand_queue: StrandQueueRef =
            Arc::new(std::sync::Mutex::new(None));

        WriteState::new(
            store,
            log_port,
            profile_repo,
            model_registry,
            state_writer,
            PathBuf::from("/test/rig"),
            strand_queue,
        )
    }

    // ── Effective thinking-level in state Tests ─────────────────────

    /// Profile-only: a profile that sets its own thinking-level (direct-spec,
    /// so no alias is consulted) shows the profile's level in state.
    #[test]
    fn build_state_thinking_level_profile_only() {
        let profile = AgentProfile::new(
            "deep".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "Deep review.".to_string(),
        )
        .unwrap()
        .with_thinking_level(Some(ThinkingLevel::XHigh));
        let registry = ModelRegistry::new();
        let uc = build_uc_with_profile_and_registry(profile, registry);

        let state = uc.build_state().unwrap();
        assert_eq!(
            state.profiles[0].thinking_level,
            Some(ThinkingLevel::XHigh)
        );

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"thinking-level\":\"xhigh\""));
    }

    /// Alias-default: a profile with no thinking-level, whose alias sets one,
    /// shows the alias's level in state.
    #[test]
    fn build_state_thinking_level_alias_default() {
        let profile = AgentProfile::with_model_ref(
            "analyst".to_string(),
            "frontier".to_string(),
            "You are an analyst.".to_string(),
        )
        .unwrap();
        let mut registry = ModelRegistry::new();
        registry.entries.insert(
            "frontier".to_string(),
            ModelRef {
                provider: "anthropic".to_string(),
                model: "claude-sonnet".to_string(),
                thinking_level: Some(ThinkingLevel::High),
            },
        );
        let uc = build_uc_with_profile_and_registry(profile, registry);

        let state = uc.build_state().unwrap();
        assert_eq!(
            state.profiles[0].thinking_level,
            Some(ThinkingLevel::High)
        );

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"thinking-level\":\"high\""));
    }

    /// Profile-override: a profile with a level wins over its alias's level.
    #[test]
    fn build_state_thinking_level_profile_overrides_alias() {
        let profile = AgentProfile::with_model_ref(
            "analyst".to_string(),
            "frontier".to_string(),
            "You are an analyst.".to_string(),
        )
        .unwrap()
        .with_thinking_level(Some(ThinkingLevel::Low));
        let mut registry = ModelRegistry::new();
        registry.entries.insert(
            "frontier".to_string(),
            ModelRef {
                provider: "anthropic".to_string(),
                model: "claude-sonnet".to_string(),
                thinking_level: Some(ThinkingLevel::High),
            },
        );
        let uc = build_uc_with_profile_and_registry(profile, registry);

        let state = uc.build_state().unwrap();
        assert_eq!(
            state.profiles[0].thinking_level,
            Some(ThinkingLevel::Low)
        );

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"thinking-level\":\"low\""));
    }

    /// Omitted: when neither the profile nor the alias sets a level, the
    /// `thinking-level` key is absent from the JSON (not null) — matching the
    /// `timeout` convention.
    #[test]
    fn build_state_thinking_level_omitted_when_neither_sets_it() {
        let profile = AgentProfile::with_model_ref(
            "fast".to_string(),
            "fast".to_string(),
            "You are fast.".to_string(),
        )
        .unwrap();
        let mut registry = ModelRegistry::new();
        registry.entries.insert(
            "fast".to_string(),
            ModelRef {
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                thinking_level: None,
            },
        );
        let uc = build_uc_with_profile_and_registry(profile, registry);

        let state = uc.build_state().unwrap();
        assert_eq!(state.profiles[0].thinking_level, None);

        let json = serde_json::to_string(&state).unwrap();
        assert!(
            !json.contains("thinking-level"),
            "thinking-level key must be absent, got: {json}"
        );
    }

    #[test]
    fn derive_knot_status_idle_from_registration() {
        let (uc, store, log_port, _, _) = build_use_case();
        store.register(test_loom("prds"));

        // Add KnotRegistered event
        log_port.add_events(
            "prds",
            vec![LoomEvent::KnotRegistered {
                loom_id: LoomId("prds".to_string()),
                knot_id: KnotId("k1".to_string()),
                timestamp: "2026-06-18T10:00:00Z".to_string(),
            }],
        );

        let state = uc.build_state().unwrap();
        let knot = &state.looms[0].knots[0];
        assert_eq!(knot.status, "idle");
        assert_eq!(
            knot.last_event_at,
            Some("2026-06-18T10:00:00Z".to_string())
        );
    }

    #[test]
    fn derive_knot_status_completed_from_log() {
        let (uc, store, log_port, _, _) = build_use_case();
        store.register(test_loom("prds"));

        log_port.add_events(
            "prds",
            vec![
                LoomEvent::KnotRegistered {
                    loom_id: LoomId("prds".to_string()),
                    knot_id: KnotId("k1".to_string()),
                    timestamp: "2026-06-18T10:00:00Z".to_string(),
                },
                LoomEvent::KnotCompleted {
                    loom_id: LoomId("prds".to_string()),
                    knot_id: KnotId("k1".to_string()),
                    strand_path: StrandPath(PathBuf::from("input.md")),
                    tie_off_path: TieOffPath(PathBuf::from("output.md")),
                    timestamp: "2026-06-18T10:05:00Z".to_string(),
                },
            ],
        );

        let state = uc.build_state().unwrap();
        let knot = &state.looms[0].knots[0];
        assert_eq!(knot.status, "completed");
        assert_eq!(
            knot.last_strand_path,
            Some("input.md".to_string())
        );
        assert_eq!(
            knot.last_tie_off_path,
            Some("output.md".to_string())
        );
    }

    #[test]
    fn derive_knot_status_failed_from_log() {
        let (uc, store, log_port, _, _) = build_use_case();
        store.register(test_loom("prds"));

        log_port.add_events(
            "prds",
            vec![
                LoomEvent::KnotRegistered {
                    loom_id: LoomId("prds".to_string()),
                    knot_id: KnotId("k1".to_string()),
                    timestamp: "2026-06-18T10:00:00Z".to_string(),
                },
                LoomEvent::KnotFailed {
                    loom_id: LoomId("prds".to_string()),
                    knot_id: KnotId("k1".to_string()),
                    strand_path: StrandPath(PathBuf::from("input.md")),
                    error: "timeout".to_string(),
                    timestamp: "2026-06-18T10:05:00Z".to_string(),
                },
            ],
        );

        let state = uc.build_state().unwrap();
        let knot = &state.looms[0].knots[0];
        assert_eq!(knot.status, "failed");
        assert_eq!(knot.last_error, Some("timeout".to_string()));
    }

    #[test]
    fn derive_knot_status_processing_from_log() {
        let (uc, store, log_port, _, _) = build_use_case();
        store.register(test_loom("prds"));

        log_port.add_events(
            "prds",
            vec![
                LoomEvent::KnotRegistered {
                    loom_id: LoomId("prds".to_string()),
                    knot_id: KnotId("k1".to_string()),
                    timestamp: "2026-06-18T10:00:00Z".to_string(),
                },
                LoomEvent::KnotProcessing {
                    loom_id: LoomId("prds".to_string()),
                    knot_id: KnotId("k1".to_string()),
                    strand_path: StrandPath(PathBuf::from("input.md")),
                    timestamp: "2026-06-18T10:01:00Z".to_string(),
                },
            ],
        );

        let state = uc.build_state().unwrap();
        let knot = &state.looms[0].knots[0];
        assert_eq!(knot.status, "processing");
        assert_eq!(
            knot.last_strand_path,
            Some("input.md".to_string())
        );
    }

    #[test]
    fn derive_knot_status_latest_event_wins() {
        let (uc, store, log_port, _, _) = build_use_case();
        store.register(test_loom("prds"));

        // Completed then failed — latest should be failed
        log_port.add_events(
            "prds",
            vec![
                LoomEvent::KnotCompleted {
                    loom_id: LoomId("prds".to_string()),
                    knot_id: KnotId("k1".to_string()),
                    strand_path: StrandPath(PathBuf::from("a.md")),
                    tie_off_path: TieOffPath(PathBuf::from("out-a.md")),
                    timestamp: "2026-06-18T10:00:00Z".to_string(),
                },
                LoomEvent::KnotFailed {
                    loom_id: LoomId("prds".to_string()),
                    knot_id: KnotId("k1".to_string()),
                    strand_path: StrandPath(PathBuf::from("b.md")),
                    error: "boom".to_string(),
                    timestamp: "2026-06-18T10:05:00Z".to_string(),
                },
            ],
        );

        let state = uc.build_state().unwrap();
        let knot = &state.looms[0].knots[0];
        assert_eq!(knot.status, "failed");
        assert_eq!(
            knot.last_strand_path,
            Some("b.md".to_string())
        );
    }

    #[test]
    fn execute_builds_and_writes_state() {
        let (mut uc, store, _, profile_repo, writer) = build_use_case();

        store.register(test_loom("prds"));
        profile_repo.add_profile(
            AgentProfile::new(
                "fast".to_string(),
                "openai".to_string(),
                "gpt-4o".to_string(),
                "Fast.".to_string(),
            )
            .unwrap(),
        );

        uc.execute().unwrap();

        let written = writer.last_write().unwrap();
        assert_eq!(written.rig_path, "/test/rig");
        assert_eq!(written.looms.len(), 1);
        assert_eq!(written.profiles.len(), 1);
    }

    #[test]
    fn execute_handles_log_port_error_gracefully() {
        let store = LoomStore::new();
        let log_port: Arc<dyn LoomLogPort> = Arc::new(MockLoomLogForState::default());
        let profile_repo: Arc<dyn AgentProfileRepository> =
            Arc::new(MockProfileRepoForState::default());
        let state_writer: Arc<dyn StateWriterPort> =
            Arc::new(MockStateWriterForState::default());
        let rig_dir = PathBuf::from("/test/rig");

        let strand_queue: StrandQueueRef =
            Arc::new(std::sync::Mutex::new(None));

        let model_registry = Arc::new(
            crate::application::usecases::test_fixtures::MockModelRegistry::default(),
        );
        let mut uc = WriteState::new(
            store.clone(),
            log_port,
            profile_repo,
            model_registry,
            state_writer,
            rig_dir,
            strand_queue,
        );

        // Even with no log events, the state should build (knots default to idle)
        store.register(test_loom("prds"));
        let result = uc.execute();
        assert!(result.is_ok());
    }

    #[test]
    fn multiple_looms_in_state() {
        let (mut uc, store, _, _, writer) = build_use_case();

        store.register(test_loom("prds"));
        let loom2 = test_loom("docs");
        store.register(loom2);

        uc.execute().unwrap();

        let written = writer.last_write().unwrap();
        let ids: Vec<_> = written.looms.iter().map(|l| &l.id).collect();
        assert!(ids.contains(&&"prds".to_string()));
        assert!(ids.contains(&&"docs".to_string()));
    }

    #[test]
    fn rig_state_json_matches_spec() {
        let (mut uc, store, log_port, profile_repo, writer) = build_use_case();

        store.register(test_loom("my-loom"));
        log_port.add_events(
            "my-loom",
            vec![LoomEvent::KnotRegistered {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
                timestamp: "2026-06-18T10:00:00Z".to_string(),
            }],
        );
        profile_repo.add_profile(
            AgentProfile::new(
                "fast".to_string(),
                "openai".to_string(),
                "gpt-4o".to_string(),
                "Fast.".to_string(),
            )
            .unwrap(),
        );

        uc.execute().unwrap();

        let written = writer.last_write().unwrap();

        // Verify the JSON matches the spec shape
        let json = serde_json::to_string(&written).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Top-level keys
        assert!(value.get("rig_path").is_some());
        assert!(value.get("looms").is_some());
        assert!(value.get("profiles").is_some());
        assert!(value.get("strand_queue").is_some());
        assert!(value.get("updated_at").is_some());

        // strand_queue is an empty array
        let queue = value["strand_queue"].as_array().unwrap();
        assert!(queue.is_empty());

        // Loom structure
        let looms = value["looms"].as_array().unwrap();
        assert_eq!(looms[0]["id"], "my-loom");
        let knots = looms[0]["knots"].as_array().unwrap();
        assert_eq!(knots[0]["id"], "k1");
        assert_eq!(knots[0]["status"], "idle");

        // Profile structure
        let profiles = value["profiles"].as_array().unwrap();
        assert_eq!(profiles[0]["name"], "fast");
        // Direct-spec profile: model-ref is null, provider/model resolve
        // to the profile's own values.
        assert_eq!(profiles[0]["model-ref"], serde_json::Value::Null);
        assert_eq!(profiles[0]["provider"], "openai");
        assert_eq!(profiles[0]["model"], "gpt-4o");
    }

    // ── Strand Queue Tests ─────────────────────────────────────────

    #[test]
    fn build_state_no_queue() {
        // strand_queue is None (backward compat) — strand_queue should be []
        let (uc, _, _, _, _) = build_use_case();

        let state = uc.build_state().unwrap();
        assert!(
            state.strand_queue.is_empty(),
            "strand_queue should be empty when no queue is wired"
        );
    }

    #[test]
    fn build_state_empty_queue() {
        // Queue is present but empty — strand_queue should be []
        let store = LoomStore::new();
        let log_port: Arc<dyn LoomLogPort> = Arc::new(MockLoomLogForState::default());
        let profile_repo: Arc<dyn AgentProfileRepository> =
            Arc::new(MockProfileRepoForState::default());
        let state_writer: Arc<dyn StateWriterPort> =
            Arc::new(MockStateWriterForState::default());
        let rig_dir = PathBuf::from("/test/rig");

        let queue: Arc<dyn StrandEventQueue> =
            Arc::new(crate::application::in_memory_event_queue::InMemoryEventQueue::new());
        let strand_queue: StrandQueueRef =
            Arc::new(std::sync::Mutex::new(Some(queue)));

        let model_registry = Arc::new(
            crate::application::usecases::test_fixtures::MockModelRegistry::default(),
        );
        let uc = WriteState::new(
            store.clone(),
            log_port,
            profile_repo,
            model_registry,
            state_writer,
            rig_dir,
            strand_queue,
        );

        let state = uc.build_state().unwrap();
        assert!(
            state.strand_queue.is_empty(),
            "strand_queue should be empty when queue has no events"
        );
    }

    /// Build a `PendingEvent` for testing.
    fn make_pending(
        kind: &str,
        loom: &str,
        knot: &str,
        path: &str,
        queued_at: &str,
    ) -> PendingEvent {
        PendingEvent {
            id: crate::domain::pending_event::PendingEventId(format!(
                "1000-{}", kind
            )),
            kind: kind.to_string(),
            loom_id: loom.to_string(),
            knot_id: knot.to_string(),
            strand_path: path.to_string(),
            queued_at: queued_at.to_string(),
        }
    }

    #[test]
    fn build_state_with_queued_events() {
        // Queue has events — strand_queue should be populated
        let store = LoomStore::new();
        let log_port: Arc<dyn LoomLogPort> = Arc::new(MockLoomLogForState::default());
        let profile_repo: Arc<dyn AgentProfileRepository> =
            Arc::new(MockProfileRepoForState::default());
        let state_writer: Arc<dyn StateWriterPort> =
            Arc::new(MockStateWriterForState::default());
        let rig_dir = PathBuf::from("/test/rig");

        let queue: Arc<dyn StrandEventQueue> =
            Arc::new(crate::application::in_memory_event_queue::InMemoryEventQueue::new());

        // Push three events
        queue.push(make_pending(
            "Created",
            "review-loom",
            "review",
            "src/main.rs",
            "2026-06-30T12:00:00Z",
        ));
        queue.push(make_pending(
            "Modified",
            "review-loom",
            "review",
            "src/lib.rs",
            "2026-06-30T12:00:01Z",
        ));
        queue.push(make_pending(
            "Deleted",
            "docs-loom",
            "docs",
            "docs/old.md",
            "2026-06-30T12:00:02Z",
        ));

        let strand_queue: StrandQueueRef =
            Arc::new(std::sync::Mutex::new(Some(queue)));

        let model_registry = Arc::new(
            crate::application::usecases::test_fixtures::MockModelRegistry::default(),
        );
        let uc = WriteState::new(
            store.clone(),
            log_port,
            profile_repo,
            model_registry,
            state_writer,
            rig_dir,
            strand_queue,
        );

        let state = uc.build_state().unwrap();

        // Should have 3 entries
        assert_eq!(state.strand_queue.len(), 3);

        // First entry: Created
        assert_eq!(
            state.strand_queue[0].strand_path, "src/main.rs",
        );
        assert_eq!(state.strand_queue[0].loom_id, "review-loom");
        assert_eq!(state.strand_queue[0].knot_id, "review");
        assert_eq!(state.strand_queue[0].event_kind, "created");
        assert_eq!(
            state.strand_queue[0].queued_at, "2026-06-30T12:00:00Z"
        );

        // Second entry: Modified
        assert_eq!(
            state.strand_queue[1].strand_path, "src/lib.rs",
        );
        assert_eq!(state.strand_queue[1].loom_id, "review-loom");
        assert_eq!(state.strand_queue[1].knot_id, "review");
        assert_eq!(state.strand_queue[1].event_kind, "modified");

        // Third entry: Deleted
        assert_eq!(
            state.strand_queue[2].strand_path, "docs/old.md",
        );
        assert_eq!(state.strand_queue[2].loom_id, "docs-loom");
        assert_eq!(state.strand_queue[2].knot_id, "docs");
        assert_eq!(state.strand_queue[2].event_kind, "deleted");
    }

    // ── Change-driven write tests (plan 083) ────────────────────────

    /// A state writer that both records every write and materialises
    /// the `state.json` file (so the existence check in `execute` sees
    /// the same file a production run would).
    struct CountingStateWriter {
        dir: PathBuf,
        writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl CountingStateWriter {
        fn new(dir: PathBuf) -> Self {
            Self {
                dir,
                writes: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }

        fn count(&self) -> usize {
            self.writes.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn state_path(&self) -> PathBuf {
            self.dir.join("state.json")
        }
    }

    impl StateWriterPort for CountingStateWriter {
        fn write_state(&self, state: &RigState) -> Result<(), PortError> {
            use std::io::Write;
            self.writes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let json = serde_json::to_string_pretty(state)
                .map_err(|e| PortError::StateWriteFailed(e.to_string()))?;
            std::fs::create_dir_all(&self.dir)
                .map_err(|e| PortError::StateWriteFailed(e.to_string()))?;
            let mut file = std::fs::File::create(self.state_path())
                .map_err(|e| PortError::StateWriteFailed(e.to_string()))?;
            file.write_all(json.as_bytes())
                .map_err(|e| PortError::StateWriteFailed(e.to_string()))?;
            Ok(())
        }
    }

    /// Build a `WriteState` over a real directory layout
    /// (`rig_dir = <tmp>/rig` → runtime root `<tmp>/tie-offs/rig`) with
    /// the counting file materialising writer.
    fn build_change_driven_uc(
        rig_dir: PathBuf,
    ) -> (
        WriteState,
        std::sync::Arc<CountingStateWriter>,
        LoomStore,
        Arc<MockProfileRepoForState>,
        PathBuf,
    ) {
        let store = LoomStore::new();
        let log_port: Arc<dyn LoomLogPort> =
            Arc::new(MockLoomLogForState::default());
        let profile_repo = Arc::new(MockProfileRepoForState::default());
        let model_registry = Arc::new(
            crate::application::usecases::test_fixtures::MockModelRegistry::default(),
        );
        let runtime_root = derive_runtime_root(&rig_dir);
        let writer = std::sync::Arc::new(CountingStateWriter::new(
            runtime_root.clone(),
        ));
        let strand_queue: StrandQueueRef =
            Arc::new(std::sync::Mutex::new(None));
        let uc = WriteState::new(
            store.clone(),
            log_port,
            profile_repo.clone(),
            model_registry,
            writer.clone() as std::sync::Arc<dyn StateWriterPort>,
            rig_dir.clone(),
            strand_queue,
        );
        (uc, writer, store, profile_repo, runtime_root)
    }

    #[test]
    fn second_identical_tick_is_skipped_and_change_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        let (mut uc, writer, store, profile_repo, runtime_root) =
            build_change_driven_uc(rig_dir);
        store.register(test_loom("prds"));

        // First write: creates state.json (the run's baseline).
        uc.execute().unwrap();
        assert_eq!(writer.count(), 1, "first write must happen");
        let state_file = runtime_root.join("state.json");
        assert!(state_file.exists());
        let first = std::fs::read_to_string(&state_file).unwrap();

        // Second tick: identical content (only the updated_at tick
        // differs) and the file exists → no write, no churn.
        uc.execute().unwrap();
        assert_eq!(
            writer.count(),
            1,
            "unchanged tick must be skipped"
        );
        assert_eq!(
            std::fs::read_to_string(&state_file).unwrap(),
            first,
            "file content must be untouched on a skipped tick"
        );

        // A real change (new profile) → written.
        profile_repo.add_profile(
            AgentProfile::new(
                "fast".to_string(),
                "openai".to_string(),
                "gpt-4o".to_string(),
                "Fast.".to_string(),
            )
            .unwrap(),
        );
        uc.execute().unwrap();
        assert_eq!(
            writer.count(),
            2,
            "changed content must be written"
        );

        // External deletion of state.json → forced rewrite even though
        // the content is unchanged again.
        std::fs::remove_file(&state_file).unwrap();
        uc.execute().unwrap();
        assert_eq!(
            writer.count(),
            3,
            "a missing state.json must be rewritten"
        );
        assert!(state_file.exists(), "state.json restored");

        // And the tick after that is skipped again.
        uc.execute().unwrap();
        assert_eq!(writer.count(), 3, "post-rewrite tick is skipped");
    }

    #[test]
    fn updated_at_alone_never_forces_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let rig_dir = dir.path().join("rig");
        let (mut uc, writer, _, _, _) = build_change_driven_uc(rig_dir);

        uc.execute().unwrap();
        assert_eq!(writer.count(), 1);
        // Every subsequent tick stamps a new updated_at; none of them
        // may force a write.
        for _ in 0..3 {
            uc.execute().unwrap();
        }
        assert_eq!(
            writer.count(),
            1,
            "only the updated_at tick must not trigger writes"
        );
    }
}
