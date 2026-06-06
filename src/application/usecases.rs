//! Application-layer use cases.
//!
//! Each use case orchestrates domain entities through port traits and the
//! in-memory loom store. Tests use mock port implementations — no IO.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::ports::{
    AgentRunner, ExecutionContext, EventSource, KnotEventType,
    LoomLogPort, LoomRepository, ProcessingStatus, PortError,
    TieOffSink,
};
use crate::application::store::LoomStore;
use crate::domain::entities::{Knot, KnotId, Loom, LoomId, StrandPath, TieOff, TieOffPath};
use crate::domain::events::{LoomEvent, StrandEvent};
use crate::domain::value_objects::RigAgentConfig;

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

/// Use case: discover looms in a workspace and register any new ones.
///
/// Calls `LoomRepository::scan()` to find looms, then for each loom
/// not already in `LoomStore`:
/// - Opens the loom activity log via `LoomLogPort::open()`
/// - Appends `KnotRegistered` to the loom log via `LoomLogPort::append()`
/// - Appends `LoomStarted` to the loom log via `LoomLogPort::append()`
/// - Registers the loom in `LoomStore`
/// - Starts file watchers via `EventSource::watch()`
///
/// Looms already present in the store are skipped — no duplicate
/// registration or watcher starts. Returns only the newly discovered
/// (previously unknown) loom IDs.
pub struct DiscoverLooms {
    repository: Arc<dyn LoomRepository>,
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
}

impl DiscoverLooms {
    /// Create a new `DiscoverLooms` use case.
    pub fn new(
        repository: Arc<dyn LoomRepository>,
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
        event_source: Arc<dyn EventSource>,
    ) -> Self {
        Self {
            repository,
            log_port,
            store,
            event_source,
        }
    }

    /// Execute discovery against the given workspace path.
    ///
    /// Returns the list of *newly* discovered looms (those not already
    /// in the store). Already-registered looms are silently skipped.
    pub fn execute(&self, workspace: &Path) -> Result<Vec<Loom>, PortError> {
        let looms = self.repository.scan(workspace)?;
        let mut new_looms = Vec::new();

        for loom in &looms {
            // Skip looms already registered in the store
            if self.store.get(&loom.id).is_some() {
                continue;
            }

            self.register_single(loom)?;
            new_looms.push(loom.clone());
        }

        Ok(new_looms)
    }

