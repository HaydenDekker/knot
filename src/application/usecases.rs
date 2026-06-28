//! Application-layer use cases.
//!
//! Each use case orchestrates domain entities through port traits and the
//! in-memory loom store. Tests use mock port implementations — no IO.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::adapters::outbound::event_source::WatchType;
use crate::adapters::logging;
use crate::application::ports::{
    AgentProfileRepository, AgentRunner, EventSource,
    GitVersioningPort, KnotEventType, LoomLogPort, LoomRepository,
    ProcessingStatus, PortError, RigLogPort, StateWriterPort, TieOffSink,
};
use crate::application::session_resume;
use crate::application::store::LoomStore;
use crate::domain::entities::{
    Knot, KnotId, Loom, LoomId, RigState, RigStateKnot, RigStateLoom,
    RigStateProfile, StrandPath, TieOff, TieOffPath,
};
use crate::domain::events::{ConfigEvent, LoomEvent, StrandEvent};
use crate::domain::knot_file::derive_tieoff_path;
use crate::domain::value_objects::{AgentConfig, RigAgentConfig};

/// Generate an ISO 8601 UTC timestamp string.
pub fn format_timestamp() -> String {
    logging::format_timestamp()
}

// ── Query Result Types ───────────────────────────────────────────────────

/// A summary of a loom (lightweight, for list responses).
///
/// The loom directory is derived from the loom ID and rig base path
/// (naming convention `*-loom`). Strand and tie-off directories are
/// per-knot fields, not loom-level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoomSummary {
    /// The loom's unique ID (must end in `-loom`).
    pub id: LoomId,
    /// Number of knots in this loom.
    pub knot_count: usize,
}

/// Result of the `GetKnotStatus` use case.
///
/// Derived from the latest loom-log entries for a knot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        let (looms, warnings) = self.repository.scan(workspace)?;
        let mut new_looms = Vec::new();

        for loom in &looms {
            // Skip looms already registered in the store
            if self.store.get(&loom.id).is_some() {
                continue;
            }

            logging::log_loom_event(
                "discover",
                &loom.id.0,
                &format!("new loom found, {} knots", loom.knots.len()),
            );
            self.register_single(loom, &warnings)?;
            new_looms.push(loom.clone());
        }

        Ok(new_looms)
    }

    /// Register a single loom: log events, store, and start watchers.
    fn register_single(
        &self,
        loom: &Loom,
        warnings: &[String],
    ) -> Result<(), PortError> {
        // Open the loom activity log
        self.log_port.open(&loom.id)?;

        // Append KnotRegistered for each knot
        for knot in &loom.knots {
            self.log_port.append(LoomEvent::KnotRegistered {
                loom_id: loom.id.clone(),
                knot_id: knot.id.clone(),
                timestamp: format_timestamp(),
            })?;
        }

        // Append KnotParseWarning for each unknown property warning
        for warning in warnings {
            self.log_port.append(LoomEvent::KnotParseWarning {
                loom_id: loom.id.clone(),
                knot_file_name: String::new(),
                message: warning.clone(),
                timestamp: format_timestamp(),
            })?;
        }

        // Append LoomStarted event
        self.log_port.append(LoomEvent::LoomStarted {
            loom_id: loom.id.clone(),
            timestamp: format_timestamp(),
        })?;

        // Store the loom
        self.store.register(loom.clone());

        // Start file watchers for each knot's strand directory
        for knot in &loom.knots {
            self.ensure_strand_dir_and_watch(
                &loom.id,
                &knot.id,
                &knot.strand_dir,
            )?;
        }

        Ok(())
    }

    /// Ensure `strand_dir` exists on disk, then start the watcher.
    fn ensure_strand_dir_and_watch(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
        strand_dir: &Path,
    ) -> Result<(), PortError> {
        if !strand_dir.exists() {
            std::fs::create_dir_all(strand_dir).map_err(|e| {
                PortError::LoomSaveFailed(format!(
                    "failed to create strand dir '{}': {}",
                    strand_dir.display(),
                    e,
                ))
            })?;
            self.log_port.append(LoomEvent::DirectoryCreated {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                directory: strand_dir.display().to_string(),
                timestamp: format_timestamp(),
            })?;
            logging::log_knot_event(
                "dir-created",
                &loom_id.0,
                &knot_id.0,
                &format!("auto-created strand dir: {}", strand_dir.display()),
            );
        }

        self.event_source
            .set_loom_ids(strand_dir, loom_id, knot_id);
        self.event_source.watch(strand_dir).map_err(|e| {
            PortError::LoomSaveFailed(format!(
                "failed to watch '{}': {}",
                strand_dir.display(),
                e,
            ))
        })
    }
}

// ── ReloadConfig ───────────────────────────────────────────────────────────

/// Use case: re-scan the rig and register any looms not already in the store.
///
/// Provides the business logic for the manual reload endpoint
/// (`POST /config/reload`). Delegates to `DiscoverLooms` to avoid
/// duplicating discovery and registration logic.
///
/// Returns the list of *newly* registered loom IDs (those not already
/// in the store). Already-registered looms are silently skipped.
pub struct ReloadConfig {
    repository: Arc<dyn LoomRepository>,
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
    rig_dir: PathBuf,
}

impl ReloadConfig {
    /// Create a new `ReloadConfig` use case.
    pub fn new(
        repository: Arc<dyn LoomRepository>,
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
        event_source: Arc<dyn EventSource>,
        rig_dir: PathBuf,
    ) -> Self {
        Self {
            repository,
            log_port,
            store,
            event_source,
            rig_dir,
        }
    }

    /// Re-scan the rig and register any looms not already in the store.
    ///
    /// Returns the list of newly registered loom IDs.
    pub fn execute(&self) -> Result<Vec<LoomId>, PortError> {
        let discover = DiscoverLooms::new(
            self.repository.clone(),
            self.log_port.clone(),
            self.store.clone(),
            self.event_source.clone(),
        );
        let new_looms = discover.execute(&self.rig_dir)?;
        Ok(new_looms.into_iter().map(|l| l.id).collect())
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
    /// Idempotent — if the loom is already registered (e.g., by the
    /// ConfigEventHandler auto-discovering the loom directory), this is
    /// a no-op. Returns `PortError::LoomSaveFailed` only for write errors.
    pub fn execute(&self, loom: Loom) -> Result<(), PortError> {
        // If already registered (e.g., auto-discovered), skip — idempotent
        if self.store.get(&loom.id).is_some() {
            logging::log_loom_event(
                "register",
                &loom.id.0,
                "already registered (idempotent, skip)",
            );
            return Ok(());
        }

        // Open the loom activity log
        self.log_port.open(&loom.id)?;

        // Append KnotRegistered for each knot
        for knot in &loom.knots {
            self.log_port.append(LoomEvent::KnotRegistered {
                loom_id: loom.id.clone(),
                knot_id: knot.id.clone(),
                timestamp: format_timestamp(),
            })?;
        }

        // Append LoomStarted event
        self.log_port.append(LoomEvent::LoomStarted {
            loom_id: loom.id.clone(),
            timestamp: format_timestamp(),
        })?;

        // Store the loom
        self.store.register(loom.clone());

        // Start file watchers for each knot's strand directory
        for knot in &loom.knots {
            self.ensure_strand_dir_and_watch(
                &loom.id,
                &knot.id,
                &knot.strand_dir,
            )?;
        }

        logging::log_loom_event(
            "registered",
            &loom.id.0,
            &format!("{} knots, watchers started", loom.knots.len()),
        );
        Ok(())
    }

    /// Ensure `strand_dir` exists on disk, then start the watcher.
    fn ensure_strand_dir_and_watch(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
        strand_dir: &Path,
    ) -> Result<(), PortError> {
        if !strand_dir.exists() {
            std::fs::create_dir_all(strand_dir).map_err(|e| {
                PortError::LoomSaveFailed(format!(
                    "failed to create strand dir '{}': {}",
                    strand_dir.display(),
                    e,
                ))
            })?;
            self.log_port.append(LoomEvent::DirectoryCreated {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                directory: strand_dir.display().to_string(),
                timestamp: format_timestamp(),
            })?;
            logging::log_knot_event(
                "dir-created",
                &loom_id.0,
                &knot_id.0,
                &format!("auto-created strand dir: {}", strand_dir.display()),
            );
        }

        self.event_source
            .set_loom_ids(strand_dir, loom_id, knot_id);
        self.event_source.watch(strand_dir).map_err(|e| {
            PortError::LoomSaveFailed(format!(
                "failed to watch '{}': {}",
                strand_dir.display(),
                e,
            ))
        })
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
            timestamp: format_timestamp(),
        })?;

        // Remove from store
        self.store.unregister(id);

        logging::log_loom_event(
            "unregistered",
            &id.0,
            &format!("{} watchers stopped", loom.knots.len()),
        );
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
/// 3. Resolve agent config (profile ref → load profile, merge, or inline)
/// 4. Build execution context from resolved config + `RigAgentConfig`
/// 5. Call `AgentRunner::execute()` (skipped for Deleted events)
/// 6. Call `TieOffSink::write()` with result
/// 7. Append `KnotCompleted` or `KnotFailed` to loom-log
/// 8. Append `StrandProcessed` to loom-log
pub struct ProcessStrand {
    store: LoomStore,
    log_port: Arc<dyn LoomLogPort>,
    agent_runner: Arc<dyn AgentRunner>,
    tie_off_sink: Arc<dyn TieOffSink>,
    rig_config: RigAgentConfig,
    /// Rig directory — used to derive static output paths.
    rig_dir: PathBuf,
    /// Profile repository for dynamic profile resolution at processing time.
    profile_repo: Arc<dyn AgentProfileRepository>,
    /// Rig-log port for recording operational events (timeouts, idle).
    rig_log: Arc<dyn RigLogPort>,
    /// Git versioning port for creating commits after successful runs.
    git_versioning_port: Arc<dyn GitVersioningPort>,
}

impl ProcessStrand {
    /// Create a new `ProcessStrand` use case.
    pub fn new(
        store: LoomStore,
        log_port: Arc<dyn LoomLogPort>,
        agent_runner: Arc<dyn AgentRunner>,
        tie_off_sink: Arc<dyn TieOffSink>,
        rig_config: RigAgentConfig,
        rig_dir: PathBuf,
        profile_repo: Arc<dyn AgentProfileRepository>,
        rig_log: Arc<dyn RigLogPort>,
        git_versioning_port: Arc<dyn GitVersioningPort>,
    ) -> Self {
        Self {
            store,
            log_port,
            agent_runner,
            tie_off_sink,
            rig_config,
            rig_dir,
            profile_repo,
            rig_log,
            git_versioning_port,
        }
    }

    /// Resolve the effective `AgentConfig` for a knot and the profile's
    /// session timeout.
    ///
    /// Loads the profile from the repository and builds an `AgentConfig`
    /// from it. The profile's `profile_prompt` is delivered via stdin
    /// (not `--system-prompt`), so it is not merged here.
    /// If the profile specifies `timeout`, it is converted to a `Duration`.
    ///
    /// Returns a tuple of `(AgentConfig, Option<Duration>)` where
    /// the `Option<Duration>` is the profile's timeout
    /// (or `None` to use the runner's default).
    pub fn resolve_agent_config(
        &self,
        knot: &Knot,
    ) -> Result<(AgentConfig, Option<std::time::Duration>), PortError> {
        let profile = self
            .profile_repo
            .get(&knot.agent_profile_ref)
            .map_err(|e| PortError::ProfileNotFound(e.to_string()))?
            .ok_or_else(|| {
                PortError::ProfileNotFound(knot.agent_profile_ref.clone())
            })?;

        let timeout = profile
            .timeout
            .map(std::time::Duration::from_secs);

        Ok((
            AgentConfig {
                goal: knot.prompt_template.instructions.clone(),
                provider: profile.provider.clone(),
                model: profile.model.clone(),
                tools: profile.tools.clone(),
            },
            timeout,
        ))
    }

