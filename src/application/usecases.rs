//! Application-layer use cases.
//!
//! Each use case orchestrates domain entities through port traits and the
//! in-memory loom store. Tests use mock port implementations — no IO.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

use crate::application::ports::{
    AgentRunner, ExecutionContext, KnotEventType, KnotState,
    LoomLogPort, LoomRepository, ProcessingStatus, PortError,
    TieOffSink,
};
use crate::application::store::LoomStore;
use crate::domain::entities::{KnotId, Loom, LoomId, StrandPath, TieOff, TieOffPath};
use crate::domain::events::{LoomEvent, StrandEvent};
use crate::domain::value_objects::RigAgentConfig;
use std::path::PathBuf;

// ── Query Result Types ───────────────────────────────────────────────────

/// A summary of a loom (lightweight, for list responses).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoomSummary {
    /// The loom's unique ID.
    pub id: LoomId,
    /// The source directory path.
    #[schema(value_type = String)]
    pub source_dir: PathBuf,
    /// The tie-off (output) directory path.
    #[schema(value_type = String)]
    pub tie_off_dir: PathBuf,
    /// Number of knots in this loom.
    pub knot_count: usize,
}

/// Result of the `GetKnotStatus` use case.
///
/// Derived from the latest loom-log entries for a knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct KnotStatus {
    /// The knot whose status was retrieved.
    pub knot_id: KnotId,
    /// The loom this knot belongs to.
    pub loom_id: LoomId,
    /// The current processing status derived from loom-log events.
    pub status: ProcessingStatus,
    /// Path to the last strand processed (if any).
    pub last_strand_path: Option<StrandPath>,
    /// Path to the last tie-off produced (if any).
    pub last_tie_off_path: Option<TieOffPath>,
    /// Error message from the last failed processing (if any).
    pub last_error: Option<String>,
}

// ── DiscoverLooms ──────────────────────────────────────────────────────────

/// Use case: discover looms in a workspace and register them.
///
/// Calls `LoomRepository::scan()` to find looms, then for each loom:
/// - Appends `KnotRegistered` to the loom log via `LoomLogPort::append()`
/// - Registers the loom in `LoomStore`
pub struct DiscoverLooms {
    repository: Arc<dyn LoomRepository>,
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
}

impl DiscoverLooms {
    /// Create a new `DiscoverLooms` use case.
    pub fn new(
        repository: Arc<dyn LoomRepository>,
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
    ) -> Self {
        Self {
            repository,
            log_port,
            store,
        }
    }

    /// Execute discovery against the given workspace path.
    ///
    /// Returns the list of discovered looms.
    pub fn execute(&self, workspace: &Path) -> Result<Vec<Loom>, PortError> {
        let looms = self.repository.scan(workspace)?;

        for loom in &looms {
            self.register_knots(loom)?;
            self.store.register(loom.clone());
        }

        Ok(looms)
    }

    /// Append `KnotRegistered` event for every knot in a loom.
    fn register_knots(&self, loom: &Loom) -> Result<(), PortError> {
        for knot in &loom.knots {
            self.log_port.append(LoomEvent::KnotRegistered {
                loom_id: loom.id.clone(),
                knot_id: knot.id.clone(),
            })?;
        }
        Ok(())
    }
}

// ── RegisterLoom ───────────────────────────────────────────────────────────

/// Use case: register a single loom.
///
/// 1. Opens the loom activity log via `LoomLogPort::open()`
/// 2. Appends `KnotRegistered` for each knot via `LoomLogPort::append()`
/// 3. Appends `LoomStarted` event via `LoomLogPort::append()`
/// 4. Stores the loom in `LoomStore`
///
/// Returns an error if a loom with the same ID already exists.
pub struct RegisterLoom {
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
}

impl RegisterLoom {
    /// Create a new `RegisterLoom` use case.
    pub fn new(
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
    ) -> Self {
        Self {
            log_port,
            store,
        }
    }

    /// Register the given loom.
    ///
    /// Returns `PortError::LoomSaveFailed` if the loom ID already exists.
    pub fn execute(&self, loom: Loom) -> Result<(), PortError> {
        // Check for duplicate ID before any side effects
        if self.store.get(&loom.id).is_some() {
            return Err(PortError::LoomSaveFailed(format!(
                "loom '{}' already registered",
                loom.id.0
            )));
        }

        // Open the loom activity log
        self.log_port.open(&loom.id)?;

        // Append KnotRegistered for each knot
        for knot in &loom.knots {
            self.log_port.append(LoomEvent::KnotRegistered {
                loom_id: loom.id.clone(),
                knot_id: knot.id.clone(),
            })?;
        }

        // Append LoomStarted event
        self.log_port.append(LoomEvent::LoomStarted {
            loom_id: loom.id.clone(),
        })?;

        // Store the loom
        self.store.register(loom);

        Ok(())
    }
}

// ── UnregisterLoom ─────────────────────────────────────────────────────────

/// Use case: unregister a loom.
///
/// 1. Appends `LoomStopped` event via `LoomLogPort::append()`
/// 2. Removes the loom from `LoomStore`
///
/// Returns an error if the loom was not found.
pub struct UnregisterLoom {
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
}

impl UnregisterLoom {
    /// Create a new `UnregisterLoom` use case.
    pub fn new(log_port: Arc<dyn LoomLogPort>, store: LoomStore) -> Self {
        Self { log_port, store }
    }

    /// Unregister the loom with the given ID.
    ///
    /// Returns `PortError::LoomNotFound` if the loom is not in the store.
    pub fn execute(&self, id: &LoomId) -> Result<(), PortError> {
        // Check loom exists before any side effects
        if self.store.get(id).is_none() {
            return Err(PortError::LoomNotFound(id.clone()));
        }

        // Append LoomStopped event
        self.log_port.append(LoomEvent::LoomStopped {
            loom_id: id.clone(),
        })?;

        // Remove from store
        self.store.unregister(id);

        Ok(())
    }
}

// ── ListLooms ──────────────────────────────────────────────────────────────

/// Use case: list all registered looms as summaries.
///
/// Reads from `LoomStore::list()` and maps each loom to a lightweight
/// `LoomSummary`.
pub struct ListLooms {
    store: LoomStore,
}

impl ListLooms {
    /// Create a new `ListLooms` use case.
    pub fn new(store: LoomStore) -> Self {
        Self { store }
    }

    /// Return summaries of all registered looms.
    pub fn execute(&self) -> Vec<LoomSummary> {
        self.store.list()
            .into_iter()
            .map(|loom| LoomSummary {
                id: loom.id,
                source_dir: loom.source_dir,
                tie_off_dir: loom.tie_off_dir,
                knot_count: loom.knots.len(),
            })
            .collect()
    }
}