    /// Register a single loom: log events, store, and start watchers.
    fn register_single(&self, loom: &Loom) -> Result<(), PortError> {
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
        self.store.register(loom.clone());

        // Start file watchers for each knot's strand directory
        for knot in &loom.knots {
            self.event_source.set_loom_ids(
                &knot.strand_dir,
                &loom.id,
                &knot.id,
            );
            self.event_source.watch(&knot.strand_dir)
                .map_err(|e| {
                    PortError::LoomSaveFailed(format!(
                        "failed to watch '{}': {}",
                        knot.strand_dir.display(),
                        e
                    ))
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
/// 5. Starts file watchers for each knot's effective source directory
///
/// Returns an error if a loom with the same ID already exists.
pub struct RegisterLoom {
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
}

impl RegisterLoom {
    /// Create a new `RegisterLoom` use case.
    pub fn new(
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
        event_source: Arc<dyn EventSource>,
    ) -> Self {
        Self {
            log_port,
            store,
            event_source,
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
        self.store.register(loom.clone());

        // Start file watchers for each knot's strand directory
        for knot in &loom.knots {
            self.event_source.set_loom_ids(
                &knot.strand_dir,
                &loom.id,
                &knot.id,
            );
            self.event_source.watch(&knot.strand_dir)
                .map_err(|e| {
                    PortError::LoomSaveFailed(format!(
                        "failed to watch '{}': {}",
                        knot.strand_dir.display(),
                        e
                    ))
                })?;
        }

        Ok(())
    }
}

// ── UnregisterLoom ─────────────────────────────────────────────────────────

/// Use case: unregister a loom.
///
/// 1. Looks up the loom in `LoomStore`
/// 2. Calls `EventSource::unwatch()` for each effective source directory
/// 3. Appends `LoomStopped` event via `LoomLogPort::append()`
/// 4. Removes the loom from `LoomStore`
///
/// Returns an error if the loom was not found.
pub struct UnregisterLoom {
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
}

impl UnregisterLoom {
    /// Create a new `UnregisterLoom` use case.
    pub fn new(
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
        event_source: Arc<dyn EventSource>,
    ) -> Self {
        Self {
            log_port,
            store,
            event_source,
        }
    }

    /// Unregister the loom with the given ID.
    ///
    /// Returns `PortError::LoomNotFound` if the loom is not in the store.
    pub fn execute(&self, id: &LoomId) -> Result<(), PortError> {
        // Check loom exists before any side effects
        let loom = self.store.get(id)
            .ok_or_else(|| PortError::LoomNotFound(id.clone()))?;

        // Stop watching strand directories for each knot
        for knot in &loom.knots {
            self.event_source.unwatch(&knot.strand_dir)
                .map_err(|e| {
                    PortError::EventUnwatchFailed(format!(
                        "failed to unwatch '{}': {}",
                        knot.strand_dir.display(),
                        e
                    ))
                })?;
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
                source_dir: PathBuf::from(""),
                tie_off_dir: PathBuf::from(""),
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

        let event_type = match event {
            StrandEvent::Created { .. } => KnotEventType::Created,
            StrandEvent::Modified { .. } => KnotEventType::Modified,
            StrandEvent::Deleted { .. } => KnotEventType::Deleted,
        };
        let event_label = match event_type {
            KnotEventType::Created => "Created".to_string(),
            KnotEventType::Modified => "Modified".to_string(),
            KnotEventType::Deleted => "Deleted".to_string(),
        };

        // Determine tie-off path (knot-level tie_off_dir if set, else loom-level)
        let tie_off_path = Self::compute_tie_off_path(&loom, knot, &strand_path);

        // 1. Append KnotProcessing to loom-log
        self.log_port.append(LoomEvent::KnotProcessing {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
        })?;

        // 2. Build CLI args from knot's agent config + prompt template
        let mut cli_args =
            knot.agent_config.build_cli_args(&knot.prompt_template);
        // Append strand content reference using pi's @file syntax
        cli_args.push(
            format!("@{}", strand_path.0.display()),
        );

        // 3. Read previous tie-off content (for event context)
        let previous_tie_off = self.tie_off_sink
            .read_content(&tie_off_path)
            .unwrap_or_default();

        // 4. Build execution context with event metadata
        let ctx = ExecutionContext {
            cli_path: self.rig_config.cli_path.clone(),
            cli_args,
            prompt: knot.prompt_template.instructions.clone(),
            strand_path: strand_path.clone(),
            event_type: event_label.clone(),
            previous_tie_off,
        };

        // 5. Execute agent and handle result
        let result = self.agent_runner.execute(ctx);

        match result {
            Ok(output) => {
                // 4. Write successful tie-off
                let tie_off = TieOff {
                    content: output.stdout,
                    path: tie_off_path.clone(),
                    status: crate::domain::entities::TieOffStatus::Produced,
                    event_type: Some(event_label.clone()),
                    strand_path: Some(strand_path.0.display().to_string()),
                    timestamp: None,
                };
                self.tie_off_sink.append(tie_off)?;

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
                    event_type: Some(event_label.clone()),
                    strand_path: Some(strand_path.0.display().to_string()),
                    timestamp: None,
                };
                self.tie_off_sink.append(tie_off)?;

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

    /// Compute the tie-off output path from knot + strand path.
    /// Uses the knot's required `tie_off_dir`.
    fn compute_tie_off_path(
        _loom: &Loom,
        knot: &Knot,
        strand_path: &StrandPath,
    ) -> TieOffPath {
        let filename = strand_path
            .0
            .file_name()
            .map(|f| format!("{}.output", f.to_string_lossy()))
            .unwrap_or_else(|| "output".to_string());
        TieOffPath(knot.tie_off_dir.join(filename))
    }

}



// ── Phase 2 Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod phase2_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    // ── Tracking EventSource Mock ──────────────────────────────────────

    /// A mock `EventSource` that records all `watch()` calls.
    struct TrackingEventSource {
        watch_calls: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl TrackingEventSource {
        fn new(
        ) -> (Self, Arc<Mutex<Vec<PathBuf>>>) {
            let watch_calls = Arc::new(Mutex::new(vec![]));
            let source = Self {
                watch_calls: watch_calls.clone(),
            };
            (source, watch_calls)
        }
    }

    impl EventSource for TrackingEventSource {
        fn watch(&self, path: &std::path::Path) -> Result<(), PortError> {
            self.watch_calls
                .lock()
                .unwrap()
                .push(path.to_path_buf());
            Ok(())
        }

        fn unwatch(&self, _path: &std::path::Path) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── Mock LoomLogPort ───────────────────────────────────────────────

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

    // ── Helpers ────────────────────────────────────────────────────────

    /// Build a loom with the given ID and optional knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    /// Build a knot with the given ID.
        fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_config: AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            },
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            tie_off_dir: PathBuf::from("tie-offs"),
        }
    }

    // ── RegisterLoom Watcher Tests ─────────────────────────────────────

    /// `RegisterLoom` with mock `EventSource`: after registration,
    /// `watch()` is called for each knot's strand directory.
    #[test]
    fn register_loom_starts_watchers() {
        let loom = build_loom(
            "watch-loom",
            vec![
                build_knot("k1"),
                build_knot("k2"),
            ],
        );
        let loom_id = loom.id.clone();

        let (event_source, watch_calls) = TrackingEventSource::new();
        let store = LoomStore::new();

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(loom);

        // Should succeed
        assert!(result.is_ok());

        // watch() called for each knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 2);

        // Both knots use their strand_dir ("strands")
        let watched: HashSet<_> =
            watches.iter().map(|p| p.as_path()).collect();
        assert!(watched.contains(Path::new("strands")));

        // Loom is in the store
        assert!(store.get(&loom_id).is_some());
    }

    /// `RegisterLoom` with no knots registers the loom without watchers.
    #[test]
    fn register_loom_starts_watcher_empty_knots() {
        let loom = build_loom("empty-loom", vec![]);
        let loom_id = loom.id.clone();

        let (event_source, watch_calls) = TrackingEventSource::new();
        let store = LoomStore::new();

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(loom);

        assert!(result.is_ok());

        // No knots means no watches
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 0);

        assert!(store.get(&loom_id).is_some());
    }

    /// `RegisterLoom` duplicate ID returns error without starting watchers.
    #[test]
    fn register_loom_duplicate_no_watchers() {
        let loom1 = build_loom("dup", vec![build_knot("k1")]);
        let loom2 = build_loom("dup", vec![build_knot("k2")]);

        let (event_source, watch_calls) = TrackingEventSource::new();
        let store = LoomStore::new();

        // Register first loom
        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            Arc::clone(&es),
        );
        assert!(use_case.execute(loom1).is_ok());

        // Verify first registration started a watcher
        {
            let watches = watch_calls.lock().unwrap();
            assert_eq!(watches.len(), 1);
        }

        // Attempt duplicate registration
        let (event_source2, watch_calls2) = TrackingEventSource::new();
        let es2: Arc<dyn EventSource> = Arc::new(event_source2);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es2,
        );
        let result = use_case.execute(loom2);

        // Should fail with LoomSaveFailed
        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomSaveFailed(msg) => {
                assert!(msg.contains("already registered"));
            }
            other => panic!("Expected LoomSaveFailed, got {other:?}"),
        }

        // No new watchers started for the duplicate
        let watches = watch_calls2.lock().unwrap();
        assert!(watches.is_empty());

        // Original store unchanged
        let stored = store.get(&LoomId("dup".to_string())).unwrap();
        assert_eq!(stored.knots.len(), 1);
        assert_eq!(stored.knots[0].id, KnotId("k1".to_string()));
    }
}

// ── Phase 3 Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod phase3_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    // ── Tracking EventSource Mock ──────────────────────────────────────

    /// A mock `EventSource` that records all `watch()` and `unwatch()` calls.
    struct TrackingEventSource {
        watch_calls: Arc<Mutex<Vec<PathBuf>>>,
        unwatch_calls: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl TrackingEventSource {
        fn new(
        ) -> (
            Self,
            Arc<Mutex<Vec<PathBuf>>>,
            Arc<Mutex<Vec<PathBuf>>>,
        ) {
            let watch_calls = Arc::new(Mutex::new(vec![]));
            let unwatch_calls = Arc::new(Mutex::new(vec![]));
            let source = Self {
                watch_calls: watch_calls.clone(),
                unwatch_calls: unwatch_calls.clone(),
            };
            (source, watch_calls, unwatch_calls)
        }
    }

    impl EventSource for TrackingEventSource {
        fn watch(&self, path: &std::path::Path) -> Result<(), PortError> {
            self.watch_calls
                .lock()
                .unwrap()
                .push(path.to_path_buf());
            Ok(())
        }

        fn unwatch(&self, path: &std::path::Path) -> Result<(), PortError> {
            self.unwatch_calls
                .lock()
                .unwrap()
                .push(path.to_path_buf());
            Ok(())
        }
    }

    // ── Mock LoomLogPort ───────────────────────────────────────────────

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

    // ── Helpers ────────────────────────────────────────────────────────

    /// Build a loom with the given ID and optional knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    /// Build a knot with the given ID.
        fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_config: AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            },
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            tie_off_dir: PathBuf::from("tie-offs"),
        }
    }