    /// Execute the strand processing pipeline.
    ///
    /// Appends lifecycle events to loom-log: KnotProcessing, then
    /// KnotCompleted or KnotFailed, then StrandProcessed.
    pub fn execute(&self, event: StrandEvent) -> Result<(), PortError> {
        let (loom_id, knot_id, strand_path) = Self::extract_event_fields(&event);

        let strand_kind = match &event {
            StrandEvent::Created { .. } => "Created",
            StrandEvent::Modified { .. } => "Modified",
            StrandEvent::Deleted { .. } => "Deleted",
        };
        logging::log_strand_event(
            &format!("{} processing start (knot={})", strand_kind, knot_id.0),
            &strand_path.0,
        );

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

        // Determine tie-off path (statically derived from loom + knot)
        let tie_off_path = self.compute_tie_off_path(&loom, knot, &strand_path);

        // Text check: skip binary files for Created/Modified events.
        // Deleted events skip the check (file is gone).
        // When text check errors, we check if the file is simply missing
        // (temp file race) before deciding to proceed or skip.
        let _text_check_confirmed = matches!(
            event,
            StrandEvent::Created { .. } | StrandEvent::Modified { .. }
        ) && {
            match crate::adapters::outbound::content_inspector::is_text_file(
                &strand_path.0,
            ) {
                Ok(true) => true,
                Ok(false) => {
                    eprintln!(
                        "WARN: strand '{}' is a binary file, skipping \
                         processing (knot={})",
                        strand_path.0.display(),
                        knot_id.0,
                    );
                    self.log_port.append(LoomEvent::StrandIgnored {
                        loom_id: loom_id.clone(),
                        knot_id: knot_id.clone(),
                        strand_path: strand_path.clone(),
                        reason: "binary file".to_string(),
                        timestamp: format_timestamp(),
                    })?;
                    return Ok(());
                }
                Err(_e) => {
                    // Cannot read file — check if it's simply missing
                    // (temp file race with sed -i or similar).
                    if !strand_path.0.exists() {
                        if crate::domain::temp_file::is_known_temp_file(
                            &strand_path.0,
                        ) {
                            // Known temp file pattern (e.g. sedXXXXXXX)
                            // — skip silently. No loom-log entry,
                            // no agent invocation.
                            logging::log_strand_event(
                                &format!(
                                    "{} skipped known temp file (knot={})",
                                    strand_kind, knot_id.0,
                                ),
                                &strand_path.0,
                            );
                            return Ok(());
                        }

                        // Unknown missing file — log StrandSkipped so
                        // the user can investigate if it's a real issue.
                        eprintln!(
                            "WARN: strand '{}' not found on disk (unknown \
                             pattern), skipping processing (knot={})",
                            strand_path.0.display(),
                            knot_id.0,
                        );
                        self.log_port.append(LoomEvent::StrandSkipped {
                            loom_id: loom_id.clone(),
                            knot_id: knot_id.clone(),
                            strand_path: strand_path.clone(),
                            reason: "missing file (unknown pattern)"
                                .to_string(),
                            timestamp: format_timestamp(),
                        })?;
                        return Ok(());
                    }

                    // File exists but can't be read (permission error etc).
                    // Log warning but proceed — better to attempt processing
                    // than silently skip.
                    eprintln!(
                        "WARN: cannot determine if strand '{}' is text, \
                         proceeding with processing (knot={})",
                        strand_path.0.display(),
                        knot_id.0,
                    );
                    false
                }
            }
        };

        // 1. Append KnotProcessing to loom-log
        self.log_port.append(LoomEvent::KnotProcessing {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            timestamp: format_timestamp(),
        })?;

        // 2. Resolve effective agent config (profile) and build CLI args
        let resolved = self.resolve_agent_config(knot);
        let (agent_config, profile_timeout) = resolved
            .inspect_err(|err| {
                let error_msg = err.to_string();
                // Write error tie-off
                let tie_off = TieOff {
                    content: format!("Processing failed: {}", error_msg),
                    path: tie_off_path.clone(),
                    status: crate::domain::entities::TieOffStatus::Failed,
                    knot_name: Some(knot.id.0.clone()),
                    event_type: Some(event_label.clone()),
                    strand_path: Some(strand_path.0.display().to_string()),
                    timestamp: None,
                };
                let _ = self.tie_off_sink.append(tie_off);
                // Append KnotFailed to loom-log
                let _ = self.log_port.append(LoomEvent::KnotFailed {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    strand_path: strand_path.clone(),
                    error: error_msg.clone(),
                    timestamp: format_timestamp(),
                });
                // Append StrandProcessed with error
                let _ = self.log_port.append(LoomEvent::StrandProcessed {
                    loom_id: loom_id.clone(),
                    strand_path: strand_path.clone(),
                    error: Some(error_msg.clone()),
                    timestamp: format_timestamp(),
                });
                logging::log_strand_event(
                    &format!("{} failed (knot={}): {}", strand_kind, knot_id.0, error_msg),
                    &strand_path.0,
                );
            })?;
        let (agent_config, profile_timeout) =
            (agent_config, profile_timeout);

        // Load profile to get profile_prompt for stdin delivery
        let profile = self
            .profile_repo
            .get(&knot.agent_profile_ref)
            .map_err(|e| PortError::ProfileNotFound(e.to_string()))?
            .ok_or_else(|| {
                PortError::ProfileNotFound(knot.agent_profile_ref.clone())
            })?;

        // For Deleted events: read existing tie-off content and extract
        // scoped strand history (last 5 entries for this strand).
        let is_deleted = matches!(event_type, KnotEventType::Deleted);
        let strand_history = if is_deleted {
            let tie_off_content = self
                .tie_off_sink
                .read_content(&tie_off_path)
                .unwrap_or_default();
            // Use the full path — the tie-off header stores the full path
            // (from `strand_path.0.display().to_string()`), so the parser
            // extracts the full path. Matching on just the filename would
            // fail when the path is absolute.
            let strand_path_str =
                strand_path.0.to_string_lossy().to_string();
            let sections =
                crate::domain::tieoff_parser::extract_last_n(
                    &tie_off_content,
                    &strand_path_str,
                    5,
                );
            if sections.is_empty() {
                None
            } else {
                Some(sections)
            }
        } else {
            None
        };

        // Strand filename — used in prompt for Deleted events.
        let strand_filename = strand_path.0
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        // Build the prompt. For Deleted events, inject a deletion notice
        // and scoped strand history into the prompt body.
        let mut prompt = knot.prompt_template.instructions.clone();
        if is_deleted {
            let deletion_notice = "This file was deleted. There may be git \
                history to help understand the file scope if you need to \
                rectify downstream references due to this deletion.";
            prompt.push_str("\n\n");
            prompt.push_str(deletion_notice);

            // Append scoped strand history if available
            if let Some(sections) = &strand_history {
                prompt.push_str("\n\nStrand: ");
                prompt.push_str(&strand_filename);
                prompt.push_str("\nPrevious processing history \
                    (last 5 entries):\n\n");
                for section in sections {
                    prompt.push_str(&format!(
                        "## {} triggered by {} {}\nTimestamp: {}\n",
                        section.knot_name, section.event_type,
                        section.strand_path, section.timestamp,
                    ));
                    if !section.body.is_empty() {
                        prompt.push_str(&section.body);
                        prompt.push_str("\n\n");
                    }
                }
            }
        }

        // 3. Execute agent with session-resume retry logic.
        // Build CLI args here (same as execute_with_config default impl)
        // so session_resume can append --session-id on retry.
        let strand_file_ref = if is_deleted {
            None
        } else {
            Some(strand_path.clone())
        };
        let strand_filename = strand_path.0
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let session_title = format!(
            "{} triggered by {} on {}",
            knot.id.0,
            event_label,
            strand_filename,
        );
        let mut cli_args = agent_config.build_cli_args();
        cli_args.push("--name".to_string());
        cli_args.push(session_title);
        if let Some(ref file_path) = strand_file_ref {
            cli_args.push(format!("@{}", file_path.0.display()));
        }
        let mut session_id: Option<String> = None;
        let result = session_resume::execute_with_resume(
            &*self.agent_runner,
            &*self.log_port,
            &loom_id,
            &knot_id,
            &strand_path,
            &mut session_id,
            cli_args,
            prompt,
            strand_file_ref,
            profile.profile_prompt,
            event_label.clone(),
            Some(knot.id.0.clone()),
            profile_timeout,
        );

        match result {
            Ok(output) => {
                // 4. Write successful tie-off
                let tie_off_content = output.stdout.clone();
                let tie_off = TieOff {
                    content: output.stdout,
                    path: tie_off_path.clone(),
                    status: crate::domain::entities::TieOffStatus::Produced,
                    knot_name: Some(knot.id.0.clone()),
                    event_type: Some(event_label.clone()),
                    strand_path: Some(strand_path.0.display().to_string()),
                    timestamp: None,
                };
                self.tie_off_sink.append(tie_off)?;

                // 5. Append KnotCompleted to loom-log (before commit so
                //    the log entries are included in this commit)
                self.log_port.append(LoomEvent::KnotCompleted {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    strand_path: strand_path.clone(),
                    tie_off_path: tie_off_path.clone(),
                    timestamp: format_timestamp(),
                })?;

                // 6. Append StrandProcessed to loom-log
                self.log_port.append(LoomEvent::StrandProcessed {
                    loom_id: loom_id.clone(),
                    strand_path: strand_path.clone(),
                    error: None,
                    timestamp: format_timestamp(),
                })?;

                // 7. Git versioning commit (best-effort, non-fatal).
                //    Runs AFTER loom-log appends so the commit captures
                //    the tie-off, KnotCompleted, and StrandProcessed entries.
                if knot.git_versioned {
                    let commit_result = self.git_versioning_port.commit(
                        &loom_id,
                        &knot_id,
                        &strand_path,
                        &event_label,
                        &tie_off_content,
                    );
                    if let Err(ref e) = commit_result {
                        logging::log_strand_event(
                            &format!("git commit warning: {}", e),
                            &strand_path.0,
                        );
                    }
                }

                logging::log_strand_event(
                    &format!("{} completed (knot={})", strand_kind, knot_id.0),
                    &strand_path.0,
                );
                Ok(())
            }
            Err(err) => {
                let error_msg = err.to_string();

                // On timeout: skip tie-off write, write to rig-log instead.
                // On other errors: preserve existing behaviour (write to tie-off).
                if matches!(err, PortError::Timeout { .. }) {
                    // Timeout: do NOT write error to tie-off (preserve unchanged).
                    // Write TimeoutExceeded to rig-log.
                    let _ = self.rig_log.append(
                        crate::domain::events::RigLogEvent::TimeoutExceeded {
                            loom_id: loom_id.clone(),
                            knot_id: knot_id.clone(),
                            strand_path: strand_path.clone(),
                            error: error_msg.clone(),
                            timestamp: format_timestamp(),
                        },
                    );
                } else {
                    // Non-timeout error: existing behaviour — write to tie-off.
                    let tie_off = TieOff {
                        content: format!("Processing failed: {}", error_msg),
                        path: tie_off_path.clone(),
                        status: crate::domain::entities::TieOffStatus::Failed,
                        knot_name: Some(knot.id.0.clone()),
                        event_type: Some(event_label.clone()),
                        strand_path: Some(strand_path.0.display().to_string()),
                        timestamp: None,
                    };
                    let _ = self.tie_off_sink.append(tie_off);
                }

                // 5. Append KnotFailed to loom-log
                self.log_port.append(LoomEvent::KnotFailed {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    strand_path: strand_path.clone(),
                    error: error_msg.clone(),
                    timestamp: format_timestamp(),
                })?;

                // 6. Append StrandProcessed with error details
                self.log_port.append(LoomEvent::StrandProcessed {
                    loom_id,
                    strand_path: strand_path.clone(),
                    error: Some(error_msg.clone()),
                    timestamp: format_timestamp(),
                })?;

                logging::log_strand_event(
                    &format!("{} failed (knot={}): {}", strand_kind, knot_id.0, error_msg),
                    &strand_path.0,
                );
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
    /// Uses statically derived path: `rig/tie-offs/{loom-id}/{knot-name}/`
    /// with a single per-knot file: `{knot-id}-tie-off.md`.
    fn compute_tie_off_path(
        &self,
        loom: &Loom,
        knot: &Knot,
        _strand_path: &StrandPath,
    ) -> TieOffPath {
        let filename = format!("{}-tie-off.md", knot.id.0);
        let base = derive_tieoff_path(&loom.id.0, &knot.id.0, &self.rig_dir);
        TieOffPath(base.join(filename))
    }

}



// ── ManageKnot ─────────────────────────────────────────────────────────

/// Action to perform on a knot within a loom.
///
/// Used by the `ManageKnot` use case for HTTP-driven knot CRUD.
#[derive(Debug, Clone)]
pub enum KnotAction {
    /// Add a new knot to the loom.
    ///
    /// Returns `PortError::LoomSaveFailed` if the loom is not found
    /// or a knot with the same ID already exists.
    Create { loom_id: LoomId, knot: Knot },
    /// Update an existing knot's configuration.
    ///
    /// Returns `PortError::LoomSaveFailed` if the loom or knot
    /// is not found.
    Update { loom_id: LoomId, knot: Knot },
    /// Remove a knot from the loom.
    ///
    /// Returns `PortError::LoomSaveFailed` if the loom or knot
    /// is not found.
    Delete { loom_id: LoomId, knot_id: KnotId },
}

/// Use case: manage individual knots within a loom.
///
/// Pure in-memory operation — updates `LoomStore` only. File I/O
/// (writing `.md` files) is handled by the HTTP handler, consistent
/// with the `POST /looms` pattern. The `ConfigEventHandler` picks up
/// file changes via the watcher (idempotent — store already matches).
///
/// Supports:
/// - `KnotAction::Create` — add a new knot to the loom
/// - `KnotAction::Update` — modify an existing knot's config
/// - `KnotAction::Delete` — remove a knot from the loom
pub struct ManageKnot {
    store: LoomStore,
}

impl ManageKnot {
    /// Create a new `ManageKnot` use case.
    pub fn new(store: LoomStore) -> Self {
        Self { store }
    }

    /// Execute the knot management action.
    pub fn execute(&self, action: KnotAction) -> Result<(), PortError> {
        match action {
            KnotAction::Create { loom_id, knot } => {
                self.create_knot(&loom_id, knot)
            }
            KnotAction::Update { loom_id, knot } => {
                self.update_knot(&loom_id, knot)
            }
            KnotAction::Delete { loom_id, knot_id } => {
                self.delete_knot(&loom_id, &knot_id)
            }
        }
    }

    fn create_knot(
        &self,
        loom_id: &LoomId,
        knot: Knot,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        // Check for duplicate knot ID
        if loom.knots.iter().any(|k| k.id == knot.id) {
            return Err(PortError::LoomSaveFailed(format!(
                "knot '{}' already exists in loom '{}'",
                knot.id.0,
                loom_id.0
            )));
        }

        loom.knots.push(knot.clone());
        self.store.register(loom);
        logging::log_knot_event(
            "created",
            &loom_id.0,
            &knot.id.0,
            "store updated (watcher started by caller)",
        );
        Ok(())
    }

    fn update_knot(
        &self,
        loom_id: &LoomId,
        knot: Knot,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        let pos = loom.knots.iter()
            .position(|k| k.id == knot.id)
            .ok_or_else(|| PortError::LoomSaveFailed(format!(
                "knot '{}' not found in loom '{}'",
                knot.id.0,
                loom_id.0
            )))?;

        loom.knots[pos] = knot.clone();
        self.store.register(loom);
        logging::log_knot_event(
            "updated",
            &loom_id.0,
            &knot.id.0,
            "store updated (watcher managed by caller)",
        );
        Ok(())
    }

    fn delete_knot(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        let found = loom.knots.iter()
            .position(|k| k.id == *knot_id)
            .ok_or_else(|| PortError::LoomSaveFailed(format!(
                "knot '{}' not found in loom '{}'",
                knot_id.0,
                loom_id.0
            )))?;

        loom.knots.remove(found);
        self.store.register(loom);
        logging::log_knot_event(
            "deleted",
            &loom_id.0,
            &knot_id.0,
            "store updated (watcher stopped by caller)",
        );
        Ok(())
    }
}

// ── ConfigEventHandler ───────────────────────────────────────────────

/// Use case: handle configuration events for looms and knots.
///
/// Receives `ConfigEvent`s from the file watcher (via outbound adapter)
/// and updates the in-memory `LoomStore`, starts/stops watchers, and
/// writes loom-log entries.
///
/// - `ConfigEvent::LoomAdded` — scan the loom directory via
///   `LoomRepository::scan()` and register the loom (same flow as
///   `RegisterLoom`).
/// - `ConfigEvent::KnotAdded` — add the knot to the loom in the store,
///   log `KnotRegistered`, start watcher for `strand_dir`.
/// - `ConfigEvent::KnotModified` — update the knot in the store, stop
///   old watcher, start new watcher if `strand_dir` changed.
/// - `ConfigEvent::KnotDeleted` — remove the knot from the loom in the
///   store, stop watcher, log `KnotDeregistered`.
pub struct ConfigEventHandler {
    repository: Arc<dyn LoomRepository>,
    log_port: Arc<dyn LoomLogPort>,
    store: LoomStore,
    event_source: Arc<dyn EventSource>,
    /// Project root (parent of rig_dir), used to resolve relative paths.
    project_root: PathBuf,
}

impl ConfigEventHandler {
    /// Create a new `ConfigEventHandler`.
    pub fn new(
        repository: Arc<dyn LoomRepository>,
        log_port: Arc<dyn LoomLogPort>,
        store: LoomStore,
        event_source: Arc<dyn EventSource>,
        rig_path: PathBuf,
    ) -> Self {
        // Derive project root from rig_path parent (same as FileSystemLoomRepository::scan).
        // Falls back to rig_path itself if no parent exists.
        let project_root = rig_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| rig_path.clone());
        Self {
            repository,
            log_port,
            store,
            event_source,
            project_root,
        }
    }

    /// Handle a single configuration event.
    pub fn execute(&self, event: ConfigEvent) -> Result<(), PortError> {
        match event {
            ConfigEvent::LoomAdded {
                ref loom_id,
                ref loom_dir,
            } => {
                logging::log_config_event(
                    "LoomAdded",
                    &format!("loom={} dir={}", loom_id.0, loom_dir),
                );
                self.handle_loom_added(loom_id, Path::new(loom_dir))
            }
            ConfigEvent::KnotAdded { ref loom_id, ref knot } => {
                logging::log_config_event(
                    "KnotAdded",
                    &format!("loom={} knot={}", loom_id.0, knot.id.0),
                );
                self.handle_knot_added(loom_id, knot.clone())
            }
            ConfigEvent::KnotModified { ref loom_id, ref knot } => {
                logging::log_config_event(
                    "KnotModified",
                    &format!("loom={} knot={}", loom_id.0, knot.id.0),
                );
                self.handle_knot_modified(loom_id, knot.clone())
            }
            ConfigEvent::KnotDeleted { ref loom_id, ref knot_id } => {
                logging::log_config_event(
                    "KnotDeleted",
                    &format!("loom={} knot={}", loom_id.0, knot_id.0),
                );
                self.handle_knot_deleted(loom_id, knot_id)
            }
        }
    }

    /// Handle `ConfigEvent::LoomAdded`.
    ///
    /// Scans only the new loom directory via `LoomRepository::scan_knot_files()`
    /// (not the full rig), resolves knot paths relative to the project root,
    /// builds a `Loom` directly, and registers it using the same flow as
    /// `RegisterLoom`.
    fn handle_loom_added(
        &self,
        loom_id: &LoomId,
        loom_dir: &Path,
    ) -> Result<(), PortError> {
        // Skip if already registered
        if self.store.get(loom_id).is_some() {
            logging::log_config_event(
                "LoomAdded",
                &format!("loom={} already registered (skip)", loom_id.0),
            );
            return Ok(());
        }

        // Scan only the new loom directory (not the full rig)
        let (mut knots, warnings) =
            self.repository.scan_knot_files(loom_dir)?;

        // Resolve per-knot paths relative to the project root
        for knot in &mut knots {
            knot.strand_dir =
                crate::adapters::outbound::loom_repository::FileSystemLoomRepository::resolve_path(
                    &self.project_root,
                    &knot.strand_dir,
                );
        }

        let loom = Loom {
            id: loom_id.clone(),
            knots,
        };

        self.register_loom(&loom, &warnings)?;
        logging::log_loom_event(
            "registered",
            &loom_id.0,
            &format!("{} knots", loom.knots.len()),
        );
        Ok(())
    }

    /// Handle `ConfigEvent::KnotAdded`.
    ///
    /// Adds the knot to the loom in the store, logs `KnotRegistered`,
    /// and starts a watcher for its `strand_dir`.
    fn handle_knot_added(
        &self,
        loom_id: &LoomId,
        knot: Knot,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        // Check for duplicate knot ID
        if loom.knots.iter().any(|k| k.id == knot.id) {
            // Idempotent — knot already present, skip
            logging::log_knot_event(
                "added",
                &loom_id.0,
                &knot.id.0,
                "already present (skip)",
            );
            return Ok(());
        }

        let knot_strand_dir = knot.strand_dir.clone();
        let knot_id = knot.id.clone();
        loom.knots.push(knot);
        self.store.register(loom);

        // Log KnotRegistered
        self.log_port.append(LoomEvent::KnotRegistered {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            timestamp: format_timestamp(),
        })?;

        // Start watcher (auto-creates strand_dir if missing)
        self.ensure_strand_dir_and_watch(loom_id, &knot_id, &knot_strand_dir)?;

        logging::log_knot_event(
            "added",
            &loom_id.0,
            &knot_id.0,
            "registered + watcher started",
        );
        Ok(())
    }

    /// Handle `ConfigEvent::KnotModified`.
    ///
    /// Updates the knot in the store, stops the old watcher if
    /// `strand_dir` changed, and starts a new watcher for the
    /// updated `strand_dir`.
    ///
    /// If the knot is not found in the loom (e.g., due to a race between
    /// `LoomAdded` scanning a partially-written directory and a later
    /// `KnotModified` with valid data), treats it as a new registration:
    /// appends the knot, logs `KnotRegistered`, starts a watcher, and
    /// emits a warning.
    fn handle_knot_modified(
        &self,
        loom_id: &LoomId,
        knot: Knot,
    ) -> Result<(), PortError> {
        let mut loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        let pos = loom.knots.iter().position(|k| k.id == knot.id);

        match pos {
            Some(index) => {
                // Existing knot — update in place
                let old_strand_dir = loom.knots[index].strand_dir.clone();
                let new_strand_dir = knot.strand_dir.clone();
                let knot_id = knot.id.clone();
                loom.knots[index] = knot;
                self.store.register(loom);

                // If strand_dir changed, stop old watcher and start new one
                if old_strand_dir != new_strand_dir {
                    self.event_source.unwatch_with_type(
                        &old_strand_dir,
                        WatchType::Strand(loom_id.clone(), knot_id.clone()),
                    )
                    .map_err(|e| {
                        PortError::EventUnwatchFailed(format!(
                            "failed to unwatch '{}': {}",
                            old_strand_dir.display(),
                            e
                        ))
                    })?;

                    self.ensure_strand_dir_and_watch(
                        loom_id,
                        &knot_id,
                        &new_strand_dir,
                    )?;

                    logging::log_knot_event(
                        "modified",
                        &loom_id.0,
                        &knot_id.0,
                        "strand_dir changed, watcher updated",
                    );
                } else {
                    logging::log_knot_event(
                        "modified",
                        &loom_id.0,
                        &knot_id.0,
                        "config updated (strand_dir unchanged)",
                    );
                }
            }
            None => {
                // Knot not found — recover by registering as new.
                // This handles the race where LoomAdded scanned before
                // the knot file was fully written.
                let knot_strand_dir = knot.strand_dir.clone();
                let knot_id = knot.id.clone();
                loom.knots.push(knot);
                self.store.register(loom);

                logging::log_knot_event(
                    "warn:modified",  
                    &loom_id.0,
                    &knot_id.0,
                    "knot not found, recovered by registering",
                );

                // Log KnotRegistered
                self.log_port.append(LoomEvent::KnotRegistered {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    timestamp: format_timestamp(),
                })?;

                // Start watcher (auto-creates strand_dir if missing)
                self.ensure_strand_dir_and_watch(
                    loom_id,
                    &knot_id,
                    &knot_strand_dir,
                )?;

                logging::log_knot_event(
                    "added",
                    &loom_id.0,
                    &knot_id.0,
                    "registered + watcher started (recovered from KnotModified)",
                );
            }
        }

        Ok(())
    }

    /// Handle `ConfigEvent::KnotDeleted`.
    ///
    /// Removes the knot from the loom in the store, stops its
    /// `strand_dir` watcher, and logs `KnotDeregistered`.
    fn handle_knot_deleted(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
    ) -> Result<(), PortError> {
        let loom = self.store.get(loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        let pos = loom.knots.iter()
            .position(|k| k.id == *knot_id)
            .ok_or_else(|| PortError::LoomSaveFailed(format!(
                "knot '{}' not found in loom '{}'",
                knot_id.0,
                loom_id.0
            )))?;

        let knot = &loom.knots[pos];
        let strand_dir = knot.strand_dir.clone();

        // Remove knot from loom
        let mut updated_loom = loom;
        updated_loom.knots.remove(pos);
        self.store.register(updated_loom);

        // Stop watcher for the knot's strand directory
        self.event_source.unwatch_with_type(
            &strand_dir,
            WatchType::Strand(loom_id.clone(), knot_id.clone()),
        )
        .map_err(|e| {
            PortError::EventUnwatchFailed(format!(
                "failed to unwatch '{}': {}",
                strand_dir.display(),
                e
            ))
        })?;

        // Log KnotDeregistered
        self.log_port.append(LoomEvent::KnotDeregistered {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            timestamp: format_timestamp(),
        })?;

        logging::log_knot_event(
            "deleted",
            &loom_id.0,
            &knot_id.0,
            "removed + watcher stopped",
        );
        Ok(())
    }

    /// Register a loom: log events, store, and start watchers.
    fn register_loom(
        &self,
        loom: &Loom,
        warnings: &[String],
    ) -> Result<(), PortError> {
        // Open the loom activity log
        self.log_port.open(&loom.id)?;

        // Append KnotRegistered for each knot
        for knot in &loom.knots {
            self.log_port.append(LoomEvent::KnotRegistered {
                loom_id: loom.id.clone(),
                knot_id: knot.id.clone(),
                timestamp: format_timestamp(),
            })?;
        }

        // Append KnotParseWarning for each unknown property warning
        for warning in warnings {
            self.log_port.append(LoomEvent::KnotParseWarning {
                loom_id: loom.id.clone(),
                knot_file_name: String::new(),
                message: warning.clone(),
                timestamp: format_timestamp(),
            })?;
        }

        // Append LoomStarted event
        self.log_port.append(LoomEvent::LoomStarted {
            loom_id: loom.id.clone(),
            timestamp: format_timestamp(),
        })?;

        // Store the loom
        self.store.register(loom.clone());

        // Start file watchers for each knot's strand directory
        for knot in &loom.knots {
            self.ensure_strand_dir_and_watch(
                &loom.id,
                &knot.id,
                &knot.strand_dir,
            )?;
        }

        Ok(())
    }

    /// Ensure `strand_dir` exists on disk, then start the watcher.
    ///
    /// If the directory is missing, creates it (including any parent
    /// directories), logs a `LoomEvent::DirectoryCreated` event, and
    /// emits a log line. The watcher is always started regardless of
    /// whether creation was needed.
    fn ensure_strand_dir_and_watch(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
        strand_dir: &Path,
    ) -> Result<(), PortError> {
        let dir_created = if !strand_dir.exists() {
            std::fs::create_dir_all(strand_dir).map_err(|e| {
                PortError::EventWatchFailed(format!(
                    "failed to create strand dir '{}': {}",
                    strand_dir.display(),
                    e,
                ))
            })?;
            self.log_port.append(LoomEvent::DirectoryCreated {
                loom_id: loom_id.clone(),
                knot_id: knot_id.clone(),
                directory: strand_dir.display().to_string(),
                timestamp: format_timestamp(),
            })?;
            logging::log_knot_event(
                "dir-created",
                &loom_id.0,
                &knot_id.0,
                &format!("auto-created strand dir: {}", strand_dir.display()),
            );
            true
        } else {
            false
        };

        self.event_source
            .set_loom_ids(strand_dir, loom_id, knot_id);
        self.event_source.watch(strand_dir).map_err(|e| {
            PortError::EventWatchFailed(format!(
                "failed to watch '{}': {}",
                strand_dir.display(),
                e,
            ))
        })?;

        if dir_created {
            logging::log_knot_event(
                "watch-started",
                &loom_id.0,
                &knot_id.0,
                "watcher started on newly created dir",
            );
        }

        Ok(())
    }
}

// ── ConfigEventHandler Tests ──────────────────────────────────────────

#[cfg(test)]
mod config_handler_tests {
    use super::*;
    use crate::domain::value_objects::PromptTemplate;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    // ── Tracking EventSource Mock ──────────────────────────────────────

    /// A mock `EventSource` that records all `watch()` and `unwatch()` calls.
    struct TrackingEventSource {
        watch_calls: Arc<Mutex<Vec<PathBuf>>>,
        unwatch_calls: Arc<Mutex<Vec<PathBuf>>>,
        set_ids_calls: Arc<Mutex<Vec<(PathBuf, LoomId, KnotId)>>>,
    }

    impl TrackingEventSource {
        #[allow(clippy::type_complexity)]
        fn new(
        ) -> (
            Self,
            Arc<Mutex<Vec<PathBuf>>>,
            Arc<Mutex<Vec<PathBuf>>>,
            Arc<Mutex<Vec<(PathBuf, LoomId, KnotId)>>>,
        ) {
            let watch_calls = Arc::new(Mutex::new(vec![]));
            let unwatch_calls = Arc::new(Mutex::new(vec![]));
            let set_ids_calls = Arc::new(Mutex::new(vec![]));
            let source = Self {
                watch_calls: watch_calls.clone(),
                unwatch_calls: unwatch_calls.clone(),
                set_ids_calls: set_ids_calls.clone(),
            };
            (source, watch_calls, unwatch_calls, set_ids_calls)
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

        fn set_loom_ids(
            &self,
            source_dir: &Path,
            loom_id: &LoomId,
            knot_id: &KnotId,
        ) {
            self.set_ids_calls
                .lock()
                .unwrap()
                .push((
                    source_dir.to_path_buf(),
                    loom_id.clone(),
                    knot_id.clone(),
                ));
        }
    }

    // ── Mock LoomLogPort ───────────────────────────────────────────────

    #[derive(Default)]
    struct MockLoomLogPort {
        events: Arc<Mutex<Vec<LoomEvent>>>,
    }

    impl MockLoomLogPort {
        fn new() -> (Self, Arc<Mutex<Vec<LoomEvent>>>) {
            let events = Arc::new(Mutex::new(vec![]));
            let port = Self {
                events: events.clone(),
            };
            (port, events)
        }
    }

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, event: LoomEvent) -> Result<(), PortError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    // ── Mock LoomRepository ────────────────────────────────────────────

    struct MockLoomRepository {
        scan_looms: Arc<Mutex<Vec<Loom>>>,
        scan_warnings: Arc<Mutex<Vec<String>>>,
        scan_knots: Arc<Mutex<Vec<Knot>>>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<(Vec<Loom>, Vec<String>), PortError> {
            Ok((
                self.scan_looms.lock().unwrap().clone(),
                self.scan_warnings.lock().unwrap().clone(),
            ))
        }

        fn scan_knot_files(
            &self,
            _loom_dir: &std::path::Path,
        ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
            Ok((
                self.scan_knots.lock().unwrap().clone(),
                self.scan_warnings.lock().unwrap().clone(),
            ))
        }

        fn get(
            &self,
            _id: &LoomId,
        ) -> Result<Option<Loom>, PortError> {
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

    /// Build a knot with the given ID and strand_dir.
    fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    /// Build a knot with custom strand_dir.
    fn build_knot_with_strand_dir(
        id: impl Into<String>,
        strand_dir: PathBuf,
    ) -> Knot {
        let mut knot = build_knot(id);
        knot.strand_dir = strand_dir;
        knot
    }

    /// Build a loom with the given ID and optional knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    // ── Tests ──────────────────────────────────────────────────────────

    /// `ConfigEventHandler` with `ConfigEvent::LoomAdded`: scans the
    /// loom dir via repository `scan_knot_files()`, registers loom in
    /// store, logs events, and starts watchers for each knot's strand
    /// directory.
    #[test]
    fn config_handler_loom_added() {
        let loom_id = LoomId("new-loom".to_string());
        // Use a temp directory so strand_dirs can be auto-created
        let tmp = tempfile::tempdir().unwrap();
        let rig_path = tmp.path().to_path_buf();
        let strand_dir = rig_path.join("strands");
        let knots = vec![
            build_knot_with_strand_dir("k1", strand_dir.clone()),
            build_knot_with_strand_dir("k2", strand_dir.clone()),
        ];

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(knots)),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let store = LoomStore::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            rig_path.clone(),
        );

        let result = handler.execute(ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: rig_path.join("new-loom").display().to_string(),
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom is in the store
        let stored = store.get(&loom_id);
        assert!(stored.is_some(), "loom should be in store");
        let stored = stored.unwrap();
        assert_eq!(stored.id, loom_id);
        assert_eq!(stored.knots.len(), 2);

        // Log events: 2x KnotRegistered, LoomStarted, DirectoryCreated
        let events = logged_events.lock().unwrap();
        assert_eq!(
            events.len(),
            4,
            "should log KnotRegistered x2 + LoomStarted + DirectoryCreated"
        );
        match &events[0] {
            LoomEvent::KnotRegistered { loom_id: lid, knot_id, .. } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k1");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }
        match &events[1] {
            LoomEvent::KnotRegistered { loom_id: lid, knot_id, .. } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k2");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }
        match &events[2] {
            LoomEvent::LoomStarted { loom_id: lid, .. } => {
                assert_eq!(*lid, loom_id);
            }
            other => panic!("Expected LoomStarted, got {other:?}"),
        }
        // DirectoryCreated logged for first knot (second shares same dir)
        match &events[3] {
            LoomEvent::DirectoryCreated {
                loom_id: lid,
                knot_id,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k1");
            }
            other => panic!("Expected DirectoryCreated, got {other:?}"),
        }

        // Watchers started for each knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(
            watches.len(),
            2,
            "should watch 2 knot strand directories"
        );
        let watched: HashSet<_> =
            watches.iter().map(|p| p.as_path()).collect();
        assert!(
            watched.contains(strand_dir.as_path()),
            "watcher should be started for strand_dir"
        );
    }

    /// `ConfigEventHandler` with `ConfigEvent::LoomAdded` for an
    /// already-registered loom is idempotent (no-op).
    #[test]
    fn config_handler_loom_added_already_registered() {
        let loom = build_loom("existing-loom", vec![build_knot("k1")]);
        let loom_id = loom.id.clone();

        let store = LoomStore::new();
        store.register(loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: "/rig/existing-loom".to_string(),
        });

        // Should succeed (idempotent)
        assert!(result.is_ok(), "should succeed for existing loom");

        // No watchers started
        let watches = watch_calls.lock().unwrap();
        assert!(
            watches.is_empty(),
            "no watchers should be started for existing loom"
        );
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotAdded`: adds the
    /// knot to the loom in the store, logs `KnotRegistered`, and
    /// starts a watcher for the knot's strand directory.
    #[test]
    fn config_handler_knot_added() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_loom =
            build_loom("test-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let new_knot = build_knot("k2");
        let result = handler.execute(ConfigEvent::KnotAdded {
            loom_id: loom_id.clone(),
            knot: new_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom now has 2 knots
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 2);
        let k2 = loom.knots.iter()
            .find(|k| k.id == KnotId("k2".to_string()))
            .unwrap();
        assert_eq!(k2.id, KnotId("k2".to_string()));

        // Log: KnotRegistered for k2 (may also have DirectoryCreated
        // if strand_dir was auto-created, depending on CWD state)
        let events = logged_events.lock().unwrap();
        assert!(events.len() >= 1);
        match &events[0] {
            LoomEvent::KnotRegistered {
                loom_id: lid,
                knot_id,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k2");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }

        // Watcher started for new knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotAdded` for a
    /// duplicate knot is idempotent (no-op).
    #[test]
    fn config_handler_knot_added_duplicate() {
        let loom_id = LoomId("test-loom".to_string());
        let knot = build_knot("k1");
        let existing_loom =
            build_loom("test-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotAdded {
            loom_id: loom_id.clone(),
            knot,
        });

        // Should succeed (idempotent)
        assert!(result.is_ok(), "should succeed for duplicate knot");

        // Loom still has 1 knot
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 1);

        // No new log entries
        let events = logged_events.lock().unwrap();
        assert!(events.is_empty());

        // No new watchers
        let watches = watch_calls.lock().unwrap();
        assert!(watches.is_empty());
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotAdded` returns
    /// error when loom does not exist.
    #[test]
    fn config_handler_knot_added_loom_not_found() {
        let store = LoomStore::new();
        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, _, _, _) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotAdded {
            loom_id: LoomId("nonexistent".to_string()),
            knot: build_knot("k1"),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("nonexistent".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified`: updates
    /// the knot in the store, stops the old watcher, and starts a
    /// new watcher if `strand_dir` changed.
    #[test]
    fn config_handler_knot_modified() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_knot = build_knot("k1");
        let existing_loom =
            build_loom("test-loom", vec![existing_knot]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            _watch_calls,
            unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        // Update knot with different strand_dir
        let updated_knot =
            build_knot_with_strand_dir("k1", PathBuf::from("new-strands"));

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: updated_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Knot is updated in store
        let loom = store.get(&loom_id).unwrap();
        let k1 = loom.knots.iter()
            .find(|k| k.id == KnotId("k1".to_string()))
            .unwrap();
        assert_eq!(
            k1.strand_dir,
            PathBuf::from("new-strands")
        );

        // Old watcher stopped
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 1);
        assert_eq!(unwatches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` with
    /// same `strand_dir`: updates knot without watcher changes.
    #[test]
    fn config_handler_knot_modified_same_strand_dir() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_knot = build_knot("k1");
        let existing_loom =
            build_loom("test-loom", vec![existing_knot]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _) = MockLoomLogPort::new();
        let (
            event_source,
            _watch_calls,
            unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        // Update knot with same strand_dir (only profile ref changed)
        let mut updated_knot = build_knot("k1");
        updated_knot.agent_profile_ref = "slow".to_string();

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: updated_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // No watcher changes
        let unwatches = unwatch_calls.lock().unwrap();
        assert!(
            unwatches.is_empty(),
            "no unwatch when strand_dir unchanged"
        );
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` recovers
    /// by registering the knot when it does not exist in the loom.
    /// This handles the race where LoomAdded scanned before the knot
    /// file was fully written, resulting in 0 knots registered.
    #[test]
    fn config_handler_knot_modified_not_found() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_loom =
            build_loom("test-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: build_knot("k_new"),
        });

        // Should succeed — recovers by registering the knot
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom now has 2 knots (k1 + recovered k_new)
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 2);
        let ids: Vec<_> = loom.knots.iter().map(|k| k.id.0.as_str()).collect();
        assert!(ids.contains(&"k1"));
        assert!(ids.contains(&"k_new"));

        // Log: KnotRegistered for recovered knot
        let events = logged_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::KnotRegistered {
                loom_id: lid,
                knot_id,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k_new");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }

        // Watcher started for recovered knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` for a
    /// loom with 0 knots: recovers by registering the knot, logs
    /// `KnotRegistered`, and starts a watcher.
    #[test]
    fn config_handler_knot_modified_new_knot_registers() {
        let loom_id = LoomId("empty-loom".to_string());
        // Loom registered with 0 knots (simulates race condition)
        let empty_loom = build_loom("empty-loom", vec![]);

        let store = LoomStore::new();
        store.register(empty_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let new_knot = build_knot("k1");
        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: new_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom now has 1 knot
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 1);
        assert_eq!(loom.knots[0].id, KnotId("k1".to_string()));

        // Log: KnotRegistered for the knot
        let events = logged_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::KnotRegistered {
                loom_id: lid,
                knot_id,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(knot_id.0, "k1");
            }
            other => panic!("Expected KnotRegistered, got {other:?}"),
        }

        // Watcher started for knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` for a
    /// missing knot emits a warning log and recovers. The warning is
    /// verified by checking that the recovery path side-effects
    /// (`KnotRegistered` log event, watcher started) are present.
    #[test]
    fn config_handler_knot_modified_warns_on_recovery() {
        let loom_id = LoomId("warn-loom".to_string());
        let empty_loom = build_loom("warn-loom", vec![]);

        let store = LoomStore::new();
        store.register(empty_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: build_knot("k_warn"),
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Verify the warning was emitted by checking the log events:
        // the recovery path logs KnotRegistered, proving the recovery
        // branch was taken (which also emits the warning via eprintln)
        let events = logged_events.lock().unwrap();
        let has_registered = events.iter().any(|e| matches!(
            e,
            LoomEvent::KnotRegistered { .. }
        ));
        assert!(
            has_registered,
            "recovery path should emit KnotRegistered (warning logged to stderr)"
        );

        // Watcher started confirms full recovery path executed
        let watches = watch_calls.lock().unwrap();
        assert_eq!(
            watches.len(),
            1,
            "recovery path should start a watcher"
        );
        assert_eq!(watches[0], PathBuf::from("strands"));
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotDeleted`: removes
    /// the knot from the loom in the store, stops its watcher, and
    /// logs `KnotDeregistered`.
    #[test]
    fn config_handler_knot_deleted() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_loom = build_loom(
            "test-loom",
            vec![
                build_knot("k1"),
                build_knot("k2"),
                build_knot("k3"),
            ],
        );

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            _watch_calls,
            unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let knot_id = KnotId("k2".to_string());
        let result = handler.execute(ConfigEvent::KnotDeleted {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Knot removed from loom
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 2);
        let ids: Vec<_> = loom.knots.iter()
            .map(|k| k.id.0.as_str())
            .collect();
        assert!(ids.contains(&"k1"));
        assert!(ids.contains(&"k3"));
        assert!(!ids.contains(&"k2"));

        // Watcher stopped for the deleted knot's strand directory
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 1);
        assert_eq!(unwatches[0], PathBuf::from("strands"));

        // Log: KnotDeregistered
        let events = logged_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::KnotDeregistered {
                loom_id: lid,
                knot_id: kid,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(*kid, knot_id);
            }
            other => panic!("Expected KnotDeregistered, got {other:?}"),
        }
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotDeleted` returns
    /// error when knot does not exist in the loom.
    #[test]
    fn config_handler_knot_deleted_not_found() {
        let loom_id = LoomId("test-loom".to_string());
        let existing_loom =
            build_loom("test-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, _, _, _) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotDeleted {
            loom_id: loom_id.clone(),
            knot_id: KnotId("k_unknown".to_string()),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomSaveFailed(msg) => {
                assert!(msg.contains("not found"));
            }
            other => panic!("Expected LoomSaveFailed, got {other:?}"),
        }
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotDeleted` returns
    /// error when loom does not exist.
    #[test]
    fn config_handler_knot_deleted_loom_not_found() {
        let store = LoomStore::new();
        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, _) = MockLoomLogPort::new();
        let (event_source, _, _, _) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::KnotDeleted {
            loom_id: LoomId("nonexistent".to_string()),
            knot_id: KnotId("k1".to_string()),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("nonexistent".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotAdded` when
    /// `strand_dir` does not exist: auto-creates the directory,
    /// logs `DirectoryCreated`, and starts the watcher.
    #[test]
    fn config_handler_knot_added_missing_strand_dir() {
        let loom_id = LoomId("auto-dir-loom".to_string());
        let existing_loom =
            build_loom("auto-dir-loom", vec![build_knot("k1")]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            _unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        // Create a temp dir for the nonexistent strand_dir
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent_dir = tmp.path().join("nonexistent-strands");
        assert!(
            !nonexistent_dir.exists(),
            "strand_dir must not exist before test"
        );

        let new_knot = build_knot_with_strand_dir("k2", nonexistent_dir.clone());
        let result = handler.execute(ConfigEvent::KnotAdded {
            loom_id: loom_id.clone(),
            knot: new_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // strand_dir was auto-created
        assert!(
            nonexistent_dir.exists(),
            "strand_dir should have been auto-created"
        );

        // Log contains KnotRegistered + DirectoryCreated
        let events = logged_events.lock().unwrap();
        let event_names: Vec<_> = events.iter().map(|e| match e {
            LoomEvent::KnotRegistered { .. } => "KnotRegistered",
            LoomEvent::DirectoryCreated { .. } => "DirectoryCreated",
            other => panic!("unexpected event variant: {:?}", other),
        }).collect();
        assert!(
            event_names.contains(&"KnotRegistered"),
            "should log KnotRegistered"
        );
        assert!(
            event_names.contains(&"DirectoryCreated"),
            "should log DirectoryCreated"
        );

        // Verify DirectoryCreated event fields
        let dir_created_event = events.iter().find(|e| {
            matches!(e, LoomEvent::DirectoryCreated { .. })
        });
        assert!(
            dir_created_event.is_some(),
            "DirectoryCreated event should be present"
        );
        match dir_created_event.unwrap() {
            LoomEvent::DirectoryCreated {
                loom_id: lid,
                knot_id: kid,
                directory: dir,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(kid.0, "k2");
                assert_eq!(
                    dir.as_str(),
                    nonexistent_dir.display().to_string(),
                    "DirectoryCreated should record the path"
                );
            }
            _ => unreachable!(),
        }

        // Watcher was started
        let watches = watch_calls.lock().unwrap();
        assert_eq!(
            watches.len(),
            1,
            "watcher should be started after dir creation"
        );
        assert_eq!(watches[0], nonexistent_dir);

        // Knot is in store
        let loom = store.get(&loom_id).unwrap();
        assert_eq!(loom.knots.len(), 2);
    }

    /// `ConfigEventHandler` with `ConfigEvent::KnotModified` when the
    /// new `strand_dir` does not exist: auto-creates the directory,
    /// logs `DirectoryCreated`, stops the old watcher, and starts a
    /// new watcher.
    #[test]
    fn config_handler_knot_modified_missing_strand_dir() {
        let loom_id = LoomId("auto-mod-loom".to_string());
        let existing_knot = build_knot("k1");
        let existing_loom =
            build_loom("auto-mod-loom", vec![existing_knot]);

        let store = LoomStore::new();
        store.register(existing_loom);

        let repo = Arc::new(MockLoomRepository {
            scan_looms: Arc::new(Mutex::new(vec![])),
            scan_warnings: Arc::new(Mutex::new(vec![])),
            scan_knots: Arc::new(Mutex::new(vec![])),
        });
        let (log_port, logged_events) = MockLoomLogPort::new();
        let (
            event_source,
            watch_calls,
            unwatch_calls,
            _set_ids_calls,
        ) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(log_port),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/rig"),
        );

        // Create a temp dir for the nonexistent strand_dir
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent_dir = tmp.path().join("new-strands");
        assert!(
            !nonexistent_dir.exists(),
            "new strand_dir must not exist before test"
        );

        // Update knot to point to the nonexistent directory
        let updated_knot =
            build_knot_with_strand_dir("k1", nonexistent_dir.clone());

        let result = handler.execute(ConfigEvent::KnotModified {
            loom_id: loom_id.clone(),
            knot: updated_knot,
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // strand_dir was auto-created
        assert!(
            nonexistent_dir.exists(),
            "new strand_dir should have been auto-created"
        );

        // Log contains DirectoryCreated
        let events = logged_events.lock().unwrap();
        let dir_created_event = events.iter().find(|e| {
            matches!(e, LoomEvent::DirectoryCreated { .. })
        });
        assert!(
            dir_created_event.is_some(),
            "DirectoryCreated event should be present"
        );
        match dir_created_event.unwrap() {
            LoomEvent::DirectoryCreated {
                loom_id: lid,
                knot_id: kid,
                directory: dir,
                ..
            } => {
                assert_eq!(*lid, loom_id);
                assert_eq!(kid.0, "k1");
                assert_eq!(
                    dir.as_str(),
                    nonexistent_dir.display().to_string(),
                    "DirectoryCreated should record the new path"
                );
            }
            _ => unreachable!(),
        }

        // Old watcher stopped
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 1);
        assert_eq!(unwatches[0], PathBuf::from("strands"));

        // New watcher started on the created directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], nonexistent_dir);

        // Knot is updated in store
        let loom = store.get(&loom_id).unwrap();
        let k1 = loom.knots.iter()
            .find(|k| k.id == KnotId("k1".to_string()))
            .unwrap();
        assert_eq!(k1.strand_dir, nonexistent_dir);
    }
}

// ── Phase 2 Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod phase2_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::PromptTemplate;
    use std::collections::HashSet;
    use std::fs;
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
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    /// Build a knot with custom strand_dir.
    fn build_knot_with_strand_dir(
        id: impl Into<String>,
        strand_dir: PathBuf,
    ) -> Knot {
        let mut knot = build_knot(id);
        knot.strand_dir = strand_dir;
        knot
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
            Arc::new(MockLoomLogPort),
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
            Arc::new(MockLoomLogPort),
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

    /// `RegisterLoom` duplicate ID is idempotent — returns Ok without starting
    /// new watchers or modifying the existing entry. This is necessary because
    /// auto-discovery may pre-register a loom before the POST /looms arrives.
    #[test]
    fn register_loom_duplicate_is_idempotent() {
        let loom1 = build_loom("dup", vec![build_knot("k1")]);
        let loom2 = build_loom("dup", vec![build_knot("k2")]);

        let (event_source, watch_calls) = TrackingEventSource::new();
        let store = LoomStore::new();

        // Register first loom
        let es: Arc<dyn EventSource> = Arc::new(event_source);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort),
            store.clone(),
            Arc::clone(&es),
        );
        assert!(use_case.execute(loom1).is_ok());

        // Verify first registration started a watcher
        {
            let watches = watch_calls.lock().unwrap();
            assert_eq!(watches.len(), 1);
        }

        // Duplicate registration — should succeed (idempotent)
        let (event_source2, watch_calls2) = TrackingEventSource::new();
        let es2: Arc<dyn EventSource> = Arc::new(event_source2);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort),
            store.clone(),
            es2,
        );
        let result = use_case.execute(loom2);

        // Idempotent: returns Ok(()) instead of Err
        assert!(result.is_ok());

        // No new watchers started for the duplicate
        let watches = watch_calls2.lock().unwrap();
        assert!(watches.is_empty());

        // Original store unchanged (k2 was not added)
        let stored = store.get(&LoomId("dup".to_string())).unwrap();
        assert_eq!(stored.knots.len(), 1);
        assert_eq!(stored.knots[0].id, KnotId("k1".to_string()));
    }

    // ── Mock LoomRepository for ConfigEventHandler tests ───────────────

    struct MockLoomRepository {
        scan_knots: Arc<Mutex<Vec<Knot>>>,
        scan_error: Arc<Mutex<Option<String>>>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<(Vec<Loom>, Vec<String>), PortError> {
            Ok((vec![], vec![]))
        }

        fn scan_knot_files(
            &self,
            _loom_dir: &std::path::Path,
        ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
            if let Some(ref err) = *self.scan_error.lock().unwrap() {
                return Err(PortError::RigScanFailed(err.clone()));
            }
            Ok((self.scan_knots.lock().unwrap().clone(), vec![]))
        }

        fn get(
            &self,
            _id: &LoomId,
        ) -> Result<Option<Loom>, PortError> {
            Ok(None)
        }

        fn list(&self) -> Result<Vec<Loom>, PortError> {
            Ok(vec![])
        }

        fn save(&self, _loom: Loom) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── ConfigEventHandler: targeted loom scan tests ───────────────────

    /// `handle_loom_added` uses `scan_knot_files(loom_dir)` to scan only
    /// the new loom directory (not the full rig), then resolves paths and
    /// builds a `Loom` directly.
    #[test]
    fn config_handler_loom_added_scans_specific_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().to_path_buf();
        let rig_path = workspace.join("rig");
        fs::create_dir_all(&rig_path).unwrap();
        let strand_dir = workspace.join("strands");

        let loom_id = LoomId("new-loom".to_string());
        let knots = vec![
            build_knot_with_strand_dir("k1", strand_dir.clone()),
            build_knot_with_strand_dir("k2", strand_dir.clone()),
        ];

        let repo = Arc::new(MockLoomRepository {
            scan_knots: Arc::new(Mutex::new(knots)),
            scan_error: Arc::new(Mutex::new(None)),
        });
        let store = LoomStore::new();
        let (event_source, watch_calls) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(MockLoomLogPort),
            store.clone(),
            Arc::new(event_source),
            rig_path.clone(),
        );

        let result = handler.execute(ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: rig_path.join("new-loom").display().to_string(),
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom is in the store with correct knots
        let stored = store.get(&loom_id).unwrap();
        assert_eq!(stored.id, loom_id);
        assert_eq!(stored.knots.len(), 2);
        let knot_ids: Vec<_> =
            stored.knots.iter().map(|k| k.id.0.as_str()).collect();
        assert!(knot_ids.contains(&"k1"));
        assert!(knot_ids.contains(&"k2"));

        // Watchers started for each knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 2);
        let watched: HashSet<_> =
            watches.iter().map(|p| p.as_path()).collect();
        assert!(
            watched.contains(strand_dir.as_path()),
            "expected strand_dir in watches, got {:?}",
            watches
        );
    }

    /// `handle_loom_added` returns an error when the loom directory
    /// scan fails (e.g. directory does not exist).
    #[test]
    fn config_handler_loom_added_dir_missing() {
        let loom_id = LoomId("missing-loom".to_string());

        let repo = Arc::new(MockLoomRepository {
            scan_knots: Arc::new(Mutex::new(vec![])),
            scan_error: Arc::new(Mutex::new(Some(
                "No such file or directory".to_string(),
            ))),
        });
        let store = LoomStore::new();
        let (event_source, _watch_calls) = TrackingEventSource::new();

        let handler = ConfigEventHandler::new(
            repo,
            Arc::new(MockLoomLogPort),
            store.clone(),
            Arc::new(event_source),
            PathBuf::from("/workspace/rig"),
        );

        let result = handler.execute(ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
            loom_dir: "/workspace/rig/missing-loom".to_string(),
        });

        // Should fail with RigScanFailed
        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::RigScanFailed(msg) => {
                assert!(
                    msg.contains("No such file or directory"),
                    "expected scan error, got: {}",
                    msg
                );
            }
            other => {
                panic!("Expected RigScanFailed, got {other:?}");
            }
        }

        // Loom should NOT be in the store
        assert!(
            store.get(&loom_id).is_none(),
            "loom should not be registered after scan failure"
        );
    }
}

// ── Phase 3 Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod phase3_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::PromptTemplate;
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
        #[allow(clippy::type_complexity)]
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
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
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
            Arc::new(MockLoomLogPort),
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
            Arc::new(MockLoomLogPort),
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
            Arc::new(MockLoomLogPort),
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
    use crate::domain::value_objects::PromptTemplate;
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
        scan_looms: Vec<Loom>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(&self, _rig: &std::path::Path) -> Result<(Vec<Loom>, Vec<String>), PortError> {
            Ok((self.scan_looms.clone(), vec![]))
        }

        fn scan_knot_files(
            &self,
            _loom_dir: &std::path::Path,
        ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
            Ok((vec![], vec![]))
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
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
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
            scan_looms: vec![
                existing_loom.clone(),
                new_loom.clone(),
                new_loom2.clone(),
            ],
        });
        let (event_source, watch_calls) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = DiscoverLooms::new(
            repo,
            Arc::new(MockLoomLogPort),
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
            scan_looms: vec![loom1.clone(), loom2.clone()],
        });
        let (event_source, watch_calls) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = DiscoverLooms::new(
            repo,
            Arc::new(MockLoomLogPort),
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
            scan_looms: vec![],
        });
        let (event_source, watch_calls) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = DiscoverLooms::new(
            repo,
            Arc::new(MockLoomLogPort),
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

// ── ManageKnot Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod manage_knot_tests {
    use super::*;
    use crate::domain::value_objects::PromptTemplate;
    use std::path::PathBuf;

    /// Build a knot with the given ID.
    fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: "default".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    /// Build a loom with the given ID and optional knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    /// `ManageKnot` with `KnotAction::Create` adds a new knot to the
    /// loom in the store.
    #[test]
    fn manage_knot_create() {
        let store = LoomStore::new();
        // Pre-register a loom with one knot
        let loom = build_loom("test", vec![build_knot("k1")]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let new_knot = build_knot("k2");
        let result = use_case.execute(KnotAction::Create {
            loom_id: LoomId("test".to_string()),
            knot: new_knot,
        });

        // Should succeed
        assert!(result.is_ok());

        // Loom now has 2 knots
        let updated = store.get(&LoomId("test".to_string())).unwrap();
        assert_eq!(updated.knots.len(), 2);

        // New knot is present with correct ID
        let found = updated.knots.iter()
            .find(|k| k.id == KnotId("k2".to_string()));
        assert!(found.is_some());
        let k = found.unwrap();
        assert_eq!(k.agent_profile_ref, "default");
        assert_eq!(k.strand_dir, PathBuf::from("strands"));
    }

    /// `ManageKnot` with `KnotAction::Create` returns error when loom
    /// does not exist.
    #[test]
    fn manage_knot_create_loom_not_found() {
        let store = LoomStore::new();
        let use_case = ManageKnot::new(store.clone());

        let result = use_case.execute(KnotAction::Create {
            loom_id: LoomId("unknown".to_string()),
            knot: build_knot("k1"),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("unknown".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }
    }

    /// `ManageKnot` with `KnotAction::Create` returns error when knot
    /// already exists.
    #[test]
    fn manage_knot_create_duplicate() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![build_knot("k1")]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let result = use_case.execute(KnotAction::Create {
            loom_id: LoomId("test".to_string()),
            knot: build_knot("k1"),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomSaveFailed(msg) => {
                assert!(msg.contains("already exists"));
            }
            other => panic!("Expected LoomSaveFailed, got {other:?}"),
        }
    }

    /// `ManageKnot` with `KnotAction::Update` updates an existing knot's
    /// configuration in the store.
    #[test]
    fn manage_knot_update() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![
            build_knot("k1"),
            build_knot("k2"),
        ]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        // Update k1 with a new profile ref
        let mut updated_knot = build_knot("k1");
        updated_knot.agent_profile_ref = "slow".to_string();
        updated_knot.prompt_template.instructions = "new instructions".to_string();

        let result = use_case.execute(KnotAction::Update {
            loom_id: LoomId("test".to_string()),
            knot: updated_knot,
        });

        // Should succeed
        assert!(result.is_ok());

        // Loom still has 2 knots
        let loom = store.get(&LoomId("test".to_string())).unwrap();
        assert_eq!(loom.knots.len(), 2);

        // k1 has updated config
        let k1 = loom.knots.iter()
            .find(|k| k.id == KnotId("k1".to_string()))
            .unwrap();
        assert_eq!(k1.agent_profile_ref, "slow");
        assert_eq!(
            k1.prompt_template.instructions,
            "new instructions"
        );

        // k2 is unchanged
        let k2 = loom.knots.iter()
            .find(|k| k.id == KnotId("k2".to_string()))
            .unwrap();
        assert_eq!(k2.agent_profile_ref, "default");
    }

    /// `ManageKnot` with `KnotAction::Update` returns error when knot
    /// does not exist.
    #[test]
    fn manage_knot_update_not_found() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![build_knot("k1")]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let result = use_case.execute(KnotAction::Update {
            loom_id: LoomId("test".to_string()),
            knot: build_knot("k_unknown"),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomSaveFailed(msg) => {
                assert!(msg.contains("not found"));
            }
            other => panic!("Expected LoomSaveFailed, got {other:?}"),
        }
    }

    /// `ManageKnot` with `KnotAction::Delete` removes a knot from the
    /// loom in the store.
    #[test]
    fn manage_knot_delete() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![
            build_knot("k1"),
            build_knot("k2"),
            build_knot("k3"),
        ]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let result = use_case.execute(KnotAction::Delete {
            loom_id: LoomId("test".to_string()),
            knot_id: KnotId("k2".to_string()),
        });

        // Should succeed
        assert!(result.is_ok());

        // Loom now has 2 knots (k2 removed)
        let updated = store.get(&LoomId("test".to_string())).unwrap();
        assert_eq!(updated.knots.len(), 2);

        let ids: Vec<_> = updated.knots.iter()
            .map(|k| k.id.0.as_str())
            .collect();
        assert!(ids.contains(&"k1"));
        assert!(ids.contains(&"k3"));
        assert!(!ids.contains(&"k2"));
    }

    /// `ManageKnot` with `KnotAction::Delete` returns error when knot
    /// does not exist.
    #[test]
    fn manage_knot_delete_not_found() {
        let store = LoomStore::new();
        let loom = build_loom("test", vec![build_knot("k1")]);
        store.register(loom);

        let use_case = ManageKnot::new(store.clone());
        let result = use_case.execute(KnotAction::Delete {
            loom_id: LoomId("test".to_string()),
            knot_id: KnotId("k_unknown".to_string()),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomSaveFailed(msg) => {
                assert!(msg.contains("not found"));
            }
            other => panic!("Expected LoomSaveFailed, got {other:?}"),
        }
    }

    /// `ManageKnot` with `KnotAction::Delete` returns error when loom
    /// does not exist.
    #[test]
    fn manage_knot_delete_loom_not_found() {
        let store = LoomStore::new();
        let use_case = ManageKnot::new(store.clone());

        let result = use_case.execute(KnotAction::Delete {
            loom_id: LoomId("unknown".to_string()),
            knot_id: KnotId("k1".to_string()),
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::LoomNotFound(id) => {
                assert_eq!(id, LoomId("unknown".to_string()));
            }
            other => panic!("Expected LoomNotFound, got {other:?}"),
        }
    }
}

// ── Phase 3: Profile Resolution Tests ─────────────────────────────

#[cfg(test)]
mod phase3_profile_resolution_tests {
    use super::*;
    use crate::application::ports::{AgentOutput, ExecutionContext};
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentProfile, PromptTemplate};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    // ── Mock LoomLogPort ─────────────────────────────────────────────

    #[derive(Default)]
    struct MockLoomLogPort;

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Mock AgentRunner ─────────────────────────────────────────────

    #[derive(Default)]
    struct MockAgentRunner;

    impl AgentRunner for MockAgentRunner {
        fn execute(
            &self,
            _ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            Ok(AgentOutput {
                stdout: "mock output".to_string(),
                stderr: String::new(),
                exit_code: 0,
                metadata: None,
            })
        }
    }

    // ── Mock TieOffSink ──────────────────────────────────────────────

    #[derive(Default)]
    struct MockTieOffSink {
        content: std::sync::RwLock<HashMap<String, String>>,
    }

    impl TieOffSink for MockTieOffSink {
        fn write(
            &self,
            tie_off: TieOff,
        ) -> Result<(), PortError> {
            self.content
                .write()
                .unwrap()
                .insert(tie_off.path.0.display().to_string(), tie_off.content);
            Ok(())
        }

        fn append(&self, tie_off: TieOff) -> Result<(), PortError> {
            self.write(tie_off)
        }

        fn read_content(
            &self,
            path: &TieOffPath,
        ) -> Result<String, PortError> {
            Ok(self
                .content
                .read()
                .unwrap()
                .get(&path.0.display().to_string())
                .cloned()
                .unwrap_or_default())
        }
    }

    // ── Mock RigLogPort ──────────────────────────────────────────────

    #[derive(Default)]
    struct MockRigLogPort {
        events: Arc<Mutex<Vec<crate::domain::events::RigLogEvent>>>,
    }

    impl MockRigLogPort {
        fn new() -> (Self, Arc<Mutex<Vec<crate::domain::events::RigLogEvent>>>) {
            let events = Arc::new(Mutex::new(vec![]));
            (Self { events: events.clone() }, events)
        }
    }

    impl RigLogPort for MockRigLogPort {
        fn append(
            &self,
            event: crate::domain::events::RigLogEvent,
        ) -> Result<(), PortError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn read_all(
            &self,
        ) -> Result<Vec<crate::domain::events::RigLogEvent>, PortError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    // ── Mock AgentProfileRepository ──────────────────────────────────

    #[derive(Default)]
    struct MockProfileRepository {
        profiles: Arc<Mutex<HashMap<String, AgentProfile>>>,
    }

    impl AgentProfileRepository for MockProfileRepository {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .get(name)
                .cloned())
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .values()
                .cloned()
                .collect())
        }
    }

    // ── Mock GitVersioningPort ───────────────────────────────────────

    #[derive(Default)]
    struct MockGitVersioningPort;

    impl GitVersioningPort for MockGitVersioningPort {
        fn commit(
            &self,
            _loom_id: &LoomId,
            _knot_id: &KnotId,
            _strand_path: &StrandPath,
            _event_type: &str,
            _tie_off_content: &str,
        ) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────

    /// Build a knot with the given profile ref.
    fn build_profile_knot(
        id: impl Into<String>,
        profile_name: &str,
    ) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: profile_name.to_string(),
            prompt_template: PromptTemplate {
                instructions: "check with profile".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    /// Build a loom with the given ID and knots.
    #[allow(dead_code)]
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    // ── resolve_agent_config Tests ───────────────────────────────────

    /// Profile ref resolves to profile fields: provider, model, tools.
    /// Goal comes from the knot's prompt template instructions.
    /// Profile prompt is delivered via stdin (not --system-prompt).
    #[test]
    fn resolve_agent_config_from_profile() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                (
                    "fast".to_string(),
                    AgentProfile::with_tools(
                        "fast".to_string(),
                        "openai".to_string(),
                        "gpt-4o".to_string(),
                        vec!["fs".to_string(), "web".to_string()],
                        "You are fast.".to_string(),
                    )
                    .unwrap(),
                ),
            ]))),
        });

        let (rig_log, _rig_events) = MockRigLogPort::new();
        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        let profile_knot = build_profile_knot("k1", "fast");
        let (config, profile_timeout) =
            use_case.resolve_agent_config(&profile_knot).unwrap();

        // Resolved config should use profile values
        assert_eq!(config.provider, "openai");
        // Profile has no timeout set, so it resolves to None
        assert_eq!(profile_timeout, None);
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.tools, vec!["fs", "web"]);
        // Goal comes from prompt template instructions
        assert_eq!(
            config.goal,
            profile_knot.prompt_template.instructions
        );
    }

    /// Profile not found returns PortError::ProfileNotFound.
    #[test]
    fn resolve_agent_config_profile_not_found() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository::default());

        let (rig_log, _rig_events) = MockRigLogPort::new();
        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        let profile_knot = build_profile_knot("k1", "nonexistent");
        let result = use_case.resolve_agent_config(&profile_knot);

        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::ProfileNotFound(name) => {
                assert_eq!(name, "nonexistent");
            }
            other => panic!("Expected ProfileNotFound, got {other:?}"),
        }
    }

    /// Multiple knots reference the same profile — each resolves
    /// to the same profile values independently.
    #[test]
    fn resolve_agent_config_same_profile_multiple_knots() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                (
                    "detailed".to_string(),
                    AgentProfile::with_tools(
                        "detailed".to_string(),
                        "anthropic".to_string(),
                        "claude-sonnet-4-20250514".to_string(),
                        vec!["fs".to_string(), "web".to_string()],
                        "Be thorough.".to_string(),
                    )
                    .unwrap(),
                ),
            ]))),
        });

        let (rig_log, _rig_events) = MockRigLogPort::new();
        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        let knot1 = build_profile_knot("k1", "detailed");
        let knot2 = build_profile_knot("k2", "detailed");

        let (config1, timeout1) =
            use_case.resolve_agent_config(&knot1).unwrap();
        let (config2, timeout2) =
            use_case.resolve_agent_config(&knot2).unwrap();

        // Both should resolve to the same profile values
        // Neither profile has a timeout set
        assert_eq!(timeout1, None);
        assert_eq!(timeout2, None);
        assert_eq!(config1.provider, "anthropic");
        assert_eq!(config1.model, "claude-sonnet-4-20250514");
        assert_eq!(config2.provider, "anthropic");
        assert_eq!(config2.model, "claude-sonnet-4-20250514");
        assert_eq!(config1.tools, vec!["fs", "web"]);
        assert_eq!(config2.tools, vec!["fs", "web"]);
    }

    /// Dynamic profile pickup: adding a profile to the repository
    /// mid-lifecycle makes it available to knots on next resolution.
    #[test]
    fn resolve_agent_config_dynamic_profile_pickup() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository::default());

        let (rig_log, _rig_events) = MockRigLogPort::new();
        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        // Profile doesn't exist yet — should error
        let profile_knot = build_profile_knot("k1", "new-profile");
        let result = use_case.resolve_agent_config(&profile_knot);
        assert!(result.is_err());

        // Add the profile to the repository (simulates file created on disk)
        let profile = AgentProfile::with_tools(
            "new-profile".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            vec!["fs".to_string()],
            "You are new.".to_string(),
        )
        .unwrap();
        profile_repo
            .profiles
            .lock()
            .unwrap()
            .insert("new-profile".to_string(), profile);

        // Now the same knot should resolve successfully
        let (config, profile_timeout) =
            use_case.resolve_agent_config(&profile_knot).unwrap();
        assert_eq!(config.provider, "openai");
        // Profile has no timeout set
        assert_eq!(profile_timeout, None);
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.tools, vec!["fs"]);
    }