// ── GetLoom ────────────────────────────────────────────────────────────────

/// Use case: retrieve a full loom by ID.
///
/// Reads from `LoomStore::get()`. Returns `PortError::LoomNotFound` if
/// the loom does not exist.
pub struct GetLoom {
    store: LoomStore,
}

impl GetLoom {
    /// Create a new `GetLoom` use case.
    pub fn new(store: LoomStore) -> Self {
        Self { store }
    }

    /// Return the full loom with the given ID.
    pub fn execute(&self, id: &LoomId) -> Result<Loom, PortError> {
        self.store.get(id)
            .ok_or_else(|| PortError::LoomNotFound(id.clone()))
    }
}

// ── GetLoomActivity ────────────────────────────────────────────────────────

/// Use case: read the activity log for a loom.
///
/// Calls `LoomLogPort::read_all()` and returns all recorded events.
pub struct GetLoomActivity {
    log_port: Arc<dyn LoomLogPort>,
}

impl GetLoomActivity {
    /// Create a new `GetLoomActivity` use case.
    pub fn new(log_port: Arc<dyn LoomLogPort>) -> Self {
        Self { log_port }
    }

    /// Return all log entries for the given loom.
    pub fn execute(&self, loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
        self.log_port.read_all(loom_id)
    }
}

// ── GetKnotStatus ──────────────────────────────────────────────────────────

/// Use case: get the current processing state of a knot.
///
/// Reads the loom-log via `LoomLogPort::read_all()` and derives the
/// current status from the latest knot-related event for the given
/// `knot_id` in the given `loom_id`.
///
/// Returns `PortError::KnotStatusDeriveFailed` if the loom is not found
/// or no events exist for the knot.
pub struct GetKnotStatus {
    store: LoomStore,
    log_port: Arc<dyn LoomLogPort>,
}

impl GetKnotStatus {
    /// Create a new `GetKnotStatus` use case.
    pub fn new(store: LoomStore, log_port: Arc<dyn LoomLogPort>) -> Self {
        Self { store, log_port }
    }

    /// Derive the current status for the given knot from loom-log events.
    pub fn execute(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) -> Result<KnotStatus, PortError> {
        // Verify the loom exists
        if self.store.get(loom_id).is_none() {
            return Err(PortError::KnotStatusDeriveFailed(format!(
                "loom '{}' not found",
                loom_id.0
            )));
        }

        // Read all events from the loom log
        let events = self.log_port
            .read_all(loom_id)
            .map_err(|_| {
                PortError::KnotStatusDeriveFailed(format!(
                    "failed to read loom-log for loom '{}'",
                    loom_id.0
                ))
            })?;

        // Find the latest knot-specific event
        let latest = Self::find_latest_knot_event(&events, knot_id);

        match latest {
            Some(event) => Ok(Self::derive_status(
                loom_id,
                knot_id,
                event,
            )),
            None => Err(PortError::KnotStatusDeriveFailed(format!(
                "no events found for knot '{}' in loom '{}'",
                knot_id.0,
                loom_id.0
            ))),
        }
    }

    /// Find the latest loom event that references the given knot.
    fn find_latest_knot_event<'a>(
        events: &'a [LoomEvent],
        knot_id: &KnotId,
    ) -> Option<&'a LoomEvent> {
        events.iter().rev().find(|event| match event {
            LoomEvent::KnotRegistered { knot_id: kid, .. }
            | LoomEvent::KnotProcessing { knot_id: kid, .. }
            | LoomEvent::KnotCompleted { knot_id: kid, .. }
            | LoomEvent::KnotFailed { knot_id: kid, .. } => kid == knot_id,
            _ => false,
        })
    }

    /// Derive a `KnotStatus` from a single loom event.
    fn derive_status(
        loom_id: &LoomId,
        knot_id: &KnotId,
        event: &LoomEvent,
    ) -> KnotStatus {
        match event {
            LoomEvent::KnotRegistered { .. } => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Idle,
                last_strand_path: None,
                last_tie_off_path: None,
                last_error: None,
            },
            LoomEvent::KnotProcessing {
                strand_path, ..
            } => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Processing,
                last_strand_path: Some(strand_path.clone()),
                last_tie_off_path: None,
                last_error: None,
            },
            LoomEvent::KnotCompleted {
                strand_path,
                tie_off_path,
                ..
            } => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Completed,
                last_strand_path: Some(strand_path.clone()),
                last_tie_off_path: Some(tie_off_path.clone()),
                last_error: None,
            },
            LoomEvent::KnotFailed { strand_path, error, .. } => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Failed,
                last_strand_path: Some(strand_path.clone()),
                last_tie_off_path: None,
                last_error: Some(error.clone()),
            },
            // Fallback for non-knot-specific events
            _ => KnotStatus {
                knot_id: knot_id.clone(),
                loom_id: loom_id.clone(),
                status: ProcessingStatus::Idle,
                last_strand_path: None,
                last_tie_off_path: None,
                last_error: None,
            },
        }
    }
}

// ── ProcessStrand ─────────────────────────────────────────────────────────

/// Use case: process a single strand event through the agent pipeline.
///
/// 1. Receive `StrandEvent` (Created / Modified / Deleted)
/// 2. Append `KnotProcessing` to loom-log
/// 3. Build execution context from `RigAgentConfig` + `Knot`
/// 4. Call `AgentRunner::execute()` (skipped for Deleted events)
/// 5. Call `TieOffSink::write()` with result
/// 6. Append `KnotCompleted` or `KnotFailed` to loom-log
/// 7. Append `StrandProcessed` to loom-log
pub struct ProcessStrand {
    store: LoomStore,
    log_port: Arc<dyn LoomLogPort>,
    agent_runner: Arc<dyn AgentRunner>,
    tie_off_sink: Arc<dyn TieOffSink>,
    rig_config: RigAgentConfig,
}

impl ProcessStrand {
    /// Create a new `ProcessStrand` use case.
    pub fn new(
        store: LoomStore,
        log_port: Arc<dyn LoomLogPort>,
        agent_runner: Arc<dyn AgentRunner>,
        tie_off_sink: Arc<dyn TieOffSink>,
        rig_config: RigAgentConfig,
    ) -> Self {
        Self {
            store,
            log_port,
            agent_runner,
            tie_off_sink,
            rig_config,
        }
    }