    // ── UnregisterLoom Watcher Tests ───────────────────────────────────

    /// `UnregisterLoom` with mock `EventSource`: after unregistration,
    /// `unwatch()` is called for each knot's strand directory.
    #[test]
    fn unregister_loom_stops_watchers() {
        let loom = build_loom(
            "unwatch-loom",
            vec![
                build_knot("k1"),
                build_knot("k2"),
            ],
        );
        let loom_id = loom.id.clone();

        let (event_source, _watch_calls, unwatch_calls) =
            TrackingEventSource::new();
        let store = LoomStore::new();

        // Register the loom first (so it exists in the store)
        store.register(loom);

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = UnregisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(&loom_id);

        // Should succeed
        assert!(result.is_ok());

        // unwatch() called for each knot's strand directory
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 2);

        // Both knots use their strand_dir ("strands")
        let unwatched: HashSet<_> =
            unwatches.iter().map(|p| p.as_path()).collect();
        assert!(unwatched.contains(Path::new("strands")));

        // Loom is no longer in the store
        assert!(store.get(&loom_id).is_none());
    }

    /// `UnregisterLoom` with no knots unregisters without unwatch.
    #[test]
    fn unregister_loom_stops_watcher_empty_knots() {
        let loom = build_loom("empty-unwatch-loom", vec![]);
        let loom_id = loom.id.clone();

        let (event_source, _watch_calls, unwatch_calls) =
            TrackingEventSource::new();
        let store = LoomStore::new();

        // Register the loom first
        store.register(loom);

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = UnregisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(&loom_id);

        assert!(result.is_ok());

        // No knots means no unwatch
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 0);

        assert!(store.get(&loom_id).is_none());
    }

    /// `UnregisterLoom` for unknown loom returns error without calling
    /// `unwatch()`.
    #[test]
    fn unregister_loom_not_found_no_unwatch() {
        let (event_source, _watch_calls, unwatch_calls) =
            TrackingEventSource::new();
        let store = LoomStore::new();

        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = UnregisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result =
            use_case.execute(&LoomId("nonexistent".to_string()));

        // Should fail with LoomNotFound
        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("nonexistent".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }

        // No unwatch calls
        let unwatches = unwatch_calls.lock().unwrap();
        assert!(unwatches.is_empty());
    }
}