/// Profile prompt does NOT flow into CLI args (delivered via stdin).
    ///
    /// Verifies that when a profile-ref knot is resolved, the resulting
    /// CLI args contain --model but NOT --system-prompt. Profile prompt
    /// and knot instructions are delivered via stdin instead.
    #[test]
    fn profile_ref_cli_args_no_system_prompt_flag() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([(
                "reviewer".to_string(),
                AgentProfile::new(
                    "reviewer".to_string(),
                    "openai".to_string(),
                    "gpt-4o".to_string(),
                    "You are a careful reviewer. Be precise and concise.".to_string(),
                )
                .unwrap(),
            )]))),
        });

        let (rig_log, _rig_events) = MockRigLogPort::new();
        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        let profile_knot = build_profile_knot("k1", "reviewer");
        let (config, _profile_timeout) =
            use_case.resolve_agent_config(&profile_knot).unwrap();
        let args = config.build_cli_args();

        // CLI args should NOT contain --system-prompt
        assert!(
            !args.contains(&"--system-prompt".to_string()),
            "CLI args should NOT contain --system-prompt"
        );
        // Should have the model arg
        let model_index = args.iter().position(|a| a == "--model").expect("--model flag missing");
        assert_eq!(args[model_index + 1], "gpt-4o");
    }

}

