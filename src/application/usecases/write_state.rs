//! Use case: snapshot the rig's current state to `rig/state.json`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::ports::{
    AgentProfileRepository, LoomLogPort, PortError, StateWriterPort,
};
use crate::application::store::LoomStore;
use crate::domain::entities::{KnotId, LoomId, RigState, RigStateKnot, RigStateLoom, RigStateProfile};
use crate::domain::events::LoomEvent;

use super::types::format_timestamp;

/// Use case: snapshot the rig's current state to `rig/state.json`.
///
/// Reads from `LoomStore` (looms + knots), `AgentProfileRepository`
/// (profiles), and `LoomLogPort` (knot processing status from logs),
/// then serialises everything into a `RigState` and delegates to
/// `StateWriterPort` for atomic write.
///
/// This is the core logic called by the background state writer task.
pub struct WriteState {
    store: LoomStore,
    log_port: Arc<dyn LoomLogPort>,
    profile_repo: Arc<dyn AgentProfileRepository>,
    state_writer: Arc<dyn StateWriterPort>,
    rig_dir: PathBuf,
}

impl WriteState {
    /// Create a new `WriteState` use case.
    pub fn new(
        store: LoomStore,
        log_port: Arc<dyn LoomLogPort>,
        profile_repo: Arc<dyn AgentProfileRepository>,
        state_writer: Arc<dyn StateWriterPort>,
        rig_dir: PathBuf,
    ) -> Self {
        Self {
            store,
            log_port,
            profile_repo,
            state_writer,
            rig_dir,
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

        let rig_state_profiles: Vec<RigStateProfile> = profiles
            .into_iter()
            .map(|p| RigStateProfile {
                name: p.name,
                provider: p.provider,
                model: p.model,
                timeout: p.timeout,
            })
            .collect();

        let rig_path = self.rig_dir.to_string_lossy().to_string();

        Ok(RigState {
            rig_path,
            looms: rig_state_looms,
            profiles: rig_state_profiles,
            updated_at: format_timestamp(),
        })
    }

    /// Execute: build state and write to disk atomically.
    pub fn execute(&self) -> Result<(), PortError> {
        let state = self.build_state()?;
        self.state_writer.write_state(&state)
    }

    /// Derive the processing status for a knot from its loom-log.
    ///
    /// Walks the loom-log events for the given loom and finds the
    /// latest event referencing the given knot, then maps it to a
    /// status string.
    fn derive_knot_state(&self, loom_id: &LoomId, knot_id: &KnotId) -> RigStateKnot {
        let events = match self.log_port.read_all(loom_id) {
            Ok(e) => e,
            Err(_) => return RigStateKnot {
                id: knot_id.0.clone(),
                status: "idle".to_string(),
                last_strand_path: None,
                last_tie_off_path: None,
                last_error: None,
                last_event_at: None,
            },
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
                    strand_dir: PathBuf::from("strands"),
                    git_versioned: true,
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
        let state_writer = Arc::new(MockStateWriterForState::default());
        let rig_dir = PathBuf::from("/test/rig");

        let use_case = WriteState::new(
            store.clone(),
            log_port.clone(),
            profile_repo.clone(),
            state_writer.clone(),
            rig_dir,
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
        assert_eq!(state.profiles[0].provider, "openai");
        assert_eq!(state.profiles[0].model, "gpt-4o");
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
        let (uc, store, _, profile_repo, writer) = build_use_case();

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

        let uc = WriteState::new(
            store.clone(),
            log_port,
            profile_repo,
            state_writer,
            rig_dir,
        );

        // Even with no log events, the state should build (knots default to idle)
        store.register(test_loom("prds"));
        let result = uc.execute();
        assert!(result.is_ok());
    }

    #[test]
    fn multiple_looms_in_state() {
        let (uc, store, _, _, writer) = build_use_case();

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
        let (uc, store, log_port, profile_repo, writer) = build_use_case();

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
        assert!(value.get("updated_at").is_some());

        // Loom structure
        let looms = value["looms"].as_array().unwrap();
        assert_eq!(looms[0]["id"], "my-loom");
        let knots = looms[0]["knots"].as_array().unwrap();
        assert_eq!(knots[0]["id"], "k1");
        assert_eq!(knots[0]["status"], "idle");

        // Profile structure
        let profiles = value["profiles"].as_array().unwrap();
        assert_eq!(profiles[0]["name"], "fast");
        assert_eq!(profiles[0]["provider"], "openai");
        assert_eq!(profiles[0]["model"], "gpt-4o");
    }
}