// ── Phase 4 Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod phase4_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    // ── Tracking EventSource Mock ──────────────────────────────────────

    /// A mock `EventSource` that records all `watch()` calls.
    struct TrackingEventSource {
        watch_calls: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl TrackingEventSource {
        fn new(
        ) -> (Self, Arc<Mutex<Vec<PathBuf>>>) {
            let watch_calls = Arc::new(Mutex::new(vec![]));
            let source = Self {
                watch_calls: watch_calls.clone(),
            };
            (source, watch_calls)
        }
    }

    impl EventSource for TrackingEventSource {
        fn watch(&self, path: &std::path::Path) -> Result<(), PortError> {
            self.watch_calls
                .lock()
                .unwrap()
                .push(path.to_path_buf());
            Ok(())
        }

        fn unwatch(&self, _path: &std::path::Path) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── Mock LoomLogPort ───────────────────────────────────────────────

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

    // ── Mock LoomRepository ────────────────────────────────────────────

    struct MockLoomRepository {
        scan_result: Vec<Loom>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(&self, _rig: &std::path::Path) -> Result<Vec<Loom>, PortError> {
            Ok(self.scan_result.clone())
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

    // ── Helpers ────────────────────────────────────────────────────────

    /// Build a loom with the given ID and optional knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    /// Build a knot with the given ID.
        fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_config: AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            },
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            tie_off_dir: PathBuf::from("tie-offs"),
        }
    }

    // ── DiscoverLooms Runtime Tests ────────────────────────────────────

    /// `DiscoverLooms` use case given looms where one ID already in store
    /// → only new looms are registered (log entries + watchers), existing
    /// ones skipped.
    #[test]
    fn discover_looms_runtime_skips_registered() {
        let existing_loom = build_loom("existing", vec![build_knot("k1")]);
        let new_loom = build_loom("new-loom", vec![build_knot("k2")]);
        let new_loom2 = build_loom("new-loom2", vec![]); // no knots

        // Pre-register one loom in the store
        let store = LoomStore::new();
        store.register(existing_loom.clone());

        let repo = Arc::new(MockLoomRepository {
            scan_result: vec![
                existing_loom.clone(),
                new_loom.clone(),
                new_loom2.clone(),
            ],
        });
        let (event_source, watch_calls) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = DiscoverLooms::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(Path::new("/workspace"));

        // Should succeed
        assert!(result.is_ok());

        // Only the 2 new looms returned (existing one skipped)
        let discovered = result.unwrap();
        assert_eq!(discovered.len(), 2);

        let ids: Vec<_> = discovered.iter().map(|l| l.id.0.as_str()).collect();
        assert!(ids.contains(&"new-loom"));
        assert!(ids.contains(&"new-loom2"));
        assert!(!ids.contains(&"existing"));

        // Watchers started only for new looms (not existing)
        let watches = watch_calls.lock().unwrap();
        // new_loom has 1 knot → 1 watch for "strands"
        // new_loom2 has 0 knots → no watch
        assert_eq!(watches.len(), 1);

        // Both new looms are in store
        assert!(store.get(&LoomId("new-loom".to_string())).is_some());
        assert!(store.get(&LoomId("new-loom2".to_string())).is_some());
    }

    /// `DiscoverLooms` with all looms already registered returns empty.
    #[test]
    fn discover_looms_all_registered_returns_empty() {
        let loom1 = build_loom("loom-a", vec![build_knot("k1")]);
        let loom2 = build_loom("loom-b", vec![build_knot("k2")]);

        let store = LoomStore::new();
        store.register(loom1.clone());
        store.register(loom2.clone());

        let repo = Arc::new(MockLoomRepository {
            scan_result: vec![loom1.clone(), loom2.clone()],
        });
        let (event_source, watch_calls) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = DiscoverLooms::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_ok());

        // No new looms discovered
        let discovered = result.unwrap();
        assert!(discovered.is_empty());

        // No watchers started
        let watches = watch_calls.lock().unwrap();
        assert!(watches.is_empty());
    }

    /// `DiscoverLooms` with empty scan returns empty (no side effects).
    #[test]
    fn discover_looms_empty_scan() {
        let store = LoomStore::new();

        let repo = Arc::new(MockLoomRepository {
            scan_result: vec![],
        });
        let (event_source, watch_calls) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = DiscoverLooms::new(
            repo,
            Arc::new(MockLoomLogPort::default()),
            store.clone(),
            es,
        );
        let result = use_case.execute(Path::new("/workspace"));

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
        let watches = watch_calls.lock().unwrap();
        assert!(watches.is_empty());
    }
}