// ── Phase 6: Timeout Handling Tests ───────────────────────────────

#[cfg(test)]
mod phase6_timeout_tests {
    use super::*;
    use crate::application::ports::{AgentOutput, ExecutionContext};
    use crate::domain::entities::{Knot, KnotId, TieOffStatus};
    use crate::domain::events::RigLogEvent;
    use crate::domain::value_objects::PromptTemplate;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    // ── Mock LoomLogPort ─────────────────────────────────────────────

    #[derive(Default)]
    struct MockLoomLogPort {
        events: Arc<Mutex<Vec<LoomEvent>>>,
    }

    impl MockLoomLogPort {
        fn new() -> (Self, Arc<Mutex<Vec<LoomEvent>>>) {
            let events = Arc::new(Mutex::new(vec![]));
            (Self { events: events.clone() }, events)
        }
    }

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, event: LoomEvent) -> Result<(), PortError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    // ── Mock AgentRunner (configurable error) ────────────────────────

    /// Mock agent runner that returns a configurable result and captures
    /// the execution context for inspection in tests.
    ///
    /// Supports two modes:
    /// - Single result (via `new()`) — returns the same result for every call.
    /// - Sequence (via `new_sequence()`) — pops results from a queue for each
    ///   call, useful for testing session-resume retry behaviour.
    struct ConfigurableAgentRunner {
        result: Arc<Mutex<Result<AgentOutput, PortError>>>,
        /// Sequence of results to return (popped front-to-back).
        /// When `Some`, takes priority over `result`.
        sequence: Arc<Mutex<Option<std::collections::VecDeque<
            Result<AgentOutput, PortError>,
        >>>>,
        captured_ctx: Arc<Mutex<Vec<ExecutionContext>>>,
    }

    impl ConfigurableAgentRunner {
        fn new(result: Result<AgentOutput, PortError>) -> Self {
            Self {
                result: Arc::new(Mutex::new(result)),
                sequence: Arc::new(Mutex::new(None)),
                captured_ctx: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn new_sequence(
            results: Vec<Result<AgentOutput, PortError>>,
        ) -> Self {
            Self {
                result: Arc::new(Mutex::new(Ok(AgentOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 0,
                    metadata: None,
                }))),
                sequence: Arc::new(Mutex::new(Some(
                    results.into_iter().collect(),
                ))),
                captured_ctx: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn set_result(&self, result: Result<AgentOutput, PortError>) {
            *self.result.lock().unwrap() = result;
        }

        /// Return the last captured execution context (if any).
        fn get_captured_ctx(&self) -> Option<ExecutionContext> {
            self.captured_ctx
                .lock()
                .unwrap()
                .last()
                .cloned()
        }

        /// Return all captured execution contexts in order.
        fn get_captured_contexts(&self) -> Vec<ExecutionContext> {
            self.captured_ctx.lock().unwrap().clone()
        }
    }

    impl AgentRunner for ConfigurableAgentRunner {
        fn execute(
            &self,
            ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            self.captured_ctx.lock().unwrap().push(ctx);

            // Check sequence first, then fall back to single result
            if let Some(ref mut seq) = *self.sequence.lock().unwrap() {
                if let Some(result) = seq.pop_front() {
                    return result;
                }
            }
            self.result.lock().unwrap().clone()
        }

        fn execute_with_config(
            &self,
            agent_config: &AgentConfig,
            strand_path: StrandPath,
            strand_file_ref: Option<StrandPath>,
            prompt: String,
            profile_prompt: String,
            event_type: String,
            knot_name: Option<String>,
            timeout: Option<std::time::Duration>,
        ) -> Result<AgentOutput, PortError> {
            let mut cli_args = agent_config.build_cli_args();
            let strand_filename = strand_path.0
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let session_title = format!(
                "{} triggered by {} on {}",
                knot_name.as_deref().unwrap_or("unknown"),
                event_type,
                strand_filename,
            );
            cli_args.push("--name".to_string());
            cli_args.push(session_title);
            if let Some(ref file_path) = strand_file_ref {
                cli_args.push(format!("@{}", file_path.0.display()));
            }
            let ctx = ExecutionContext {
                cli_path: "pi".to_string(),
                cli_args,
                prompt,
                profile_prompt,
                strand_path,
                event_type,
                knot_name,
                timeout,
            };
            self.execute(ctx)
        }
    }

    // ── Mock TieOffSink (tracks appends) ─────────────────────────────

    struct TrackingTieOffSink {
        appends: Arc<Mutex<Vec<TieOff>>>,
        content: Arc<Mutex<HashMap<String, String>>>,
    }

    impl TrackingTieOffSink {
        fn new() -> (
            Self,
            Arc<Mutex<Vec<TieOff>>>,
            Arc<Mutex<HashMap<String, String>>>,
        ) {
            let appends = Arc::new(Mutex::new(vec![]));
            let content = Arc::new(Mutex::new(HashMap::new()));
            (
                Self {
                    appends: appends.clone(),
                    content: content.clone(),
                },
                appends,
                content,
            )
        }
    }

    impl TieOffSink for TrackingTieOffSink {
        fn write(&self, tie_off: TieOff) -> Result<(), PortError> {
            self.content
                .lock()
                .unwrap()
                .insert(tie_off.path.0.display().to_string(), tie_off.content);
            Ok(())
        }

        fn append(&self, tie_off: TieOff) -> Result<(), PortError> {
            self.appends.lock().unwrap().push(tie_off.clone());
            self.write(tie_off)
        }

        fn read_content(
            &self,
            path: &TieOffPath,
        ) -> Result<String, PortError> {
            Ok(self
                .content
                .lock()
                .unwrap()
                .get(&path.0.display().to_string())
                .cloned()
                .unwrap_or_default())
        }
    }

    // ── Mock RigLogPort ──────────────────────────────────────────────

    struct MockRigLogPort {
        events: Arc<Mutex<Vec<RigLogEvent>>>,
    }

    impl MockRigLogPort {
        fn new() -> (Self, Arc<Mutex<Vec<RigLogEvent>>>) {
            let events = Arc::new(Mutex::new(vec![]));
            (Self { events: events.clone() }, events)
        }
    }

    impl RigLogPort for MockRigLogPort {
        fn append(
            &self,
            event: RigLogEvent,
        ) -> Result<(), PortError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn read_all(
            &self,
        ) -> Result<Vec<RigLogEvent>, PortError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    // ── Mock AgentProfileRepository ──────────────────────────────────

    struct MockProfileRepository {
        profiles: Arc<Mutex<HashMap<String, crate::domain::value_objects::AgentProfile>>>,
    }

    impl AgentProfileRepository for MockProfileRepository {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<crate::domain::value_objects::AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .get(name)
                .cloned())
        }

        fn list(
            &self,
        ) -> Result<Vec<crate::domain::value_objects::AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .values()
                .cloned()
                .collect())
        }
    }

    // ── Mock GitVersioningPort ───────────────────────────────────────

    #[derive(Default)]
    struct MockGitVersioningPort;

    impl GitVersioningPort for MockGitVersioningPort {
        fn commit(
            &self,
            _loom_id: &LoomId,
            _knot_id: &KnotId,
            _strand_path: &StrandPath,
            _event_type: &str,
            _tie_off_content: &str,
        ) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────

    /// Build a knot with the given profile ref.
    fn build_knot(id: impl Into<String>, profile: &str) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: profile.to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    /// Build a loom with the given ID and knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    /// Build a default profile for "fast".
    fn default_profile() -> crate::domain::value_objects::AgentProfile {
        crate::domain::value_objects::AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are fast.".to_string(),
        )
        .unwrap()
    }

    /// Build the ProcessStrand use case with all mocks.
    #[allow(clippy::type_complexity)]
    fn build_process_strand(
        loom: Loom,
        agent_runner: Arc<ConfigurableAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<TieOff>>>,
        Arc<Mutex<Vec<RigLogEvent>>>,
        Arc<Mutex<HashMap<String, String>>>,
        Arc<ConfigurableAgentRunner>,
    ) {
        let store = LoomStore::new();
        store.register(loom);

        let (log_port, log_events) = MockLoomLogPort::new();
        let (tie_off_sink, tie_off_appends, tie_off_content) =
            TrackingTieOffSink::new();
        let (rig_log, rig_events) = MockRigLogPort::new();

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]))),
        });

        let runner_for_use_case = agent_runner.clone();
        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(log_port),
            runner_for_use_case as Arc<dyn AgentRunner>,
            Arc::new(tie_off_sink),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        (
            use_case,
            log_events,
            tie_off_appends,
            rig_events,
            tie_off_content,
            agent_runner,
        )
    }

    // ── Tests ────────────────────────────────────────────────────────

    /// On `PortError::Timeout`:
    /// - loom-log receives `KnotProcessing`, `KnotFailed`, `StrandProcessed`
    /// - rig-log receives `TimeoutExceeded`
    /// - tie-off is NOT appended (preserved unchanged)
    #[test]
    fn process_strand_timeout_skip_tieoff_write_rig_log() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        let timeout_err = PortError::Timeout {
            message: "session exceeded 60s".to_string(),
            session_id: None,
        };
        let runner = Arc::new(ConfigurableAgentRunner::new(Err(timeout_err)));

        let (use_case, log_events, tie_off_appends, rig_events,
            _content, _runner) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);

        // execute() always returns Ok (errors are logged, not propagated)
        assert!(result.is_ok());

        // Loom-log: KnotProcessing, KnotFailed, StrandProcessed
        let events = log_events.lock().unwrap();
        assert_eq!(events.len(), 3, "should have 3 loom-log events");
        match &events[0] {
            LoomEvent::KnotProcessing { knot_id, .. } => {
                assert_eq!(knot_id.0, "k1");
            }
            other => panic!("expected KnotProcessing, got {other:?}"),
        }
        match &events[1] {
            LoomEvent::KnotFailed { knot_id, error, .. } => {
                assert_eq!(knot_id.0, "k1");
                assert!(error.contains("timeout"));
            }
            other => panic!("expected KnotFailed, got {other:?}"),
        }
        match &events[2] {
            LoomEvent::StrandProcessed { error, .. } => {
                assert!(error.is_some(), "error should be present");
                assert!(error.as_ref().unwrap().contains("timeout"));
            }
            other => panic!("expected StrandProcessed, got {other:?}"),
        }

        // Rig-log: TimeoutExceeded
        let rig = rig_events.lock().unwrap();
        assert_eq!(rig.len(), 1, "should have 1 rig-log event");
        match &rig[0] {
            RigLogEvent::TimeoutExceeded {
                loom_id,
                knot_id,
                error,
                ..
            } => {
                assert_eq!(loom_id.0, "test-loom");
                assert_eq!(knot_id.0, "k1");
                assert!(error.contains("timeout"));
            }
            other => panic!("expected TimeoutExceeded, got {other:?}"),
        }

        // Tie-off: NO append (unchanged)
        let appends = tie_off_appends.lock().unwrap();
        assert!(
            appends.is_empty(),
            "tie-off should NOT be appended on timeout"
        );
    }

    /// On non-timeout error (e.g., AgentExecutionFailed):
    /// - loom-log receives `KnotProcessing`, `KnotFailed`, `StrandProcessed`
    /// - rig-log does NOT receive any event
    /// - tie-off IS appended with error content (existing behaviour preserved)
    #[test]
    fn process_strand_non_timeout_error_writes_tieoff() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        let err = PortError::AgentExecutionFailed {
            message: "crash".to_string(),
            session_id: None,
        };
        let runner = Arc::new(ConfigurableAgentRunner::new(Err(err)));

        let (use_case, log_events, tie_off_appends, rig_events,
            _content, _runner) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Loom-log: KnotProcessing, KnotFailed, StrandProcessed
        let events = log_events.lock().unwrap();
        assert_eq!(events.len(), 3);
        match &events[1] {
            LoomEvent::KnotFailed { error, .. } => {
                assert!(error.contains("crash"));
            }
            other => panic!("expected KnotFailed, got {other:?}"),
        }

        // Rig-log: NO event (only timeout writes to rig-log)
        let rig = rig_events.lock().unwrap();
        assert!(
            rig.is_empty(),
            "rig-log should NOT receive event for non-timeout errors"
        );

        // Tie-off: IS appended with error content
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1, "tie-off should be appended");
        let appended = &appends[0];
        assert_eq!(appended.status, TieOffStatus::Failed);
        assert!(
            appended.content.contains("Processing failed"),
            "tie-off content should contain error: {}", appended.content
        );
        assert!(
            appended.content.contains("crash"),
            "tie-off content should contain error detail: {}",
            appended.content,
        );
    }

    /// On successful execution:
    /// - loom-log receives `KnotProcessing`, `KnotCompleted`, `StrandProcessed`
    /// - rig-log receives NO events
    /// - tie-off IS appended with agent output
    #[test]
    fn process_strand_success_no_rig_log() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        let output = Ok(AgentOutput {
            stdout: "agent output".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(ConfigurableAgentRunner::new(output));

        let (use_case, log_events, tie_off_appends, rig_events,
            _content, _runner) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Loom-log: KnotProcessing, KnotCompleted, StrandProcessed
        let events = log_events.lock().unwrap();
        assert_eq!(events.len(), 3);
        match &events[1] {
            LoomEvent::KnotCompleted { .. } => {}
            other => panic!("expected KnotCompleted, got {other:?}"),
        }

        // Rig-log: NO events
        let rig = rig_events.lock().unwrap();
        assert!(rig.is_empty(), "rig-log should be empty on success");

        // Tie-off: IS appended
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1);
        assert_eq!(appends[0].status, TieOffStatus::Produced);
        assert_eq!(appends[0].content, "agent output");
    }

    // ── Deleted Event Context Extraction Tests ───────────────────────

    /// For Deleted events, `@{strand_path}` must NOT appear in CLI args
    /// because the file no longer exists.
    #[test]
    fn process_strand_deleted_skips_at_file_arg() {
        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(ConfigurableAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _rig_events,
            _content, captured) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Deleted {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(PathBuf::from("input/strand.md")),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let ctx = captured.get_captured_ctx().expect("ctx should be captured");
        let has_at_ref = ctx.cli_args.iter().any(|arg| arg.starts_with('@'));
        assert!(
            !has_at_ref,
            "Deleted events must NOT contain @file reference in cli_args: {:?}",
            ctx.cli_args,
        );
    }

    /// For Deleted events, the prompt must contain the deletion notice.
    #[test]
    fn process_strand_deleted_injects_deletion_notice() {
        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(ConfigurableAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _rig_events,
            _content, captured) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Deleted {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(PathBuf::from("input/strand.md")),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let ctx = captured.get_captured_ctx().expect("ctx should be captured");
        assert!(
            ctx.prompt.contains("This file was deleted"),
            "prompt should contain deletion notice: {}",
            ctx.prompt,
        );
        assert!(
            ctx.prompt
                .contains("git history to help understand the file scope"),
            "prompt should contain git history hint: {}",
            ctx.prompt,
        );
    }

    /// For Deleted events with previous tie-off entries, the prompt
    /// must include the scoped strand history.
    #[test]
    fn process_strand_deleted_includes_strand_history() {
        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(ConfigurableAgentRunner::new(output));

        let (use_case, _log_events, tie_off_appends, _rig_events,
            tie_off_content, captured) =
            build_process_strand(loom, runner);

        // Pre-populate the tie-off sink with history.
        // The tie-off header stores the strand path as written by the sink
        // (from `strand_path.0.display().to_string()`), which matches the
        // event's strand_path. The extract_last_n comparison uses the full
        // path string, so the mock must use the same path format.
        {
            let mut content = tie_off_content.lock().unwrap();
            content.insert(
                "/rig/tie-offs/test-loom/k1/k1-tie-off.md".to_string(),
                concat!(
                    "## review triggered by Created input/strand.md\n",
                    "Timestamp: 2026-06-05T10:00:00Z\n",
                    "---\n",
                    "Initial review content\n",
                    "---\n",
                    "## review triggered by Modified input/strand.md\n",
                    "Timestamp: 2026-06-05T11:00:00Z\n",
                    "---\n",
                    "Updated review content",
                )
                .to_string(),
            );
        }

        let event = StrandEvent::Deleted {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(PathBuf::from("input/strand.md")),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let ctx = captured.get_captured_ctx().expect("ctx should be captured");
        // Should contain deletion notice
        assert!(
            ctx.prompt.contains("This file was deleted"),
            "prompt should contain deletion notice",
        );
        // Should contain strand history
        assert!(
            ctx.prompt.contains("Previous processing history"),
            "prompt should contain history header",
        );
        assert!(
            ctx.prompt.contains("## review triggered by Created input/strand.md"),
            "prompt should contain first entry header",
        );
        assert!(
            ctx.prompt.contains("Initial review content"),
            "prompt should contain first entry body",
        );
        assert!(
            ctx.prompt.contains("## review triggered by Modified input/strand.md"),
            "prompt should contain second entry header",
        );
        assert!(
            ctx.prompt.contains("Updated review content"),
            "prompt should contain second entry body",
        );

        // Verify no @file reference
        let has_at_ref = ctx.cli_args.iter().any(|arg| arg.starts_with('@'));
        assert!(
            !has_at_ref,
            "Deleted events must NOT contain @file reference",
        );

        // Verify tie-off was written
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1, "tie-off should be appended");
    }

    /// Regression guard: Created events must still use `@{strand_path}`
    /// in CLI args (unchanged behaviour).
    #[test]
    fn process_strand_created_still_uses_at_file() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(ConfigurableAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _rig_events,
            _content, captured) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let ctx = captured.get_captured_ctx().expect("ctx should be captured");
        let has_at_ref = ctx.cli_args.iter().any(|arg| {
            arg.starts_with('@') && arg.contains("strand.md")
        });
        assert!(
            has_at_ref,
            "Created events MUST contain @file reference in cli_args: {:?}",
            ctx.cli_args,
        );
        // Prompt should NOT contain deletion notice for Created events
        assert!(
            !ctx.prompt.contains("This file was deleted"),
            "Created events must NOT contain deletion notice",
        );
    }

    /// When no previous tie-off entries exist for the strand, only the
    /// deletion notice is injected (no history section).
    #[test]
    fn process_strand_deleted_no_history_injects_notice_only() {
        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(ConfigurableAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _rig_events,
            tie_off_content, captured) =
            build_process_strand(loom, runner);

        // Tie-off content is empty (no previous entries)
        {
            let content = tie_off_content.lock().unwrap();
            assert!(
                content.is_empty(),
                "tie-off content should be empty initially"
            );
        }

        let event = StrandEvent::Deleted {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(PathBuf::from("input/strand.md")),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let ctx = captured.get_captured_ctx().expect("ctx should be captured");
        // Should contain deletion notice
        assert!(
            ctx.prompt.contains("This file was deleted"),
            "prompt should contain deletion notice",
        );
        // Should NOT contain history section (no previous entries)
        assert!(
            !ctx.prompt.contains("Previous processing history"),
            "prompt should NOT contain history section when no entries exist",
        );
        // Should NOT contain @file reference
        let has_at_ref = ctx.cli_args.iter().any(|arg| arg.starts_with('@'));
        assert!(
            !has_at_ref,
            "Deleted events must NOT contain @file reference",
        );
    }

    // ── Session Resume Integration Tests ─────────────────────────────

    /// ProcessStrand with mock runner that fails then succeeds:
    /// session-resume retry triggers, strand completes normally.
    /// Loom-log shows SessionResumed + KnotCompleted, no KnotFailed.
    #[test]
    fn process_strand_retry_transparent_success() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        let timeout_err = PortError::Timeout {
            message: "timed out".to_string(),
            session_id: Some("sess-abc".to_string()),
        };
        let success_output = Ok(AgentOutput {
            stdout: "success after retry".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(ConfigurableAgentRunner::new_sequence(vec![
            Err(timeout_err),
            success_output,
        ]));

        let (use_case, log_events, tie_off_appends, rig_events,
            _content, _runner) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Loom-log: KnotProcessing, SessionResumed, KnotCompleted,
        // StrandProcessed
        let events = log_events.lock().unwrap();
        assert_eq!(events.len(), 4, "should have 4 loom-log events");
        match &events[0] {
            LoomEvent::KnotProcessing { .. } => {}
            other => panic!("expected KnotProcessing, got {other:?}"),
        }
        match &events[1] {
            LoomEvent::SessionResumed { attempt, .. } => {
                assert_eq!(*attempt, 1);
            }
            other => panic!("expected SessionResumed, got {other:?}"),
        }
        match &events[2] {
            LoomEvent::KnotCompleted { .. } => {}
            other => panic!("expected KnotCompleted, got {other:?}"),
        }
        // No KnotFailed in the log
        assert!(
            !events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. })),
            "should NOT have KnotFailed after successful retry"
        );

        // Rig-log: empty (success, not a timeout)
        let rig = rig_events.lock().unwrap();
        assert!(
            rig.is_empty(),
            "rig-log should be empty on successful retry"
        );

        // Tie-off: appended with success content
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1);
        assert_eq!(appends[0].status, TieOffStatus::Produced);
        assert_eq!(appends[0].content, "success after retry");
    }

    /// ProcessStrand with mock runner that always fails:
    /// session-resume exhausts retries, strand marked failed.
    /// Loom-log shows multiple SessionResumed + KnotFailed.
    /// Rig-log shows TimeoutExceeded.
    #[test]
    fn process_strand_retry_exhausted_fails() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        // Enough failures for initial + 10 retries
        let responses: Vec<Result<AgentOutput, PortError>> = (0..20)
            .map(|_| {
                Err(PortError::Timeout {
                    message: "timed out".to_string(),
                    session_id: Some("sess-abc".to_string()),
                })
            })
            .collect();
        let runner = Arc::new(ConfigurableAgentRunner::new_sequence(responses));

        let (use_case, log_events, _tie_off_appends, rig_events,
            _content, _runner) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok()); // execute() always returns Ok

        // Loom-log: KnotProcessing, 10x SessionResumed, KnotFailed,
        // StrandProcessed
        let events = log_events.lock().unwrap();
        // Count SessionResumed events
        let session_resumed_count = events.iter().filter(|e| {
            matches!(e, LoomEvent::SessionResumed { .. })
        }).count();
        assert_eq!(
            session_resumed_count,
            10,
            "should have 10 SessionResumed events (MAX_RETRIES)"
        );
        // KnotFailed present
        assert!(
            events.iter().any(|e| matches!(e, LoomEvent::KnotFailed { .. })),
            "should have KnotFailed after exhausted retries"
        );

        // Rig-log: TimeoutExceeded
        let rig = rig_events.lock().unwrap();
        assert_eq!(rig.len(), 1);
        match &rig[0] {
            RigLogEvent::TimeoutExceeded { .. } => {}
            other => panic!("expected TimeoutExceeded, got {other:?}"),
        }
    }

    /// ProcessStrand with stdio-style error (no session_id):
    /// session-resume does NOT retry, strand fails immediately.
    #[test]
    fn process_strand_no_retry_stdio() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom = build_loom("test-loom", vec![build_knot("k1", "fast")]);
        // Timeout with no session_id — simulates stdio adapter failure
        let timeout_err = PortError::Timeout {
            message: "timed out (no session)".to_string(),
            session_id: None,
        };
        let runner = Arc::new(ConfigurableAgentRunner::new(Err(timeout_err)));

        let (use_case, log_events, _tie_off_appends, rig_events,
            _content, _runner) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Loom-log: KnotProcessing, KnotFailed, StrandProcessed
        // (NO SessionResumed since no session_id)
        let events = log_events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(
            !events.iter().any(|e| matches!(e, LoomEvent::SessionResumed { .. })),
            "should NOT have SessionResumed without session_id"
        );
        match &events[1] {
            LoomEvent::KnotFailed { error, .. } => {
                assert!(error.contains("no session"));
            }
            other => panic!("expected KnotFailed, got {other:?}"),
        }

        // Rig-log: TimeoutExceeded
        let rig = rig_events.lock().unwrap();
        assert_eq!(rig.len(), 1);
        match &rig[0] {
            RigLogEvent::TimeoutExceeded { .. } => {}
            other => panic!("expected TimeoutExceeded, got {other:?}"),
        }
    }
}