    /// Execute the strand processing pipeline.
    ///
    /// Appends lifecycle events to loom-log: KnotProcessing, then
    /// KnotCompleted or KnotFailed, then StrandProcessed.
    pub fn execute(&self, event: StrandEvent) -> Result<(), PortError> {
        let (loom_id, knot_id, strand_path) = Self::extract_event_fields(&event);
        let is_deleted = matches!(event, StrandEvent::Deleted { .. });

        // Look up the loom and knot
        let loom = self
            .store
            .get(&loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;
        let knot = loom
            .knots
            .iter()
            .find(|k| k.id == knot_id)
            .ok_or_else(|| PortError::KnotStatusDeriveFailed(format!(
                "knot '{}' not found in loom '{}'",
                knot_id.0, loom_id.0
            )))?;

        let _event_type = match event {
            StrandEvent::Created { .. } => KnotEventType::Created,
            StrandEvent::Modified { .. } => KnotEventType::Modified,
            StrandEvent::Deleted { .. } => KnotEventType::Deleted,
        };

        // Determine tie-off path
        let tie_off_path = Self::compute_tie_off_path(&loom, &strand_path);

        // 1. Append KnotProcessing to loom-log
        self.log_port.append(LoomEvent::KnotProcessing {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
        })?;

        if is_deleted {
            // For deleted events, write a report tie-off without running agent
            let tie_off = TieOff {
                content: format!(
                    "Strand deleted: {}\nPrevious output at: {}",
                    strand_path.0.display(),
                    tie_off_path.0.display()
                ),
                path: tie_off_path.clone(),
                status: crate::domain::entities::TieOffStatus::Produced,
            };
            self.tie_off_sink.write(tie_off)?;

            // Append KnotCompleted to loom-log
            self.log_port.append(LoomEvent::KnotCompleted {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                strand_path: strand_path.clone(),
                tie_off_path: tie_off_path.clone(),
            })?;

            // Append StrandProcessed
            self.log_port.append(LoomEvent::StrandProcessed {
                loom_id,
                strand_path,
                error: None,
            })?;

            return Ok(());
        }

        // 2. Build CLI args from knot's agent config + prompt template
        let mut cli_args =
            knot.agent_config.build_cli_args(&knot.prompt_template);
        // Append strand content reference using pi's @file syntax
        cli_args.push(
            format!("@{}", strand_path.0.display()),
        );

        // 3. Build execution context
        let ctx = ExecutionContext {
            cli_path: self.rig_config.cli_path.clone(),
            cli_args,
            prompt: knot.prompt_template.instructions.clone(),
            strand_path: strand_path.clone(),
        };

        // 4. Execute agent and handle result
        let result = self.agent_runner.execute(ctx);

        match result {
            Ok(output) => {
                // 4. Write successful tie-off
                let tie_off = TieOff {
                    content: output.stdout,
                    path: tie_off_path.clone(),
                    status: crate::domain::entities::TieOffStatus::Produced,
                };
                self.tie_off_sink.write(tie_off)?;

                // 5. Append KnotCompleted to loom-log
                self.log_port.append(LoomEvent::KnotCompleted {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    strand_path: strand_path.clone(),
                    tie_off_path: tie_off_path.clone(),
                })?;

                // 6. Append StrandProcessed
                self.log_port.append(LoomEvent::StrandProcessed {
                    loom_id,
                    strand_path,
                    error: None,
                })?;

                Ok(())
            }
            Err(err) => {
                let error_msg = err.to_string();

                // 4. Write error tie-off
                let tie_off = TieOff {
                    content: format!("Processing failed: {}", error_msg),
                    path: tie_off_path.clone(),
                    status: crate::domain::entities::TieOffStatus::Failed,
                };
                self.tie_off_sink.write(tie_off)?;

                // 5. Append KnotFailed to loom-log
                self.log_port.append(LoomEvent::KnotFailed {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    strand_path: strand_path.clone(),
                    error: error_msg.clone(),
                })?;

                // 6. Append StrandProcessed with error details
                self.log_port.append(LoomEvent::StrandProcessed {
                    loom_id,
                    strand_path,
                    error: Some(error_msg),
                })?;

                Ok(())
            }
        }
    }

    /// Extract common fields from any `StrandEvent` variant.
    fn extract_event_fields(
        event: &StrandEvent,
    ) -> (LoomId, KnotId, StrandPath) {
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
            } => (loom_id.clone(), knot_id.clone(), strand_path.clone()),
        }
    }

    /// Compute the tie-off output path from loom + strand path.
    fn compute_tie_off_path(loom: &Loom, strand_path: &StrandPath) -> TieOffPath {
        let filename = strand_path
            .0
            .file_name()
            .map(|f| format!("{}.output", f.to_string_lossy()))
            .unwrap_or_else(|| "output".to_string());
        TieOffPath(loom.tie_off_dir.join(filename))
    }

}

// ── Tests ──────────────────────────────────────────────────────────────────

// #[cfg(test)]
#[cfg(feature = "__disabled_tests")]
mod tests {
    use super::*;
    use crate::application::ports::AgentOutput;
    use crate::domain::entities::{Knot, KnotId, LoomId};
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
    use std::collections::HashSet;
    use std::path::PathBuf;

    // ── Mock Implementations ────────────────────────────────────────────

    /// Mock `LoomRepository` with configurable scan results.
    struct MockLoomRepository {
        scan_result: Result<Vec<Loom>, PortError>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(&self, _rig: &Path) -> Result<Vec<Loom>, PortError> {
            self.scan_result.clone()
        }

        fn get(&self, _id: &LoomId) -> Result<Option<Loom>, PortError> {
            Ok(None)
        }

        fn list(&self) -> Result<Vec<Loom>, PortError> {
            Ok(vec![])
        }

        fn save(&self, _loom: Loom) -> Result<(), PortError> {
            Ok(())
        }
    }

    /// Mock `KnotStatePort` that never errors.
    #[derive(Default)]
    struct MockKnotStatePort;

    impl KnotStatePort for MockKnotStatePort {
        fn create(&self, _knot_id: &KnotId) -> Result<(), PortError> {
            Ok(())
        }

        fn update(&self, _state: crate::application::ports::KnotState) -> Result<(), PortError> {
            Ok(())
        }

        fn get(
            &self,
            _knot_id: &KnotId,
        ) -> Result<Option<crate::application::ports::KnotState>, PortError> {
            Ok(None)
        }
    }