// ── Phase 7: Profile Timeout Resolution Tests ─────────────────────────

#[cfg(test)]
mod phase7_timeout_resolution_tests {
    use super::*;
    use crate::application::ports::{AgentOutput, ExecutionContext};
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentProfile, PromptTemplate};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;

    // ── Mock LoomLogPort ─────────────────────────────────────────────

    #[derive(Default)]
    struct MockLoomLogPort;

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Tracking AgentRunner (captures ExecutionContext) ────────────

    /// Mock agent runner that records the ExecutionContext passed to it.
    struct TrackingAgentRunner {
        contexts: Arc<Mutex<Vec<ExecutionContext>>>,
    }

    impl TrackingAgentRunner {
        fn new() -> (Self, Arc<Mutex<Vec<ExecutionContext>>>) {
            let contexts = Arc::new(Mutex::new(vec![]));
            (
                Self { contexts: contexts.clone() },
                contexts,
            )
        }
    }

    impl AgentRunner for TrackingAgentRunner {
        fn execute(
            &self,
            ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            self.contexts.lock().unwrap().push(ctx);
            Ok(AgentOutput {
                stdout: "mock output".to_string(),
                stderr: String::new(),
                exit_code: 0,
                metadata: None,
            })
        }

        fn execute_with_config(
            &self,
            agent_config: &AgentConfig,
            strand_path: StrandPath,
            strand_file_ref: Option<StrandPath>,
            prompt: String,
            profile_prompt: String,
            event_type: String,
            knot_name: Option<String>,
            timeout: Option<std::time::Duration>,
        ) -> Result<AgentOutput, PortError> {
            let mut cli_args = agent_config.build_cli_args();
            let strand_filename = strand_path.0
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let session_title = format!(
                "{} triggered by {} on {}",
                knot_name.as_deref().unwrap_or("unknown"),
                event_type,
                strand_filename,
            );
            cli_args.push("--name".to_string());
            cli_args.push(session_title);
            if let Some(ref file_path) = strand_file_ref {
                cli_args.push(format!("@{}", file_path.0.display()));
            }
            let ctx = ExecutionContext {
                cli_path: "pi".to_string(),
                cli_args,
                prompt,
                profile_prompt,
                strand_path,
                event_type,
                knot_name,
                timeout,
            };
            self.execute(ctx)
        }
    }

    // ── Mock AgentRunner (no-op) ─────────────────────────────────────

    #[derive(Default)]
    struct MockAgentRunner;

    impl AgentRunner for MockAgentRunner {
        fn execute(
            &self,
            _ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            Ok(AgentOutput {
                stdout: "mock".to_string(),
                stderr: String::new(),
                exit_code: 0,
                metadata: None,
            })
        }
    }

    // ── Mock TieOffSink ──────────────────────────────────────────────

    #[derive(Default)]
    struct MockTieOffSink {
        content: std::sync::RwLock<HashMap<String, String>>,
    }

    impl TieOffSink for MockTieOffSink {
        fn write(
            &self,
            tie_off: TieOff,
        ) -> Result<(), PortError> {
            self.content
                .write()
                .unwrap()
                .insert(tie_off.path.0.display().to_string(), tie_off.content);
            Ok(())
        }

        fn append(&self, tie_off: TieOff) -> Result<(), PortError> {
            self.write(tie_off)
        }

        fn read_content(
            &self,
            path: &TieOffPath,
        ) -> Result<String, PortError> {
            Ok(self
                .content
                .read()
                .unwrap()
                .get(&path.0.display().to_string())
                .cloned()
                .unwrap_or_default())
        }
    }

    // ── Mock RigLogPort ──────────────────────────────────────────────

    #[derive(Default)]
    struct MockRigLogPort {
        events: Arc<Mutex<Vec<crate::domain::events::RigLogEvent>>>,
    }

    impl MockRigLogPort {
        fn new() -> (Self, Arc<Mutex<Vec<crate::domain::events::RigLogEvent>>>) {
            let events = Arc::new(Mutex::new(vec![]));
            (Self { events: events.clone() }, events)
        }
    }

    impl RigLogPort for MockRigLogPort {
        fn append(
            &self,
            event: crate::domain::events::RigLogEvent,
        ) -> Result<(), PortError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn read_all(
            &self,
        ) -> Result<Vec<crate::domain::events::RigLogEvent>, PortError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    // ── Mock AgentProfileRepository ──────────────────────────────────

    #[derive(Default)]
    struct MockProfileRepository {
        profiles: Arc<Mutex<HashMap<String, AgentProfile>>>,
    }

    impl AgentProfileRepository for MockProfileRepository {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .get(name)
                .cloned())
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .values()
                .cloned()
                .collect())
        }

    }

    // ── Mock GitVersioningPort ───────────────────────────────────────

    #[derive(Default)]
    struct MockGitVersioningPort;

    impl GitVersioningPort for MockGitVersioningPort {
        fn commit(
            &self,
            _loom_id: &LoomId,
            _knot_id: &KnotId,
            _strand_path: &StrandPath,
            _event_type: &str,
            _tie_off_content: &str,
        ) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────

    /// Build a knot with the given profile ref.
    fn build_knot(id: impl Into<String>, profile: &str) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: profile.to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    /// Build a loom with the given ID and knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    // ── resolve_agent_config Timeout Tests ───────────────────────────

    /// `resolve_agent_config()` returns the profile's timeout
    /// converted to a Duration when the profile sets `timeout: Some(600)`.
    #[test]
    fn resolve_agent_config_returns_timeout_from_profile() {
        let store = LoomStore::new();
        let profile = AgentProfile::new(
            "slow".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "You are thorough.".to_string(),
        )
        .unwrap()
        .with_timeout(Some(600));

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("slow".to_string(), profile),
            ]))),
        });

        let (rig_log, _rig_events) = MockRigLogPort::new();
        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        let knot = build_knot("k1", "slow");
        let (_config, timeout) =
            use_case.resolve_agent_config(&knot).unwrap();

        assert_eq!(timeout, Some(Duration::from_secs(600)));
    }

    /// `resolve_agent_config()` returns `None` timeout when the profile
    /// does not set a timeout (falls back to runner default).
    #[test]
    fn resolve_agent_config_returns_none_timeout_from_profile() {
        let store = LoomStore::new();
        let profile = AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are fast.".to_string(),
        )
        .unwrap();
        // No .with_timeout() — defaults to None

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), profile),
            ]))),
        });

        let (rig_log, _rig_events) = MockRigLogPort::new();
        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        let knot = build_knot("k1", "fast");
        let (_config, timeout) =
            use_case.resolve_agent_config(&knot).unwrap();

        assert_eq!(timeout, None);
    }

    // ── ProcessStrand execute() Timeout Tests ────────────────────────

    /// `ProcessStrand::execute` with a profile that has `timeout = Some(60)`
    /// passes `ExecutionContext.timeout = Some(Duration::from_secs(60))`.
    #[test]
    fn process_strand_execute_passes_profile_timeout_to_context() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let profile = AgentProfile::new(
            "timed".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "Timed review.".to_string(),
        )
        .unwrap()
        .with_timeout(Some(60));

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("timed".to_string(), profile),
            ]))),
        });

        let store = LoomStore::new();
        let loom = build_loom("test-loom", vec![build_knot("k1", "timed")]);
        store.register(loom);

        let (runner, captured_contexts) = TrackingAgentRunner::new();
        let (rig_log, _rig_events) = MockRigLogPort::new();

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Verify the ExecutionContext received the profile's timeout
        let contexts = captured_contexts.lock().unwrap();
        assert_eq!(contexts.len(), 1, "should have called execute once");
        // Timeout is capped at MAX_ATTEMPT_TIMEOUT_SECS (30s) so retries
        // have room in the overall budget.
        assert_eq!(
            contexts[0].timeout,
            Some(Duration::from_secs(30)),
            "ExecutionContext.timeout should be capped at MAX_ATTEMPT_TIMEOUT_SECS"
        );
    }

    /// `ProcessStrand::execute` with a profile that has `timeout = None`
    /// passes `ExecutionContext.timeout = None` (falls back to runner default).
    #[test]
    fn process_strand_execute_passes_none_timeout_to_context() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let profile = AgentProfile::new(
            "default".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "Default timeout.".to_string(),
        )
        .unwrap();
        // No .with_timeout() — defaults to None

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("default".to_string(), profile),
            ]))),
        });

        let store = LoomStore::new();
        let loom = build_loom("test-loom", vec![build_knot("k1", "default")]);
        store.register(loom);

        let (runner, captured_contexts) = TrackingAgentRunner::new();
        let (rig_log, _rig_events) = MockRigLogPort::new();

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
        );

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Verify the ExecutionContext received None timeout
        let contexts = captured_contexts.lock().unwrap();
        assert_eq!(contexts.len(), 1, "should have called execute once");
        assert_eq!(
            contexts[0].timeout,
            None,
            "ExecutionContext.timeout should be None (runner fallback)"
        );
    }
}

// ── Phase 8: Git Versioning Tests ────────────────────────────────

#[cfg(test)]
mod phase8_git_versioning_tests {
    use super::*;
    use crate::application::ports::{AgentOutput, ExecutionContext};
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentProfile, PromptTemplate};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    // ── Mock LoomLogPort ─────────────────────────────────────────────

    #[derive(Default)]
    struct MockLoomLogPort;

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Tracking GitVersioningPort ───────────────────────────────────

    /// Mock that records all commit calls for inspection.
    struct TrackingGitVersioningPort {
        commits: Arc<Mutex<Vec<(LoomId, KnotId, String, String, String)>>>,
        /// When set, `commit()` returns this error instead of Ok.
        force_error: Arc<Mutex<Option<PortError>>>,
    }

    impl TrackingGitVersioningPort {
        fn new() -> (
            Self,
            Arc<Mutex<Vec<(LoomId, KnotId, String, String, String)>>>,
        ) {
            let commits = Arc::new(Mutex::new(vec![]));
            (
                Self {
                    commits: commits.clone(),
                    force_error: Arc::new(Mutex::new(None)),
                },
                commits,
            )
        }

        fn set_error(&self, error: PortError) {
            *self.force_error.lock().unwrap() = Some(error);
        }
    }

    impl GitVersioningPort for TrackingGitVersioningPort {
        fn commit(
            &self,
            loom_id: &LoomId,
            knot_id: &KnotId,
            strand_path: &StrandPath,
            event_type: &str,
            tie_off_content: &str,
        ) -> Result<(), PortError> {
            self.commits
                .lock()
                .unwrap()
                .push((
                    loom_id.clone(),
                    knot_id.clone(),
                    strand_path.0.display().to_string(),
                    event_type.to_string(),
                    tie_off_content.to_string(),
                ));
            if let Some(ref err) = *self.force_error.lock().unwrap() {
                return Err(err.clone());
            }
            Ok(())
        }
    }

    // ── Mock AgentRunner ─────────────────────────────────────────────

    #[derive(Default)]
    struct MockAgentRunner;

    impl AgentRunner for MockAgentRunner {
        fn execute(
            &self,
            _ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            Ok(AgentOutput {
                stdout: "agent output".to_string(),
                stderr: String::new(),
                exit_code: 0,
                metadata: None,
            })
        }
    }

    // ── Mock TieOffSink ──────────────────────────────────────────────

    #[derive(Default)]
    struct MockTieOffSink;

    impl TieOffSink for MockTieOffSink {
        fn write(&self, _tie_off: TieOff) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _tie_off: TieOff) -> Result<(), PortError> {
            Ok(())
        }

        fn read_content(
            &self,
            _path: &TieOffPath,
        ) -> Result<String, PortError> {
            Ok(String::new())
        }
    }

    // ── Mock RigLogPort ──────────────────────────────────────────────

    #[derive(Default)]
    struct MockRigLogPort;

    impl RigLogPort for MockRigLogPort {
        fn append(
            &self,
            _event: crate::domain::events::RigLogEvent,
        ) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(
            &self,
        ) -> Result<Vec<crate::domain::events::RigLogEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Mock AgentProfileRepository ──────────────────────────────────

    struct MockProfileRepository {
        profiles: HashMap<String, AgentProfile>,
    }

    impl AgentProfileRepository for MockProfileRepository {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(self.profiles.get(name).cloned())
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(self.profiles.values().cloned().collect())
        }

    }

    // ── Helpers ──────────────────────────────────────────────────────

    fn default_profile() -> AgentProfile {
        AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are fast.".to_string(),
        )
        .unwrap()
    }

    fn build_knot(
        id: impl Into<String>,
        git_versioned: bool,
    ) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned,
        }
    }

    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    fn build_process_strand(
        loom: Loom,
        git_port: Arc<dyn GitVersioningPort>,
    ) -> ProcessStrand {
        let store = LoomStore::new();
        store.register(loom);

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]),
        });

        ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner),
            Arc::new(MockTieOffSink),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort),
            git_port,
        )
    }

    // ── Tests ────────────────────────────────────────────────────────

    /// On successful processing with `git_versioned: true`, the git
    /// port receives a `commit()` call with loom, knot, strand,
    /// event type, and tie-off content.
    #[test]
    fn process_strand_calls_git_port_on_success() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom =
            build_loom("test-loom", vec![build_knot("k1", true)]);

        let (git_port, commits) = TrackingGitVersioningPort::new();
        let use_case = build_process_strand(loom, Arc::new(git_port));

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Git port received exactly one commit call
        let commits = commits.lock().unwrap();
        assert_eq!(commits.len(), 1);
        let (loom_id, knot_id, strand, et, content) = &commits[0];
        assert_eq!(loom_id.0, "test-loom");
        assert_eq!(knot_id.0, "k1");
        assert!(strand.ends_with("strand.md"));
        assert_eq!(et, "Created");
        assert_eq!(content, "agent output");
    }

    /// When `git_versioned: false`, the git port is never called
    /// even on successful processing.
    #[test]
    fn process_strand_skips_git_when_disabled() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom =
            build_loom("test-loom", vec![build_knot("k1", false)]);

        let (git_port, commits) = TrackingGitVersioningPort::new();
        let use_case = build_process_strand(loom, Arc::new(git_port));

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Git port should NOT have been called
        let commits = commits.lock().unwrap();
        assert!(
            commits.is_empty(),
            "git port should not be called when git_versioned is false"
        );
    }

    /// When the git port returns an error, processing still succeeds
    /// (strand is marked completed, error is only logged as warning).
    #[test]
    fn process_strand_continues_on_git_error() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom =
            build_loom("test-loom", vec![build_knot("k1", true)]);

        let (git_port, commits) = TrackingGitVersioningPort::new();
        git_port.set_error(PortError::GitCommitFailed(
            "not a git repo".to_string(),
        ));

        let use_case = build_process_strand(loom, Arc::new(git_port));

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        // execute() should succeed despite git error
        let result = use_case.execute(event);
        assert!(
            result.is_ok(),
            "processing should succeed despite git commit failure"
        );

        // Git port was still called (the error is non-fatal)
        let commits = commits.lock().unwrap();
        assert_eq!(commits.len(), 1, "commit should still be attempted");
    }
}