    /// Mock `LoomLogPort` that never errors.
    #[derive(Default)]
    struct MockLoomLogPort;

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(&self, _loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
            Ok(vec![])
        }
    }

    use std::sync::{Arc, RwLock};

    /// Tracking mock `LoomLogPort` that records all method calls.
    /// Uses `Arc<RwLock<...>>` so data survives after boxing.
    struct TrackingLoomLogPort {
        open_calls: Arc<RwLock<Vec<LoomId>>>,
        append_calls: Arc<RwLock<Vec<LoomEvent>>>,
    }

    impl TrackingLoomLogPort {
        fn new() -> (
            Self,
            Arc<RwLock<Vec<LoomId>>>,
            Arc<RwLock<Vec<LoomEvent>>>,
        ) {
            let open_calls = Arc::new(RwLock::new(vec![]));
            let append_calls = Arc::new(RwLock::new(vec![]));
            let port = Self {
                open_calls: open_calls.clone(),
                append_calls: append_calls.clone(),
            };
            (port, open_calls, append_calls)
        }
    }

    impl LoomLogPort for TrackingLoomLogPort {
        fn open(&self, loom_id: &LoomId) -> Result<(), PortError> {
            self.open_calls
                .write()
                .unwrap()
                .push(loom_id.clone());
            Ok(())
        }

        fn append(&self, event: LoomEvent) -> Result<(), PortError> {
            self.append_calls.write().unwrap().push(event);
            Ok(())
        }

        fn read_all(&self, _loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
            Ok(vec![])
        }
    }

    /// Tracking mock `KnotStatePort` that records all `create` calls.
    /// Uses `Arc<RwLock<...>>` so data survives after boxing.
    struct TrackingKnotStatePort {
        create_calls: Arc<RwLock<Vec<KnotId>>>,
    }

    impl TrackingKnotStatePort {
        fn new() -> (Self, Arc<RwLock<Vec<KnotId>>>) {
            let create_calls = Arc::new(RwLock::new(vec![]));
            let port = Self {
                create_calls: create_calls.clone(),
            };
            (port, create_calls)
        }
    }

    impl KnotStatePort for TrackingKnotStatePort {
        fn create(&self, knot_id: &KnotId) -> Result<(), PortError> {
            self.create_calls
                .write()
                .unwrap()
                .push(knot_id.clone());
            Ok(())
        }

        fn update(
            &self,
            _state: crate::application::ports::KnotState,
        ) -> Result<(), PortError> {
            Ok(())
        }

        fn get(
            &self,
            _knot_id: &KnotId,
        ) -> Result<Option<crate::application::ports::KnotState>, PortError> {
            Ok(None)
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Build a loom with the given ID and optional knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            source_dir: PathBuf::from("src"),
            tie_off_dir: PathBuf::from("out"),
            knots,
        }
    }

    /// Build a knot with the given ID.
    fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_config: AgentConfig::new(
                "review".to_string(),
                "openai".to_string(),
                "gpt-4o".to_string(),
            )
            .unwrap(),
            prompt_template: PromptTemplate::new(
                "full-file".to_string(),
                "check it".to_string(),
            )
            .unwrap(),
            source_dir: None,
            tie_off_dir: None,
        }
    }

    // ── DiscoverLooms Tests ─────────────────────────────────────────────

    #[test]
    fn discover_looms_success() {
        let loom1 = build_loom("looms/a", vec![build_knot("k1")]);
        let loom2 = build_loom("looms/b", vec![build_knot("k2"), build_knot("k3")]);
        let discovered = vec![loom1.clone(), loom2.clone()];

        let repo = Arc::new(MockLoomRepository {
            scan_result: Ok(discovered),
        });
        let state_port = Arc::new(MockKnotStatePort::default());
        let log_port = Arc::new(MockLoomLogPort::default());
        let store = LoomStore::new();

        let use_case =
            DiscoverLooms::new(repo, state_port, log_port, store.clone());

        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_ok());
        let looms = result.unwrap();
        assert_eq!(looms.len(), 2);

        // Store should contain both looms
        let stored = store.list();
        assert_eq!(stored.len(), 2);
        let stored_ids: HashSet<_> = stored.iter().map(|l| l.id.0.as_str()).collect();
        assert!(stored_ids.contains("looms/a"));
        assert!(stored_ids.contains("looms/b"));
    }

    #[test]
    fn discover_looms_empty_workspace() {
        let repo = Arc::new(MockLoomRepository {
            scan_result: Ok(vec![]),
        });
        let state_port = Arc::new(MockKnotStatePort::default());
        let log_port = Arc::new(MockLoomLogPort::default());
        let store = LoomStore::new();

        let use_case =
            DiscoverLooms::new(repo, state_port, log_port, store.clone());

        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
        assert!(store.list().is_empty());
    }

    #[test]
    fn discover_looms_repository_error() {
        let repo = Arc::new(MockLoomRepository {
            scan_result: Err(PortError::RigScanFailed(
                "permission denied".to_string(),
            )),
        });
        let state_port = Arc::new(MockKnotStatePort::default());
        let log_port = Arc::new(MockLoomLogPort::default());
        let store = LoomStore::new();

        let use_case =
            DiscoverLooms::new(repo, state_port, log_port, store.clone());

        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            PortError::RigScanFailed("permission denied".to_string())
        );
        // Store should be untouched
        assert!(store.list().is_empty());
    }

    // ── RegisterLoom Tests ──────────────────────────────────────────────

    #[test]
    fn register_loom_creates_state_files() {
        let loom = build_loom("my-loom", vec![build_knot("k1"), build_knot("k2")]);
        let loom_id = loom.id.clone();

        let (log_port, log_open, log_append) = TrackingLoomLogPort::new();
        let (state_port, st_create) = TrackingKnotStatePort::new();
        let store = LoomStore::new();

        let use_case =
            RegisterLoom::new(Arc::new(log_port), Arc::new(state_port), store.clone());
        let result = use_case.execute(loom);

        // Should succeed
        assert!(result.is_ok());

        // LoomLogPort::open() was called once
        let open_calls = log_open.read().unwrap();
        assert_eq!(open_calls.len(), 1);
        assert_eq!(open_calls[0], loom_id);

        // KnotStatePort::create() was called for each knot
        let create_calls = st_create.read().unwrap();
        assert_eq!(create_calls.len(), 2);
        assert!(create_calls.contains(&KnotId("k1".to_string())));
        assert!(create_calls.contains(&KnotId("k2".to_string())));

        // LoomLogPort::append(LoomStarted) was called
        let append_calls = log_append.read().unwrap();
        assert_eq!(append_calls.len(), 1);
        match &append_calls[0] {
            LoomEvent::LoomStarted { loom_id: id } => {
                assert_eq!(*id, loom_id);
            }
            _ => panic!("Expected LoomStarted event"),
        }

        // Loom is in the store
        assert!(store.get(&loom_id).is_some());
    }

    #[test]
    fn register_loom_duplicate_id_error() {
        let loom1 = build_loom("existing", vec![build_knot("k1")]);
        let loom2 = build_loom("existing", vec![build_knot("k2")]);

        let (log_port, _log_open, _log_append) = TrackingLoomLogPort::new();
        let (state_port, _st_create) = TrackingKnotStatePort::new();
        let store = LoomStore::new();

        // Register first loom
        let use_case = RegisterLoom::new(
            Arc::new(log_port),
            Arc::new(state_port),
            store.clone(),
        );
        assert!(use_case.execute(loom1).is_ok());

        // Attempt to register duplicate — must fail without side effects
        let (log_port2, _, _) = TrackingLoomLogPort::new();
        let (state_port2, _) = TrackingKnotStatePort::new();
        let use_case = RegisterLoom::new(
            Arc::new(log_port2),
            Arc::new(state_port2),
            store.clone(),
        );
        let result = use_case.execute(loom2);

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomSaveFailed(msg) => {
                assert!(msg.contains("existing"));
                assert!(msg.contains("already registered"));
            }
            other => panic!("Expected LoomSaveFailed, got {other:?}"),
        }

        // Store should still have only the original loom
        let stored = store.get(&LoomId("existing".to_string()));
        assert!(stored.is_some());
        // Should have the original knots (k1), not the duplicate (k2)
        let stored = stored.unwrap();
        assert_eq!(stored.knots.len(), 1);
        assert_eq!(stored.knots[0].id, KnotId("k1".to_string()));
    }

    // ── UnregisterLoom Tests ────────────────────────────────────────────

    #[test]
    fn unregister_loom_logs_stopped_event() {
        let loom = build_loom("to-remove", vec![build_knot("k1")]);
        let loom_id = loom.id.clone();

        let (log_port, _log_open, _log_append) = TrackingLoomLogPort::new();
        let (state_port, _st_create) = TrackingKnotStatePort::new();
        let store = LoomStore::new();

        // Register loom first
        let reg = RegisterLoom::new(
            Arc::new(log_port),
            Arc::new(state_port),
            store.clone(),
        );
        assert!(reg.execute(loom).is_ok());

        // Unregister with a fresh tracking log port
        let (unreg_log_port, _, unreg_append) = TrackingLoomLogPort::new();
        let use_case =
            UnregisterLoom::new(Arc::new(unreg_log_port), store.clone());
        let result = use_case.execute(&loom_id);

        // Should succeed
        assert!(result.is_ok());

        // LoomStopped event was appended
        let append_calls = unreg_append.read().unwrap();
        assert_eq!(append_calls.len(), 1);
        match &append_calls[0] {
            LoomEvent::LoomStopped { loom_id: id } => {
                assert_eq!(*id, loom_id);
            }
            _ => panic!("Expected LoomStopped event"),
        }

        // Loom is no longer in the store
        assert!(store.get(&loom_id).is_none());
    }

    // ── ListLooms Tests ─────────────────────────────────────────────────

    #[test]
    fn list_looms_returns_summaries() {
        let store = LoomStore::new();
        store.register(build_loom("loom-a", vec![build_knot("k1")]));
        store.register(build_loom(
            "loom-b",
            vec![build_knot("k2"), build_knot("k3")],
        ));

        let use_case = ListLooms::new(store);
        let summaries = use_case.execute();

        assert_eq!(summaries.len(), 2);

        // Find each summary by id
        let summary_a = summaries
            .iter()
            .find(|s| s.id == LoomId("loom-a".to_string()))
            .expect("loom-a summary missing");
        assert_eq!(summary_a.knot_count, 1);
        assert_eq!(summary_a.source_dir, PathBuf::from("src"));
        assert_eq!(summary_a.tie_off_dir, PathBuf::from("out"));

        let summary_b = summaries
            .iter()
            .find(|s| s.id == LoomId("loom-b".to_string()))
            .expect("loom-b summary missing");
        assert_eq!(summary_b.knot_count, 2);
    }

    // ── GetLoom Tests ───────────────────────────────────────────────────

    #[test]
    fn get_loom_by_id() {
        let store = LoomStore::new();
        let loom = build_loom("my-loom", vec![build_knot("k1")]);
        let loom_id = loom.id.clone();
        store.register(loom.clone());

        let use_case = GetLoom::new(store);
        let result = use_case.execute(&loom_id);

        assert!(result.is_ok());
        let found = result.unwrap();
        assert_eq!(found.id, loom_id);
        assert_eq!(found.knots.len(), 1);
    }

    #[test]
    fn get_loom_missing_returns_error() {
        let store = LoomStore::new();
        let use_case = GetLoom::new(store);
        let result = use_case.execute(&LoomId("unknown".to_string()));

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("unknown".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }
    }

    // ── GetLoomActivity Tests ───────────────────────────────────────────

    /// Mock `LoomLogPort` that returns configurable events from `read_all`.
    struct MockLoomLogPortWithEvents {
        events: Vec<LoomEvent>,
    }

    impl LoomLogPort for MockLoomLogPortWithEvents {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(&self, _loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
            Ok(self.events.clone())
        }
    }

    #[test]
    fn get_loom_activity_from_log() {
        let events = vec![
            LoomEvent::LoomStarted {
                loom_id: LoomId("my-loom".to_string()),
            },
            LoomEvent::KnotRegistered {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
            },
        ];

        let log_port = Arc::new(MockLoomLogPortWithEvents { events });
        let use_case = GetLoomActivity::new(log_port);
        let result = use_case.execute(&LoomId("my-loom".to_string()));

        assert!(result.is_ok());
        let got = result.unwrap();
        assert_eq!(got.len(), 2);
        match &got[0] {
            LoomEvent::LoomStarted { loom_id } => {
                assert_eq!(*loom_id, LoomId("my-loom".to_string()));
            }
            _ => panic!("Expected LoomStarted"),
        }
        match &got[1] {
            LoomEvent::KnotRegistered { loom_id, knot_id } => {
                assert_eq!(*loom_id, LoomId("my-loom".to_string()));
                assert_eq!(*knot_id, KnotId("k1".to_string()));
            }
            _ => panic!("Expected KnotRegistered"),
        }
    }

    #[test]
    fn get_loom_activity_empty_log() {
        let log_port = Arc::new(MockLoomLogPortWithEvents { events: vec![] });
        let use_case = GetLoomActivity::new(log_port);
        let result = use_case.execute(&LoomId("empty".to_string()));

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    // ── GetKnotStatus Tests ─────────────────────────────────────────────

    /// Mock `KnotStatePort` that returns configurable state from `get`.
    struct MockKnotStatePortWithState {
        state: Option<KnotState>,
    }

    impl KnotStatePort for MockKnotStatePortWithState {
        fn create(&self, _knot_id: &KnotId) -> Result<(), PortError> {
            Ok(())
        }

        fn update(&self, _state: KnotState) -> Result<(), PortError> {
            Ok(())
        }

        fn get(&self, _knot_id: &KnotId) -> Result<Option<KnotState>, PortError> {
            Ok(self.state.clone())
        }
    }

    #[test]
    fn get_knot_status_from_state() {
        let state = KnotState {
            knot_id: KnotId("k1".to_string()),
            event_type: crate::application::ports::KnotEventType::Modified,
            strand_path: crate::domain::entities::StrandPath(PathBuf::from(
                "src/input.md",
            )),
            tie_off_path: Some(crate::domain::entities::TieOffPath(PathBuf::from(
                "out/output.md",
            ))),
            status: crate::application::ports::ProcessingStatus::Completed,
            error: None,
            last_updated: "2026-06-03T12:00:00Z".to_string(),
        };

        let state_port = Arc::new(MockKnotStatePortWithState {
            state: Some(state.clone()),
        });
        let use_case = GetKnotStatus::new(state_port);
        let knot_id = KnotId("k1".to_string());
        let result = use_case.execute(&knot_id);

        assert!(result.is_ok());
        let status = result.unwrap();
        assert_eq!(status.knot_id, knot_id);
        assert_eq!(
            status.state.status,
            crate::application::ports::ProcessingStatus::Completed,
        );
        assert_eq!(status.state.error, None);
    }

    #[test]
    fn get_knot_status_missing_returns_error() {
        let state_port = Arc::new(MockKnotStatePortWithState { state: None });
        let use_case = GetKnotStatus::new(state_port);
        let result = use_case.execute(&KnotId("k1".to_string()));

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::KnotStateGetFailed(msg) => {
                assert!(msg.contains("k1"));
            }
            other => panic!("Expected KnotStateGetFailed, got {other:?}"),
        }
    }

    // ── Tracking Mocks for ProcessStrand ────────────────────────────────

    /// Tracking mock `KnotStatePort` that records `update` calls.
    struct TrackingKnotStatePortForUpdates {
        update_calls: Arc<RwLock<Vec<KnotState>>>,
    }

    impl TrackingKnotStatePortForUpdates {
        fn new() -> (Self, Arc<RwLock<Vec<KnotState>>>) {
            let update_calls = Arc::new(RwLock::new(vec![]));
            let port = Self {
                update_calls: update_calls.clone(),
            };
            (port, update_calls)
        }
    }

    impl KnotStatePort for TrackingKnotStatePortForUpdates {
        fn create(&self, _knot_id: &KnotId) -> Result<(), PortError> {
            Ok(())
        }

        fn update(&self, state: KnotState) -> Result<(), PortError> {
            self.update_calls.write().unwrap().push(state);
            Ok(())
        }

        fn get(
            &self,
            _knot_id: &KnotId,
        ) -> Result<Option<KnotState>, PortError> {
            Ok(None)
        }
    }

    /// Tracking mock `TieOffSink` that records all `write` calls.
    struct TrackingTieOffSink {
        write_calls: Arc<RwLock<Vec<TieOff>>>,
    }

    impl TrackingTieOffSink {
        fn new() -> (Self, Arc<RwLock<Vec<TieOff>>>) {
            let write_calls = Arc::new(RwLock::new(vec![]));
            let sink = Self {
                write_calls: write_calls.clone(),
            };
            (sink, write_calls)
        }
    }

    impl TieOffSink for TrackingTieOffSink {
        fn write(&self, tie_off: TieOff) -> Result<(), PortError> {
            self.write_calls.write().unwrap().push(tie_off);
            Ok(())
        }
    }

    /// Configurable mock `AgentRunner` that can return success or error.
    struct ConfigurableAgentRunner {
        result: Result<AgentOutput, PortError>,
    }

    impl AgentRunner for ConfigurableAgentRunner {
        fn execute(&self, _ctx: ExecutionContext) -> Result<AgentOutput, PortError> {
            self.result.clone()
        }
    }

    /// Build a Created StrandEvent for testing.
    fn build_created_event(loom_id: &str, knot_id: &str, path: &str) -> StrandEvent {
        StrandEvent::Created {
            loom_id: LoomId(loom_id.to_string()),
            knot_id: KnotId(knot_id.to_string()),
            strand_path: StrandPath(PathBuf::from(path)),
        }
    }

    /// Build a Deleted StrandEvent for testing.
    fn build_deleted_event(loom_id: &str, knot_id: &str, path: &str) -> StrandEvent {
        StrandEvent::Deleted {
            loom_id: LoomId(loom_id.to_string()),
            knot_id: KnotId(knot_id.to_string()),
            strand_path: StrandPath(PathBuf::from(path)),
        }
    }

    // ── ProcessStrand Tests ─────────────────────────────────────────────

    #[test]
    fn process_strand_success() {
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let store = LoomStore::new();
        store.register(loom);

        let (state_port, state_updates) =
            TrackingKnotStatePortForUpdates::new();
        let (log_port, _, log_append) = TrackingLoomLogPort::new();
        let (sink, sink_writes) = TrackingTieOffSink::new();

        let agent_runner = Arc::new(ConfigurableAgentRunner {
            result: Ok(AgentOutput {
                stdout: "agent output content".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        });

        let use_case = ProcessStrand::new(
            store,
            Arc::new(state_port),
            Arc::new(log_port),
            agent_runner,
            Arc::new(sink),
            RigAgentConfig::default_config(),
        );

        let event = build_created_event("test-loom", "k1", "src/file.md");
        let result = use_case.execute(event);

        assert!(result.is_ok());

        // Verify state transitions: processing -> completed
        let updates = state_updates.read().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].status, ProcessingStatus::Processing);
        assert_eq!(updates[1].status, ProcessingStatus::Completed);

        // Verify tie-off was written with agent content
        let writes = sink_writes.read().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].content, "agent output content");
        assert_eq!(writes[0].status, crate::domain::entities::TieOffStatus::Produced);

        // Verify loom-log got StrandProcessed (no error on success)
        let appends = log_append.read().unwrap();
        assert_eq!(appends.len(), 1);
        match &appends[0] {
            LoomEvent::StrandProcessed {
                loom_id,
                strand_path,
                error,
            } => {
                assert_eq!(*loom_id, LoomId("test-loom".to_string()));
                assert_eq!(
                    *strand_path,
                    StrandPath(PathBuf::from("src/file.md"))
                );
                assert!(error.is_none());
            }
            _ => panic!("Expected StrandProcessed event"),
        }
    }

    #[test]
    fn process_strand_agent_error() {
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let store = LoomStore::new();
        store.register(loom);

        let (state_port, state_updates) =
            TrackingKnotStatePortForUpdates::new();
        let (log_port, _, log_append) = TrackingLoomLogPort::new();
        let (sink, sink_writes) = TrackingTieOffSink::new();

        let agent_runner = Arc::new(ConfigurableAgentRunner {
            result: Err(PortError::AgentExecutionFailed(
                "agent crashed".to_string(),
            )),
        });

        let use_case = ProcessStrand::new(
            store,
            Arc::new(state_port),
            Arc::new(log_port),
            agent_runner,
            Arc::new(sink),
            RigAgentConfig::default_config(),
        );

        let event = build_created_event("test-loom", "k1", "src/file.md");
        let result = use_case.execute(event);

        // ProcessStrand returns Ok even on agent error (it records the failure)
        assert!(result.is_ok());

        // State transitions: processing -> failed
        let updates = state_updates.read().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].status, ProcessingStatus::Processing);
        assert_eq!(updates[1].status, ProcessingStatus::Failed);

        // Final state has error details
        assert_eq!(
            updates[1].error,
            Some("agent execution failed: agent crashed".to_string())
        );

        // Tie-off written with Failed status and error content
        let writes = sink_writes.read().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].status, crate::domain::entities::TieOffStatus::Failed);
        assert!(writes[0].content.contains("agent crashed"));

        // Loom-log got StrandProcessed with error details
        let appends = log_append.read().unwrap();
        assert_eq!(appends.len(), 1);
        match &appends[0] {
            LoomEvent::StrandProcessed { error, .. } => {
                assert_eq!(
                    error.as_deref(),
                    Some("agent execution failed: agent crashed")
                );
            }
            _ => panic!("Expected StrandProcessed event"),
        }
    }

    /// AgentRunner returns `PortError::CommandNotFound`; verify knot-state
    /// `error` field and loom-log event both contain the error message.
    #[test]
    fn process_strand_agent_not_found_logs_error() {
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let store = LoomStore::new();
        store.register(loom);

        let (state_port, state_updates) =
            TrackingKnotStatePortForUpdates::new();
        let (log_port, _, log_append) = TrackingLoomLogPort::new();
        let (sink, sink_writes) = TrackingTieOffSink::new();

        let agent_runner = Arc::new(ConfigurableAgentRunner {
            result: Err(PortError::CommandNotFound(
                "pi: command not found".to_string(),
            )),
        });

        let use_case = ProcessStrand::new(
            store,
            Arc::new(state_port),
            Arc::new(log_port),
            agent_runner,
            Arc::new(sink),
            RigAgentConfig::default_config(),
        );

        let event = build_created_event("test-loom", "k1", "src/file.md");
        let result = use_case.execute(event);

        // ProcessStrand returns Ok (failure is recorded, not propagated)
        assert!(result.is_ok());

        // State: processing -> failed, with error message
        let updates = state_updates.read().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[1].status, ProcessingStatus::Failed);
        assert_eq!(
            updates[1].error,
            Some("command not found: pi: command not found".to_string())
        );

        // Tie-off has Failed status
        let writes = sink_writes.read().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].status, crate::domain::entities::TieOffStatus::Failed);

        // Loom-log event contains error details
        let appends = log_append.read().unwrap();
        assert_eq!(appends.len(), 1);
        match &appends[0] {
            LoomEvent::StrandProcessed { error, .. } => {
                assert_eq!(
                    error.as_deref(),
                    Some("command not found: pi: command not found")
                );
            }
            _ => panic!("Expected StrandProcessed event"),
        }
    }

    /// AgentRunner returns `PortError::AgentExecutionFailed`; verify
    /// knot-state `error` field and loom-log event contain the message.
    #[test]
    fn process_strand_agent_nonzero_exit_logs_error() {
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let store = LoomStore::new();
        store.register(loom);

        let (state_port, state_updates) =
            TrackingKnotStatePortForUpdates::new();
        let (log_port, _, log_append) = TrackingLoomLogPort::new();
        let (sink, sink_writes) = TrackingTieOffSink::new();

        let agent_runner = Arc::new(ConfigurableAgentRunner {
            result: Err(PortError::AgentExecutionFailed(
                "exit code 1".to_string(),
            )),
        });

        let use_case = ProcessStrand::new(
            store,
            Arc::new(state_port),
            Arc::new(log_port),
            agent_runner,
            Arc::new(sink),
            RigAgentConfig::default_config(),
        );

        let event = build_created_event("test-loom", "k1", "src/file.md");
        let result = use_case.execute(event);

        assert!(result.is_ok());

        // State: processing -> failed, with error message
        let updates = state_updates.read().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[1].status, ProcessingStatus::Failed);
        assert_eq!(
            updates[1].error,
            Some("agent execution failed: exit code 1".to_string())
        );

        // Tie-off has Failed status
        let writes = sink_writes.read().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].status, crate::domain::entities::TieOffStatus::Failed);

        // Loom-log event contains error details
        let appends = log_append.read().unwrap();
        assert_eq!(appends.len(), 1);
        match &appends[0] {
            LoomEvent::StrandProcessed { error, .. } => {
                assert_eq!(
                    error.as_deref(),
                    Some("agent execution failed: exit code 1")
                );
            }
            _ => panic!("Expected StrandProcessed event"),
        }
    }

    #[test]
    fn process_strand_state_transitions() {
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let store = LoomStore::new();
        store.register(loom);

        let (state_port, state_updates) =
            TrackingKnotStatePortForUpdates::new();
        let (log_port, _, _) = TrackingLoomLogPort::new();
        let (sink, _) = TrackingTieOffSink::new();

        let agent_runner = Arc::new(ConfigurableAgentRunner {
            result: Ok(AgentOutput {
                stdout: "ok".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        });

        let use_case = ProcessStrand::new(
            store,
            Arc::new(state_port),
            Arc::new(log_port),
            agent_runner,
            Arc::new(sink),
            RigAgentConfig::default_config(),
        );

        let event = build_created_event("test-loom", "k1", "src/file.md");
        use_case.execute(event).unwrap();

        // Verify exact state sequence:
        // 1. Initial state is implicitly Idle (before processing)
        // 2. First update: Processing
        // 3. Second update: Completed
        let updates = state_updates.read().unwrap();
        assert_eq!(updates.len(), 2);

        // First state: Processing (transition from implicit Idle)
        assert_eq!(updates[0].status, ProcessingStatus::Processing);
        assert_eq!(updates[0].event_type, KnotEventType::Created);
        assert_eq!(updates[0].error, None);

        // Second state: Completed
        assert_eq!(updates[1].status, ProcessingStatus::Completed);
        assert_eq!(updates[1].event_type, KnotEventType::Created);
        assert_eq!(updates[1].error, None);

        // Both states reference the same strand
        assert_eq!(updates[0].strand_path, updates[1].strand_path);
    }

    #[test]
    fn process_strand_deleted_event() {
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let store = LoomStore::new();
        store.register(loom);

        let (state_port, state_updates) =
            TrackingKnotStatePortForUpdates::new();
        let (log_port, _, log_append) = TrackingLoomLogPort::new();
        let (sink, sink_writes) = TrackingTieOffSink::new();

        // Agent runner should NOT be called for deleted events
        let agent_runner = Arc::new(ConfigurableAgentRunner {
            result: Err(PortError::AgentExecutionFailed(
                "should not be called".to_string(),
            )),
        });

        let use_case = ProcessStrand::new(
            store,
            Arc::new(state_port),
            Arc::new(log_port),
            agent_runner,
            Arc::new(sink),
            RigAgentConfig::default_config(),
        );

        let event = build_deleted_event("test-loom", "k1", "src/file.md");
        let result = use_case.execute(event);

        // Should succeed (deleted events do not error)
        assert!(result.is_ok());

        // State transitions: processing -> completed
        let updates = state_updates.read().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].status, ProcessingStatus::Processing);
        assert_eq!(updates[1].status, ProcessingStatus::Completed);
        assert_eq!(updates[0].event_type, KnotEventType::Deleted);

        // Tie-off still written (reports what was undone)
        let writes = sink_writes.read().unwrap();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].content.contains("deleted"));
        assert_eq!(writes[0].status, crate::domain::entities::TieOffStatus::Produced);

        // Loom-log got StrandProcessed (no error for deleted)
        let appends = log_append.read().unwrap();
        assert_eq!(appends.len(), 1);
        match &appends[0] {
            LoomEvent::StrandProcessed { error, .. } => {
                assert!(error.is_none());
            }
            _ => panic!("Expected StrandProcessed event"),
        }
    }

    /// Tracking mock `AgentRunner` that records the `ExecutionContext` it
    /// receives, then returns a configurable result.
    struct TrackingAgentRunner {
        captured_ctx: Arc<RwLock<Option<ExecutionContext>>>,
        result: Result<AgentOutput, PortError>,
    }

    impl TrackingAgentRunner {
        fn new(
            result: Result<AgentOutput, PortError>,
        ) -> (
            Self,
            Arc<RwLock<Option<ExecutionContext>>>,
        ) {
            let captured_ctx = Arc::new(RwLock::new(None));
            let runner = Self {
                captured_ctx: captured_ctx.clone(),
                result,
            };
            (runner, captured_ctx)
        }
    }

    impl AgentRunner for TrackingAgentRunner {
        fn execute(
            &self,
            ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            self.captured_ctx.write().unwrap().replace(ctx);
            self.result.clone()
        }
    }

    /// Verify that `ProcessStrand` constructs CLI args from the knot's
    /// `AgentConfig` + `PromptTemplate` instead of using raw
    /// `RigAgentConfig.cli_args`.
    #[test]
    fn process_strand_builds_pi_cli_args() {
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let store = LoomStore::new();
        store.register(loom);

        let (state_port, _) = TrackingKnotStatePortForUpdates::new();
        let (log_port, _, _) = TrackingLoomLogPort::new();
        let (sink, _) = TrackingTieOffSink::new();

        let (agent_runner, captured_ctx) = TrackingAgentRunner::new(Ok(
            AgentOutput {
                stdout: "ok".to_string(),
                stderr: String::new(),
                exit_code: 0,
            },
        ));

        let use_case = ProcessStrand::new(
            store,
            Arc::new(state_port),
            Arc::new(log_port),
            Arc::new(agent_runner),
            Arc::new(sink),
            RigAgentConfig::default_config(),
        );

        let event = build_created_event("test-loom", "k1", "src/file.md");
        use_case.execute(event).unwrap();

        // Verify the captured ExecutionContext has the correct args
        let ctx = captured_ctx.read().unwrap();
        let ctx = ctx.as_ref().expect("agent runner should have been called");

        // cli_path should come from rig config (default: "pi")
        assert_eq!(ctx.cli_path, "pi");

        // cli_args should be built from knot config, NOT workspace cli_args
        // Expected: -p --model gpt-4o --system-prompt check it --no-session
        //           --no-tools @src/file.md
        let args = &ctx.cli_args;
        assert!(
            args.contains(&"-p".to_string()),
            "args should contain -p flag"
        );
        assert!(
            args.contains(&"--model".to_string()),
            "args should contain --model flag"
        );
        assert!(
            args.contains(&"gpt-4o".to_string()),
            "args should contain model name from knot config"
        );
        assert!(
            args.contains(&"--system-prompt".to_string()),
            "args should contain --system-prompt flag"
        );
        assert!(
            args.contains(&"check it".to_string()),
            "args should contain instructions from prompt template"
        );
        assert!(
            args.contains(&"--no-session".to_string()),
            "args should contain --no-session flag"
        );
        assert!(
            args.contains(&"--no-tools".to_string()),
            "args should contain --no-tools flag (no tools configured)"
        );
        // Strand path appended with @ prefix
        let has_strand_ref = args
            .iter()
            .any(|a| a.starts_with("@src/file.md"));
        assert!(
            has_strand_ref,
            "args should contain @strand_path reference"
        );
    }

    /// Verify that `ProcessStrand` passes the prompt (from
    /// `prompt_template.instructions`) and strand path into the
    /// `ExecutionContext`.
    #[test]
    fn process_strand_passes_prompt_and_strand_to_context() {
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let store = LoomStore::new();
        store.register(loom);

        let (state_port, _) = TrackingKnotStatePortForUpdates::new();
        let (log_port, _, _) = TrackingLoomLogPort::new();
        let (sink, _) = TrackingTieOffSink::new();

        let (agent_runner, captured_ctx) = TrackingAgentRunner::new(Ok(
            AgentOutput {
                stdout: "ok".to_string(),
                stderr: String::new(),
                exit_code: 0,
            },
        ));

        let use_case = ProcessStrand::new(
            store,
            Arc::new(state_port),
            Arc::new(log_port),
            Arc::new(agent_runner),
            Arc::new(sink),
            RigAgentConfig::default_config(),
        );

        let event = build_created_event("test-loom", "k1", "src/file.md");
        use_case.execute(event).unwrap();

        let ctx = captured_ctx.read().unwrap();
        let ctx = ctx.as_ref().expect("agent runner should have been called");

        // Prompt comes from knot's prompt_template.instructions
        assert_eq!(
            ctx.prompt, "check it",
            "prompt should be from prompt template instructions"
        );

        // Strand path is carried in the context
        assert_eq!(
            ctx.strand_path,
            StrandPath(PathBuf::from("src/file.md")),
            "strand_path should match the event strand"
        );
    }
}