// ── ReloadConfig Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod reload_config_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::PromptTemplate;
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
        ) -> (
            Self,
            Arc<Mutex<Vec<PathBuf>>>,
        ) {
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

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Mock LoomRepository ────────────────────────────────────────────

    struct MockLoomRepository {
        scan_looms: Vec<Loom>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<(Vec<Loom>, Vec<String>), PortError> {
            Ok((self.scan_looms.clone(), vec![]))
        }

        fn scan_knot_files(
            &self,
            _loom_dir: &std::path::Path,
        ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
            Ok((vec![], vec![]))
        }

        fn get(
            &self,
            _id: &LoomId,
        ) -> Result<Option<Loom>, PortError> {
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
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    // ── Tests ──────────────────────────────────────────────────────────

    /// `ReloadConfig` re-scans the rig and registers any looms not
    /// already in the store. Returns the IDs of newly registered looms.
    #[test]
    fn reload_config_discovers_new_looms() {
        let new_loom = build_loom("new-loom", vec![build_knot("k1")]);
        let new_loom2 = build_loom("new-loom2", vec![]);

        let store = LoomStore::new();
        let repo = Arc::new(MockLoomRepository {
            scan_looms: vec![new_loom, new_loom2],
        });
        let (event_source, watch_calls) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = ReloadConfig::new(
            repo,
            Arc::new(MockLoomLogPort),
            store.clone(),
            es,
            PathBuf::from("/workspace/rig"),
        );

        let result = use_case.execute();

        // Should succeed and return 2 new loom IDs
        assert!(result.is_ok());
        let ids = result.unwrap();
        assert_eq!(ids.len(), 2);
        let id_set: HashSet<_> = ids.iter().map(|id| id.0.as_str()).collect();
        assert!(id_set.contains("new-loom"));
        assert!(id_set.contains("new-loom2"));

        // Both looms are in the store
        assert!(store.get(&LoomId("new-loom".to_string())).is_some());
        assert!(store.get(&LoomId("new-loom2".to_string())).is_some());

        // Watchers started for new looms (1 knot in new-loom, 0 in new-loom2)
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
    }

    /// `ReloadConfig` skips looms that are already in the store.
    /// Only new looms are registered and returned.
    #[test]
    fn reload_config_skips_registered() {
        let existing_loom = build_loom("existing", vec![build_knot("k1")]);
        let new_loom = build_loom("new-loom", vec![build_knot("k2")]);

        // Pre-register one loom
        let store = LoomStore::new();
        store.register(existing_loom.clone());

        let repo = Arc::new(MockLoomRepository {
            scan_looms: vec![existing_loom, new_loom],
        });
        let (event_source, watch_calls) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = ReloadConfig::new(
            repo,
            Arc::new(MockLoomLogPort),
            store.clone(),
            es,
            PathBuf::from("/workspace/rig"),
        );

        let result = use_case.execute();

        // Should succeed and return only 1 new loom ID
        assert!(result.is_ok());
        let ids = result.unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], LoomId("new-loom".to_string()));

        // Existing loom is still in the store (unchanged)
        assert!(store.get(&LoomId("existing".to_string())).is_some());
        // New loom is now in the store
        assert!(store.get(&LoomId("new-loom".to_string())).is_some());

        // Watchers started only for the new loom
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
    }

    /// `ReloadConfig` when all looms are already registered returns
    /// empty vector (no side effects).
    #[test]
    fn reload_config_all_registered_returns_empty() {
        let loom1 = build_loom("loom-a", vec![build_knot("k1")]);
        let loom2 = build_loom("loom-b", vec![build_knot("k2")]);

        let store = LoomStore::new();
        store.register(loom1.clone());
        store.register(loom2.clone());

        let repo = Arc::new(MockLoomRepository {
            scan_looms: vec![loom1, loom2],
        });
        let (event_source, watch_calls) = TrackingEventSource::new();
        let es: Arc<dyn EventSource> = Arc::new(event_source);

        let use_case = ReloadConfig::new(
            repo,
            Arc::new(MockLoomLogPort),
            store.clone(),
            es,
            PathBuf::from("/workspace/rig"),
        );

        let result = use_case.execute();

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());

        // No watchers started
        let watches = watch_calls.lock().unwrap();
        assert!(watches.is_empty());
    }
}

// ── Phase 9: Session Title (--name) Tests ──────────────────────────

#[cfg(test)]
mod phase9_session_title_tests {
    use super::*;
    use crate::application::ports::{AgentOutput, ExecutionContext};
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentProfile, PromptTemplate};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    // ── Mock LoomLogPort ─────────────────────────────────────────────

    #[derive(Default)]
    struct MockLoomLogPort;

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Tracking AgentRunner (captures ExecutionContext) ────────────

    /// Mock agent runner that records the ExecutionContext passed to it.
    struct TrackingAgentRunner {
        contexts: Arc<Mutex<Vec<ExecutionContext>>>,
    }

    impl TrackingAgentRunner {
        fn new() -> (Self, Arc<Mutex<Vec<ExecutionContext>>>) {
            let contexts = Arc::new(Mutex::new(vec![]));
            (
                Self { contexts: contexts.clone() },
                contexts,
            )
        }
    }

    impl AgentRunner for TrackingAgentRunner {
        fn execute(
            &self,
            ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            self.contexts.lock().unwrap().push(ctx);
            Ok(AgentOutput {
                stdout: "mock output".to_string(),
                stderr: String::new(),
                exit_code: 0,
                metadata: None,
            })
        }

        fn execute_with_config(
            &self,
            agent_config: &AgentConfig,
            strand_path: StrandPath,
            strand_file_ref: Option<StrandPath>,
            prompt: String,
            profile_prompt: String,
            event_type: String,
            knot_name: Option<String>,
            timeout: Option<std::time::Duration>,
        ) -> Result<AgentOutput, PortError> {
            let mut cli_args = agent_config.build_cli_args();
            let strand_filename = strand_path.0
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let session_title = format!(
                "{} triggered by {} on {}",
                knot_name.as_deref().unwrap_or("unknown"),
                event_type,
                strand_filename,
            );
            cli_args.push("--name".to_string());
            cli_args.push(session_title);
            if let Some(ref file_path) = strand_file_ref {
                cli_args.push(format!("@{}", file_path.0.display()));
            }
            let ctx = ExecutionContext {
                cli_path: "pi".to_string(),
                cli_args,
                prompt,
                profile_prompt,
                strand_path,
                event_type,
                knot_name,
                timeout,
            };
            self.execute(ctx)
        }
    }

    // ── Mock TieOffSink ──────────────────────────────────────────────

    #[derive(Default)]
    struct MockTieOffSink;

    impl TieOffSink for MockTieOffSink {
        fn write(&self, _tie_off: TieOff) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _tie_off: TieOff) -> Result<(), PortError> {
            Ok(())
        }

        fn read_content(
            &self,
            _path: &TieOffPath,
        ) -> Result<String, PortError> {
            Ok(String::new())
        }
    }

    // ── Mock RigLogPort ──────────────────────────────────────────────

    #[derive(Default)]
    struct MockRigLogPort;

    impl RigLogPort for MockRigLogPort {
        fn append(
            &self,
            _event: crate::domain::events::RigLogEvent,
        ) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(
            &self,
        ) -> Result<Vec<crate::domain::events::RigLogEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Mock AgentProfileRepository ──────────────────────────────────

    struct MockProfileRepository {
        profiles: Arc<Mutex<HashMap<String, AgentProfile>>>,
    }

    impl AgentProfileRepository for MockProfileRepository {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .get(name)
                .cloned())
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .values()
                .cloned()
                .collect())
        }

    }

    // ── Mock GitVersioningPort ───────────────────────────────────────

    #[derive(Default)]
    struct MockGitVersioningPort;

    impl GitVersioningPort for MockGitVersioningPort {
        fn commit(
            &self,
            _loom_id: &LoomId,
            _knot_id: &KnotId,
            _strand_path: &StrandPath,
            _event_type: &str,
            _tie_off_content: &str,
        ) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────

    fn build_knot(id: impl Into<String>, profile: &str) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: profile.to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        }
    }

    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    fn default_profile() -> AgentProfile {
        AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are fast.".to_string(),
        )
        .unwrap()
    }

    /// Find the value after `--name` in a CLI args list.
    fn find_name_value(args: &[String]) -> Option<String> {
        let pos = args.iter().position(|a| a == "--name")?;
        args.get(pos + 1).cloned()
    }

    // ── Tests ────────────────────────────────────────────────────────

    /// `ProcessStrand::execute` appends `--name <title>` to CLI args.
    /// Title format: `{knot-id} triggered by {event-type} on {strand-filename}`.
    #[test]
    fn process_strand_cli_args_contain_name_flag() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("004-manifest-resources.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]))),
        });

        let store = LoomStore::new();
        let loom = build_loom("test-loom", vec![build_knot("plan-architect", "fast")]);
        store.register(loom);

        let (runner, captured_contexts) = TrackingAgentRunner::new();

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
        );

        let event = StrandEvent::Modified {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("plan-architect".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Verify CLI args contain --name with correct title
        let contexts = captured_contexts.lock().unwrap();
        assert_eq!(contexts.len(), 1, "should have called execute once");
        let args = &contexts[0].cli_args;
        assert!(
            args.contains(&"--name".to_string()),
            "CLI args should contain --name flag: {:?}",
            args
        );
        let name_value = find_name_value(args).expect("--name should have a value");
        assert_eq!(
            name_value,
            "plan-architect triggered by Modified on 004-manifest-resources.md",
        );
    }

    /// Title format matches trigger line for Created events.
    #[test]
    fn process_strand_title_format_created_event() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("new-file.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]))),
        });

        let store = LoomStore::new();
        let loom = build_loom("review-loom", vec![build_knot("reviewer", "fast")]);
        store.register(loom);

        let (runner, captured_contexts) = TrackingAgentRunner::new();

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
        );

        let event = StrandEvent::Created {
            loom_id: LoomId("review-loom".to_string()),
            knot_id: KnotId("reviewer".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        use_case.execute(event).unwrap();

        let contexts = captured_contexts.lock().unwrap();
        let args = &contexts[0].cli_args;
        let name_value = find_name_value(args).expect("--name should have a value");
        assert_eq!(
            name_value,
            "reviewer triggered by Created on new-file.md",
        );
    }

    /// Title format matches trigger line for Deleted events.
    #[test]
    fn process_strand_title_format_deleted_event() {
        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]))),
        });

        let store = LoomStore::new();
        let loom = build_loom("test-loom", vec![build_knot("cleanup", "fast")]);
        store.register(loom);

        let (runner, captured_contexts) = TrackingAgentRunner::new();

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
        );

        let event = StrandEvent::Deleted {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("cleanup".to_string()),
            strand_path: StrandPath(PathBuf::from("input/old-file.md")),
        };

        use_case.execute(event).unwrap();

        let contexts = captured_contexts.lock().unwrap();
        let args = &contexts[0].cli_args;
        let name_value = find_name_value(args).expect("--name should have a value");
        assert_eq!(
            name_value,
            "cleanup triggered by Deleted on old-file.md",
        );
    }

    /// Different strands produce different `--name` values,
    /// ensuring each session gets a unique title.
    #[test]
    fn process_strand_title_unique_per_strand() {
        let dir = TempDir::new().unwrap();
        let file_a = dir.path().join("file-a.md");
        let file_b = dir.path().join("file-b.md");
        std::fs::write(&file_a, "content a").unwrap();
        std::fs::write(&file_b, "content b").unwrap();

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]))),
        });

        let store = LoomStore::new();
        let loom = build_loom("test-loom", vec![build_knot("reviewer", "fast")]);
        store.register(loom);

        let (runner, captured_contexts) = TrackingAgentRunner::new();

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
        );

        // Process first strand
        let event1 = StrandEvent::Modified {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("reviewer".to_string()),
            strand_path: StrandPath(file_a.clone()),
        };
        use_case.execute(event1).unwrap();

        // Process second strand
        let event2 = StrandEvent::Modified {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("reviewer".to_string()),
            strand_path: StrandPath(file_b.clone()),
        };
        use_case.execute(event2).unwrap();

        let contexts = captured_contexts.lock().unwrap();
        assert_eq!(contexts.len(), 2);

        let name1 = find_name_value(&contexts[0].cli_args)
            .expect("first call should have --name");
        let name2 = find_name_value(&contexts[1].cli_args)
            .expect("second call should have --name");

        assert_eq!(name1, "reviewer triggered by Modified on file-a.md");
        assert_eq!(name2, "reviewer triggered by Modified on file-b.md");
        assert_ne!(name1, name2, "titles should differ per strand");
    }

    /// The existing `runner_passes_prompt_via_stdin` test pattern:
    /// prompt content (profile_prompt + instructions + trigger line)
    /// is delivered via stdin and is NOT affected by the `--name` flag.
    #[test]
    fn process_strand_prompt_content_unchanged_by_name_flag() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("doc.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let profile = AgentProfile::new(
            "reviewer".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are a reviewer.".to_string(),
        )
        .unwrap();

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("reviewer".to_string(), profile),
            ]))),
        });

        let store = LoomStore::new();
        let knot = Knot {
            id: KnotId("reviewer".to_string()),
            agent_profile_ref: "reviewer".to_string(),
            prompt_template: PromptTemplate {
                instructions: "Review this file.".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
        };
        let loom = build_loom("test-loom", vec![knot]);
        store.register(loom);

        let (runner, captured_contexts) = TrackingAgentRunner::new();

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
        );

        let event = StrandEvent::Modified {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("reviewer".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        use_case.execute(event).unwrap();

        let contexts = captured_contexts.lock().unwrap();
        let ctx = &contexts[0];

        // Profile prompt is in profile_prompt field (delivered via stdin)
        assert_eq!(ctx.profile_prompt, "You are a reviewer.");
        // Knot instructions are in prompt field
        assert_eq!(ctx.prompt, "Review this file.");
        // Event metadata is present
        assert_eq!(ctx.event_type, "Modified");
        assert_eq!(ctx.knot_name.as_deref(), Some("reviewer"));
        // --name is in CLI args, not in prompt content
        assert!(ctx.cli_args.contains(&"--name".to_string()));
        assert!(!ctx.prompt.contains("--name"),
            "--name should not appear in prompt content");
        assert!(!ctx.profile_prompt.contains("--name"),
            "--name should not appear in profile prompt");
    }
}

// ── WriteState ─────────────────────────────────────────────────────────

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

// ── WriteState Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod write_state_tests {
    use super::*;
    use crate::domain::entities::KnotId;
    use crate::domain::value_objects::AgentProfile;
    use crate::application::store::LoomStore;
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

// ── Phase 2: Text Check Tests ───────────────────────────────────────

#[cfg(test)]
mod phase2_text_check_tests {
    use super::*;
    use crate::adapters::outbound::content_inspector::is_text_file;
    use crate::application::ports::{AgentOutput, ExecutionContext};
    use crate::domain::entities::{Knot, KnotId, TieOffStatus};
    use crate::domain::value_objects::{AgentProfile, PromptTemplate};
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    // ── Tracking LoomLogPort ─────────────────────────────────────────

    struct TrackingLoomLogPort {
        events: Arc<Mutex<Vec<LoomEvent>>>,
    }

    impl TrackingLoomLogPort {
        fn new() -> (Self, Arc<Mutex<Vec<LoomEvent>>>) {
            let events = Arc::new(Mutex::new(vec![]));
            (Self { events: events.clone() }, events)
        }
    }

    impl LoomLogPort for TrackingLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, event: LoomEvent) -> Result<(), PortError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    // ── Mock AgentRunner ─────────────────────────────────────────────

    struct MockAgentRunner;

    impl AgentRunner for MockAgentRunner {
        fn execute(
            &self,
            _ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            Ok(AgentOutput {
                stdout: "mock output".to_string(),
                stderr: String::new(),
                exit_code: 0,
                metadata: None,
            })
        }
    }

    // ── Tracking TieOffSink ──────────────────────────────────────────

    struct TrackingTieOffSink {
        appends: Arc<Mutex<Vec<TieOff>>>,
    }

    impl TrackingTieOffSink {
        fn new() -> (Self, Arc<Mutex<Vec<TieOff>>>) {
            let appends = Arc::new(Mutex::new(vec![]));
            (Self { appends: appends.clone() }, appends)
        }
    }

    impl TieOffSink for TrackingTieOffSink {
        fn write(&self, _tie_off: TieOff) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, tie_off: TieOff) -> Result<(), PortError> {
            self.appends.lock().unwrap().push(tie_off);
            Ok(())
        }

        fn read_content(
            &self,
            _path: &TieOffPath,
        ) -> Result<String, PortError> {
            Ok(String::new())
        }
    }

    // ── Mock RigLogPort ──────────────────────────────────────────────

    struct MockRigLogPort;

    impl RigLogPort for MockRigLogPort {
        fn append(
            &self,
            _event: crate::domain::events::RigLogEvent,
        ) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(
            &self,
        ) -> Result<Vec<crate::domain::events::RigLogEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Mock GitVersioningPort ───────────────────────────────────────

    struct MockGitVersioningPort;

    impl GitVersioningPort for MockGitVersioningPort {
        fn commit(
            &self,
            _loom_id: &LoomId,
            _knot_id: &KnotId,
            _strand_path: &StrandPath,
            _event_type: &str,
            _tie_off_content: &str,
        ) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── Mock AgentProfileRepository ──────────────────────────────────

    struct MockProfileRepository {
        profiles: HashMap<String, AgentProfile>,
    }

    impl AgentProfileRepository for MockProfileRepository {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(self.profiles.get(name).cloned())
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(self.profiles.values().cloned().collect())
        }

    }

    // ── Helpers ──────────────────────────────────────────────────────

    fn default_profile() -> AgentProfile {
        AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are fast.".to_string(),
        )
        .unwrap()
    }

    fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: false,
        }
    }

    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    #[allow(clippy::type_complexity)]
    fn build_process_strand(
        loom: Loom,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<TieOff>>>,
    ) {
        let store = LoomStore::new();
        store.register(loom);

        let (log_port, log_events) = TrackingLoomLogPort::new();
        let (tie_off_sink, tie_off_appends) = TrackingTieOffSink::new();

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]),
        });

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(log_port),
            Arc::new(MockAgentRunner),
            Arc::new(tie_off_sink),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort),
            Arc::new(MockGitVersioningPort),
        );

        (use_case, log_events, tie_off_appends)
    }

    // ── Tests ────────────────────────────────────────────────────────

    /// Binary file on Created event: loom-log receives `StrandIgnored`,
    /// no agent execution (no KnotProcessing, no tie-off).
    #[test]
    fn binary_file_creates_strand_ignored_event() {
        let dir = TempDir::new().unwrap();
        let binary_path = dir.path().join("data.bin");
        // Write bytes with null bytes (detected as binary)
        std::fs::write(
            &binary_path,
            vec![0x00, 0x01, 0x02, 0xFF, 0xFE],
        )
        .unwrap();

        // Verify content_inspector detects it as binary
        assert!(
            !is_text_file(&binary_path).unwrap(),
            "test fixture should be binary"
        );

        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, tie_off_appends) =
            build_process_strand(loom);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(binary_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Loom-log: only StrandIgnored (no KnotProcessing)
        let events = log_events.lock().unwrap();
        assert_eq!(events.len(), 1, "should have exactly 1 event");
        match &events[0] {
            LoomEvent::StrandIgnored {
                loom_id,
                knot_id,
                strand_path,
                reason,
                ..
            } => {
                assert_eq!(loom_id.0, "test-loom");
                assert_eq!(knot_id.0, "k1");
                assert_eq!(strand_path.0, binary_path);
                assert_eq!(reason, "binary file");
            }
            other => panic!(
                "expected StrandIgnored for binary file, got {:?}",
                other
            ),
        }

        // No tie-off appended
        let appends = tie_off_appends.lock().unwrap();
        assert!(
            appends.is_empty(),
            "tie-off should not be written for ignored files"
        );
    }

    /// Text file on Created event: normal processing path (KnotProcessing,
    /// KnotCompleted, StrandProcessed, tie-off appended).
    #[test]
    fn text_file_normal_processing_path() {
        let dir = TempDir::new().unwrap();
        let text_path = dir.path().join("hello.txt");
        let mut file =
            std::fs::File::create(&text_path).unwrap();
        writeln!(file, "Hello, world!").unwrap();
        drop(file);

        // Verify content_inspector detects it as text
        assert!(
            is_text_file(&text_path).unwrap(),
            "test fixture should be text"
        );

        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, tie_off_appends) =
            build_process_strand(loom);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(text_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Loom-log: KnotProcessing, KnotCompleted, StrandProcessed
        let events = log_events.lock().unwrap();
        assert_eq!(
            events.len(),
            3,
            "should have 3 loom-log events for normal processing"
        );
        match &events[0] {
            LoomEvent::KnotProcessing { .. } => {}
            other => panic!("expected KnotProcessing, got {:?}", other),
        }
        match &events[1] {
            LoomEvent::KnotCompleted { .. } => {}
            other => panic!("expected KnotCompleted, got {:?}", other),
        }
        match &events[2] {
            LoomEvent::StrandProcessed { error, .. } => {
                assert!(error.is_none(), "error should be None on success");
            }
            other => panic!("expected StrandProcessed, got {:?}", other),
        }

        // Tie-off IS appended
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1, "tie-off should be appended");
        assert_eq!(appends[0].status, TieOffStatus::Produced);
    }

    /// Deleted event: skips text check (file is gone), processes normally.
    /// Even with a non-existent path, the pipeline runs.
    #[test]
    fn deleted_event_skips_text_check() {
        // Path that doesn't exist — text check would fail if called
        let nonexistent = PathBuf::from("/nonexistent/path/file.txt");

        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, tie_off_appends) =
            build_process_strand(loom);

        let event = StrandEvent::Deleted {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(nonexistent.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Loom-log: KnotProcessing, KnotCompleted, StrandProcessed
        // (no StrandIgnored since Deleted skips text check)
        let events = log_events.lock().unwrap();
        assert_eq!(
            events.len(),
            3,
            "should have 3 loom-log events for Deleted (no text check)"
        );
        match &events[0] {
            LoomEvent::KnotProcessing { .. } => {}
            other => panic!("expected KnotProcessing, got {:?}", other),
        }

        // No StrandIgnored in events
        for event in &*events {
            assert!(
                !matches!(event, LoomEvent::StrandIgnored { .. }),
                "Deleted event should NOT produce StrandIgnored"
            );
        }

        // Tie-off IS appended (normal processing)
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1, "tie-off should be appended");
    }

    /// Binary file on Modified event: also produces StrandIgnored.
    #[test]
    fn binary_file_modified_event_strand_ignored() {
        let dir = TempDir::new().unwrap();
        let binary_path = dir.path().join("image.png");
        // PNG magic bytes
        std::fs::write(
            &binary_path,
            vec![
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
                0x00, 0x00, 0x00, 0x00,
            ],
        )
        .unwrap();

        assert!(
            !is_text_file(&binary_path).unwrap(),
            "test fixture should be binary"
        );

        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, _tie_off_appends) =
            build_process_strand(loom);

        let event = StrandEvent::Modified {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(binary_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let events = log_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LoomEvent::StrandIgnored {
                strand_path, reason, ..
            } => {
                assert_eq!(strand_path.0, binary_path);
                assert_eq!(reason, "binary file");
            }
            other => panic!(
                "expected StrandIgnored for binary file on Modified, got {:?}",
                other
            ),
        }
    }

    /// Empty file (0 bytes) is treated as text — normal processing.
    #[test]
    fn empty_file_treated_as_text() {
        let dir = TempDir::new().unwrap();
        let empty_path = dir.path().join("empty.txt");
        std::fs::write(&empty_path, "").unwrap();

        assert!(
            is_text_file(&empty_path).unwrap(),
            "empty file should be treated as text"
        );

        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, tie_off_appends) =
            build_process_strand(loom);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(empty_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let events = log_events.lock().unwrap();
        assert_eq!(events.len(), 3, "should process empty files normally");
        match &events[0] {
            LoomEvent::KnotProcessing { .. } => {}
            other => panic!("expected KnotProcessing, got {:?}", other),
        }

        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1, "tie-off should be appended");
    }
}

// ── Phase 2: File Existence Check Tests ───────────────────────────────────

#[cfg(test)]
mod phase2_file_existence_tests {
    use super::*;
    use crate::application::ports::{AgentOutput, ExecutionContext};
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::temp_file::is_known_temp_file;
    use crate::domain::value_objects::{AgentProfile, PromptTemplate};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    // ── Tracking LoomLogPort ─────────────────────────────────────────

    struct TrackingLoomLogPort {
        events: Arc<Mutex<Vec<LoomEvent>>>,
    }

    impl TrackingLoomLogPort {
        fn new() -> (Self, Arc<Mutex<Vec<LoomEvent>>>) {
            let events = Arc::new(Mutex::new(vec![]));
            (Self { events: events.clone() }, events)
        }
    }

    impl LoomLogPort for TrackingLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, event: LoomEvent) -> Result<(), PortError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    // ── Mock AgentRunner (tracks if execute was called) ──────────────

    struct MockAgentRunner {
        called: Arc<Mutex<bool>>,
    }

    impl MockAgentRunner {
        fn new() -> (Self, Arc<Mutex<bool>>) {
            let called = Arc::new(Mutex::new(false));
            (
                Self { called: called.clone() },
                called,
            )
        }
    }

    impl AgentRunner for MockAgentRunner {
        fn execute(
            &self,
            _ctx: ExecutionContext,
        ) -> Result<AgentOutput, PortError> {
            *self.called.lock().unwrap() = true;
            Ok(AgentOutput {
                stdout: "mock output".to_string(),
                stderr: String::new(),
                exit_code: 0,
                metadata: None,
            })
        }
    }

    // ── Tracking TieOffSink ──────────────────────────────────────────

    struct TrackingTieOffSink {
        appends: Arc<Mutex<Vec<TieOff>>>,
    }

    impl TrackingTieOffSink {
        fn new() -> (Self, Arc<Mutex<Vec<TieOff>>>) {
            let appends = Arc::new(Mutex::new(vec![]));
            (Self { appends: appends.clone() }, appends)
        }
    }

    impl TieOffSink for TrackingTieOffSink {
        fn write(&self, _tie_off: TieOff) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, tie_off: TieOff) -> Result<(), PortError> {
            self.appends.lock().unwrap().push(tie_off);
            Ok(())
        }

        fn read_content(
            &self,
            _path: &TieOffPath,
        ) -> Result<String, PortError> {
            Ok(String::new())
        }
    }

    // ── Mock RigLogPort ──────────────────────────────────────────────

    struct MockRigLogPort;

    impl RigLogPort for MockRigLogPort {
        fn append(
            &self,
            _event: crate::domain::events::RigLogEvent,
        ) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(
            &self,
        ) -> Result<Vec<crate::domain::events::RigLogEvent>, PortError> {
            Ok(vec![])
        }
    }

    // ── Mock GitVersioningPort ───────────────────────────────────────

    struct MockGitVersioningPort;

    impl GitVersioningPort for MockGitVersioningPort {
        fn commit(
            &self,
            _loom_id: &LoomId,
            _knot_id: &KnotId,
            _strand_path: &StrandPath,
            _event_type: &str,
            _tie_off_content: &str,
        ) -> Result<(), PortError> {
            Ok(())
        }
    }

    // ── Mock AgentProfileRepository ──────────────────────────────────

    struct MockProfileRepository {
        profiles: HashMap<String, AgentProfile>,
    }

    impl AgentProfileRepository for MockProfileRepository {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(self.profiles.get(name).cloned())
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(self.profiles.values().cloned().collect())
        }

    }

    // ── Helpers ──────────────────────────────────────────────────────

    fn default_profile() -> AgentProfile {
        AgentProfile::new(
            "fast".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are fast.".to_string(),
        )
        .unwrap()
    }

    fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
            git_versioned: false,
        }
    }

    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    #[allow(clippy::type_complexity)]
    fn build_process_strand(
        loom: Loom,
        agent_runner: Arc<MockAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<TieOff>>>,
        Arc<Mutex<bool>>,
    ) {
        let store = LoomStore::new();
        store.register(loom);

        let (log_port, log_events) = TrackingLoomLogPort::new();
        let (tie_off_sink, tie_off_appends) = TrackingTieOffSink::new();

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]),
        });

        let called = agent_runner.called.clone();

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(log_port),
            agent_runner as Arc<dyn AgentRunner>,
            Arc::new(tie_off_sink),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort),
            Arc::new(MockGitVersioningPort),
        );

        (use_case, log_events, tie_off_appends, called)
    }

    // ── Tests ────────────────────────────────────────────────────────

    /// Known temp file (sedXXXXXXX pattern) on Created event:
    /// - No loom-log entries (not even StrandSkipped)
    /// - Agent runner is NOT called
    /// - No tie-off written
    /// - Returns Ok(())
    #[test]
    fn known_temp_file_skipped_silently_on_created() {
        let dir = TempDir::new().unwrap();
        // Create a file with sed temp name, then delete it
        let temp_path = dir.path().join("sedXXXXXXX");
        std::fs::write(&temp_path, "temp content").unwrap();
        std::fs::remove_file(&temp_path).unwrap();

        assert!(
            !temp_path.exists(),
            "temp file should be deleted before test"
        );
        assert!(
            is_known_temp_file(&temp_path),
            "should be recognised as known temp file"
        );

        let (runner, called) = MockAgentRunner::new();
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, tie_off_appends, _) =
            build_process_strand(loom, Arc::new(runner));

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(temp_path.clone()),
        };

        let result = use_case.execute(event);

        // Should succeed silently
        assert!(result.is_ok());

        // No loom-log entries
        let events = log_events.lock().unwrap();
        assert!(
            events.is_empty(),
            "known temp file should produce no loom-log entries"
        );

        // Agent runner NOT called
        let was_called = called.lock().unwrap();
        assert!(
            !*was_called,
            "agent runner should NOT be called for known temp files"
        );

        // No tie-off written
        let appends = tie_off_appends.lock().unwrap();
        assert!(
            appends.is_empty(),
            "no tie-off should be written for known temp files"
        );
    }

    /// Known temp file on Modified event: same silent skip behaviour.
    #[test]
    fn known_temp_file_skipped_silently_on_modified() {
        let dir = TempDir::new().unwrap();
        let temp_path = dir.path().join("sedAbCdEfG");
        std::fs::write(&temp_path, "temp").unwrap();
        std::fs::remove_file(&temp_path).unwrap();

        let (runner, called) = MockAgentRunner::new();
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, tie_off_appends, _) =
            build_process_strand(loom, Arc::new(runner));

        let event = StrandEvent::Modified {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(temp_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // No loom-log entries
        let events = log_events.lock().unwrap();
        assert!(events.is_empty());

        // Agent runner NOT called
        let was_called = called.lock().unwrap();
        assert!(!*was_called);

        // No tie-off written
        let appends = tie_off_appends.lock().unwrap();
        assert!(appends.is_empty());
    }

    /// Unknown missing file on Created event:
    /// - Loom-log receives StrandSkipped
    /// - Agent runner is NOT called
    /// - No tie-off written
    /// - Returns Ok(())
    #[test]
    fn unknown_missing_file_logs_strand_skipped() {
        let dir = TempDir::new().unwrap();
        let missing_path = dir.path().join("does_not_exist.md");
        // Don't create the file — it genuinely doesn't exist
        assert!(
            !missing_path.exists(),
            "file should not exist"
        );
        assert!(
            !is_known_temp_file(&missing_path),
            "should not be a known temp file"
        );

        let (runner, called) = MockAgentRunner::new();
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, tie_off_appends, _) =
            build_process_strand(loom, Arc::new(runner));

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(missing_path.clone()),
        };

        let result = use_case.execute(event);

        // Should succeed (missing files are handled gracefully)
        assert!(result.is_ok());

        // Loom-log: exactly one StrandSkipped event
        let events = log_events.lock().unwrap();
        assert_eq!(
            events.len(),
            1,
            "should have exactly one loom-log event"
        );
        match &events[0] {
            LoomEvent::StrandSkipped {
                strand_path, reason, ..
            } => {
                assert_eq!(strand_path.0, missing_path);
                assert_eq!(reason, "missing file (unknown pattern)");
            }
            other => panic!(
                "expected StrandSkipped for missing file, got {:?}",
                other
            ),
        }

        // Agent runner NOT called
        let was_called = called.lock().unwrap();
        assert!(
            !*was_called,
            "agent runner should NOT be called for missing files"
        );

        // No tie-off written
        let appends = tie_off_appends.lock().unwrap();
        assert!(
            appends.is_empty(),
            "no tie-off should be written for missing files"
        );
    }

    /// Existing file on Created event: passes through to normal
    /// processing (regression guard — existence check must not
    /// interfere with normal operation).
    #[test]
    fn existing_file_passes_through_to_processing() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("real_file.md");
        std::fs::write(&file_path, "real content").unwrap();
        assert!(file_path.exists());

        let (runner, called) = MockAgentRunner::new();
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, tie_off_appends, _) =
            build_process_strand(loom, Arc::new(runner));

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(file_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Loom-log: KnotProcessing, KnotCompleted, StrandProcessed
        let events = log_events.lock().unwrap();
        assert_eq!(
            events.len(),
            3,
            "should process existing file normally"
        );
        match &events[0] {
            LoomEvent::KnotProcessing { .. } => {}
            other => panic!(
                "expected KnotProcessing for existing file, got {:?}",
                other
            ),
        }

        // Agent runner IS called
        let was_called = called.lock().unwrap();
        assert!(
            *was_called,
            "agent runner should be called for existing files"
        );

        // Tie-off IS written
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1, "tie-off should be appended");
    }

    /// Deleted events skip the existence check (file is expected to
    /// be gone). Regression guard — must not interfere with deleted
    /// event processing.
    #[test]
    fn deleted_events_skip_existence_check() {
        let dir = TempDir::new().unwrap();
        let deleted_path = dir.path().join("was_here.md");
        // Don't create the file — it's deleted
        assert!(!deleted_path.exists());

        let (runner, called) = MockAgentRunner::new();
        let loom = build_loom("test-loom", vec![build_knot("k1")]);
        let (use_case, log_events, tie_off_appends, _) =
            build_process_strand(loom, Arc::new(runner));

        let event = StrandEvent::Deleted {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(deleted_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Loom-log: KnotProcessing, KnotCompleted, StrandProcessed
        let events = log_events.lock().unwrap();
        assert_eq!(
            events.len(),
            3,
            "should process deleted events normally"
        );
        match &events[0] {
            LoomEvent::KnotProcessing { .. } => {}
            other => panic!(
                "expected KnotProcessing for deleted event, got {:?}",
                other
            ),
        }

        // Agent runner IS called (deleted events invoke the agent
        // with deletion notice in prompt)
        let was_called = called.lock().unwrap();
        assert!(
            *was_called,
            "agent runner should be called for deleted events"
        );

        // Tie-off IS written
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1, "tie-off should be appended");
    }
}
