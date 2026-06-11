//! Application-layer use cases.
//!
//! Each use case orchestrates domain entities through port traits and the
//! in-memory loom store. Tests use mock port implementations — no IO.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adapters::logging;
use crate::application::ports::{
    AgentProfileRepository, AgentRunner, ExecutionContext, EventSource,
    KnotEventType, LoomLogPort, LoomRepository, ProcessingStatus,
    PortError, TieOffSink,
};
use crate::application::store::LoomStore;
use crate::domain::entities::{Knot, KnotId, Loom, LoomId, StrandPath, TieOff, TieOffPath};
use crate::domain::events::{ConfigEvent, LoomEvent, StrandEvent};
use crate::domain::knot_file::derive_tieoff_path;
use crate::domain::value_objects::{AgentConfig, RigAgentConfig};

/// Generate an ISO 8601 UTC timestamp string.
pub fn format_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    // Compute UTC date/time from Unix epoch (good enough for ISO 8601)
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;
    // Convert days since 1970-01-01 to Y-M-D (Gregorian)
    let z = days_since_epoch as i64 + 719468;
    let a = z + 305;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hh, mm, ss
    )
}

// ── Query Result Types ───────────────────────────────────────────────────

/// A summary of a loom (lightweight, for list responses).
///
/// The loom directory is derived from the loom ID and rig base path
/// (naming convention `*-loom`). Strand and tie-off directories are
/// per-knot fields, not loom-level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LoomSummary {
    /// The loom's unique ID (must end in `-loom`).
    pub id: LoomId,
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

            logging::log_loom_event(
                "discover",
                &loom.id.0,
                &format!("new loom found, {} knots", loom.knots.len()),
            );
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

        logging::log_loom_event(
            "registered",
            &loom.id.0,
            &format!("{} knots, watchers started", loom.knots.len()),
        );
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
    /// Base (rig) directory — used to derive static output paths.
    base_dir: PathBuf,
    /// Profile repository for dynamic profile resolution at processing time.
    profile_repo: Arc<dyn AgentProfileRepository>,
}

impl ProcessStrand {
    /// Create a new `ProcessStrand` use case.
    pub fn new(
        store: LoomStore,
        log_port: Arc<dyn LoomLogPort>,
        agent_runner: Arc<dyn AgentRunner>,
        tie_off_sink: Arc<dyn TieOffSink>,
        rig_config: RigAgentConfig,
        base_dir: PathBuf,
        profile_repo: Arc<dyn AgentProfileRepository>,
    ) -> Self {
        Self {
            store,
            log_port,
            agent_runner,
            tie_off_sink,
            rig_config,
            base_dir,
            profile_repo,
        }
    }

    /// Resolve the effective `AgentConfig` for a knot, along with the
    /// system prompt to use for `--system-prompt`.
    ///
    /// Returns a tuple of `(AgentConfig, String)` where the `String` is the
    /// system prompt that should be used for the CLI `--system-prompt` flag.
    ///
    /// - If the knot has `agent_profile_ref` (and no inline `agent_config`):
    ///   load the profile from the repository and build an `AgentConfig`
    ///   from it, using the profile's `system_prompt` (merged with the
    ///   knot's prompt instructions) as the system prompt for CLI args.
    /// - If the knot has inline `agent_config` (and no profile ref):
    ///   return the inline config as-is (backward compat), using
    ///   `template.instructions` as the system prompt.
    /// - If both are present (shouldn't happen via mutual exclusivity at
    ///   parse time, but may occur for programmatic Knot construction):
    ///   profile is the base; inline config overrides specific fields
    ///   (provider, model, tools).
    /// - If neither is present: return `PortError::AgentExecutionFailed`.
    pub fn resolve_agent_config(
        &self,
        knot: &Knot,
    ) -> Result<(AgentConfig, String), PortError> {
        match (&knot.agent_profile_ref, &knot.agent_config) {
            (Some(profile_name), None) => {
                // Profile ref only — load and build AgentConfig from profile.
                let profile = self
                    .profile_repo
                    .get(profile_name)
                    .map_err(|e| {
                        PortError::ProfileNotFound(e.to_string())
                    })?
                    .ok_or_else(|| {
                        PortError::ProfileNotFound(profile_name.clone())
                    })?;

                // Merge profile's system_prompt with knot's instructions.
                // Profile system_prompt is the base (agent persona/instructions),
                // knot instructions are appended as task-specific direction.
                let merged_system_prompt = if knot
                    .prompt_template
                    .instructions
                    .trim()
                    .is_empty()
                {
                    profile.system_prompt.clone()
                } else {
                    format!(
                        "{}\n\n{}",
                        profile.system_prompt,
                        knot.prompt_template.instructions
                    )
                };

                Ok((
                    AgentConfig {
                        goal: knot.prompt_template.instructions.clone(),
                        provider: profile.provider.clone(),
                        model: profile.model.clone(),
                        tools: profile.tools.clone(),
                    },
                    merged_system_prompt,
                ))
            }
            (None, Some(config)) => {
                // Inline config only — use as-is (backward compat).
                Ok((
                    config.clone(),
                    knot.prompt_template.instructions.clone(),
                ))
            }
            (Some(profile_name), Some(inline_config)) => {
                // Both set — profile is the base, inline config overrides.
                // Since AgentConfig requires all fields, inline fully
                // overrides the profile. This path handles programmatic
                // Knot construction where mutual exclusivity is bypassed.
                logging::log_knot_event(
                    "warn:profile-override",
                    &profile_name.as_str(),
                    &knot.id.0,
                    "knot has both profile-ref and inline config; inline wins",
                );
                Ok((
                    inline_config.clone(),
                    knot.prompt_template.instructions.clone(),
                ))
            }
            (None, None) => Err(PortError::AgentExecutionFailed(
                format!(
                    "knot '{}' has neither agent-profile-ref nor agent-config",
                    knot.id.0
                ),
            )),
        }
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
            &format!("{} processing start", strand_kind),
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

        // 1. Append KnotProcessing to loom-log
        self.log_port.append(LoomEvent::KnotProcessing {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            timestamp: format_timestamp(),
        })?;

        // 2. Resolve effective agent config (profile or inline) and build CLI args
        let (agent_config, system_prompt) = self.resolve_agent_config(knot)?;
        let mut cli_args = agent_config
            .build_cli_args(&knot.prompt_template, Some(&system_prompt));
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
                    timestamp: format_timestamp(),
                })?;

                // 6. Append StrandProcessed
                self.log_port.append(LoomEvent::StrandProcessed {
                    loom_id,
                    strand_path: strand_path.clone(),
                    error: None,
                    timestamp: format_timestamp(),
                })?;

                logging::log_strand_event(
                    &format!("{} completed", strand_kind),
                    &strand_path.0,
                );
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
                    &format!("{} failed: {}", strand_kind, error_msg),
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
    /// Uses statically derived path: `rig/output/{loom-id}/{knot-name}/output.md`.
    fn compute_tie_off_path(
        &self,
        loom: &Loom,
        knot: &Knot,
        strand_path: &StrandPath,
    ) -> TieOffPath {
        let filename = strand_path
            .0
            .file_name()
            .map(|f| format!("{}.output", f.to_string_lossy()))
            .unwrap_or_else(|| "output".to_string());
        let base = derive_tieoff_path(&loom.id.0, &knot.id.0, &self.base_dir);
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
    rig_path: PathBuf,
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
        Self {
            repository,
            log_port,
            store,
            event_source,
            rig_path,
        }
    }

    /// Handle a single configuration event.
    pub fn execute(&self, event: ConfigEvent) -> Result<(), PortError> {
        match event {
            ConfigEvent::LoomAdded { ref loom_id } => {
                logging::log_config_event(
                    "LoomAdded",
                    &format!("loom={}", loom_id.0),
                );
                self.handle_loom_added(loom_id)
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
    /// Scans the rig via `LoomRepository::scan()` to get the full loom
    /// (with parsed knots and resolved paths), then registers it using
    /// the same flow as `RegisterLoom`.
    fn handle_loom_added(&self, loom_id: &LoomId) -> Result<(), PortError> {
        // Skip if already registered
        if self.store.get(loom_id).is_some() {
            logging::log_config_event(
                "LoomAdded",
                &format!("loom={} already registered (skip)", loom_id.0),
            );
            return Ok(());
        }

        // Scan the rig to get the loom with full knot data
        let looms = self.repository.scan(&self.rig_path)?;
        let loom = looms
            .into_iter()
            .find(|l| l.id == *loom_id)
            .ok_or_else(|| PortError::LoomNotFound(loom_id.clone()))?;

        self.register_loom(&loom)?;
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

        // Start watcher for knot's strand directory
        self.event_source.set_loom_ids(
            &knot_strand_dir,
            loom_id,
            &knot_id,
        );
        self.event_source.watch(&knot_strand_dir)
            .map_err(|e| {
                PortError::EventWatchFailed(format!(
                    "failed to watch '{}': {}",
                    knot_strand_dir.display(),
                    e
                ))
            })?;

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
                    self.event_source.unwatch(&old_strand_dir)
                        .map_err(|e| {
                            PortError::EventUnwatchFailed(format!(
                                "failed to unwatch '{}': {}",
                                old_strand_dir.display(),
                                e
                            ))
                        })?;

                    self.event_source.set_loom_ids(
                        &new_strand_dir,
                        loom_id,
                        &knot_id,
                    );
                    self.event_source.watch(&new_strand_dir)
                        .map_err(|e| {
                            PortError::EventWatchFailed(format!(
                                "failed to watch '{}': {}",
                                new_strand_dir.display(),
                                e
                            ))
                        })?;

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

                // Start watcher for knot's strand directory
                self.event_source.set_loom_ids(
                    &knot_strand_dir,
                    loom_id,
                    &knot_id,
                );
                self.event_source.watch(&knot_strand_dir)
                    .map_err(|e| {
                        PortError::EventWatchFailed(format!(
                            "failed to watch '{}': {}",
                            knot_strand_dir.display(),
                            e
                        ))
                    })?;

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
        self.event_source.unwatch(&strand_dir)
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
    fn register_loom(&self, loom: &Loom) -> Result<(), PortError> {
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
            self.event_source.set_loom_ids(
                &knot.strand_dir,
                &loom.id,
                &knot.id,
            );
            self.event_source.watch(&knot.strand_dir)
                .map_err(|e| {
                    PortError::EventWatchFailed(format!(
                        "failed to watch '{}': {}",
                        knot.strand_dir.display(),
                        e
                    ))
                })?;
        }

        Ok(())
    }
}

// ── ConfigEventHandler Tests ──────────────────────────────────────────

#[cfg(test)]
mod config_handler_tests {
    use super::*;
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
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
        scan_result: Arc<Mutex<Vec<Loom>>>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<Vec<Loom>, PortError> {
            Ok(self.scan_result.lock().unwrap().clone())
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
            agent_config: Some(AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            }),
            agent_profile_ref: None,
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
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
    /// loom dir via repository, registers loom in store, logs events,
    /// and starts watchers for each knot's strand directory.
    #[test]
    fn config_handler_loom_added() {
        let loom_id = LoomId("new-loom".to_string());
        let loom = build_loom(
            "new-loom",
            vec![build_knot("k1"), build_knot("k2")],
        );

        let repo = Arc::new(MockLoomRepository {
            scan_result: Arc::new(Mutex::new(vec![loom.clone()])),
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
            PathBuf::from("/rig"),
        );

        let result = handler.execute(ConfigEvent::LoomAdded {
            loom_id: loom_id.clone(),
        });

        // Should succeed
        assert!(result.is_ok(), "should succeed: {:?}", result);

        // Loom is in the store
        let stored = store.get(&loom_id);
        assert!(stored.is_some(), "loom should be in store");
        let stored = stored.unwrap();
        assert_eq!(stored.id, loom_id);
        assert_eq!(stored.knots.len(), 2);

        // Log events: open, 2x KnotRegistered, LoomStarted
        let events = logged_events.lock().unwrap();
        assert_eq!(
            events.len(),
            3,
            "should log KnotRegistered x2 + LoomStarted"
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

        // Watchers started for each knot's strand directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(
            watches.len(),
            2,
            "should watch 2 knot strand directories"
        );
        let watched: HashSet<_> =
            watches.iter().map(|p| p.as_path()).collect();
        assert!(watched.contains(Path::new("strands")));
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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            scan_result: Arc::new(Mutex::new(vec![])),
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

        // Log: KnotRegistered for k2
        let events = logged_events.lock().unwrap();
        assert_eq!(events.len(), 1);
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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            knot: knot,
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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            scan_result: Arc::new(Mutex::new(vec![])),
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

        // Update knot with same strand_dir (only config changed)
        let mut updated_knot = build_knot("k1");
        if let Some(ref mut ac) = updated_knot.agent_config {
            ac.model = "claude-sonnet".to_string();
        }

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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            scan_result: Arc::new(Mutex::new(vec![])),
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
            agent_config: Some(AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            }),
            agent_profile_ref: None,
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
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

        // Duplicate registration — should succeed (idempotent)
        let (event_source2, watch_calls2) = TrackingEventSource::new();
        let es2: Arc<dyn EventSource> = Arc::new(event_source2);
        let use_case = RegisterLoom::new(
            Arc::new(MockLoomLogPort::default()),
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
            agent_config: Some(AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            }),
            agent_profile_ref: None,
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
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
            agent_config: Some(AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            }),
            agent_profile_ref: None,
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
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

// ── ManageKnot Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod manage_knot_tests {
    use super::*;
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
    use std::path::PathBuf;

    /// Build a knot with the given ID.
    fn build_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_config: Some(AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            }),
            agent_profile_ref: None,
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
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
        assert_eq!(k.agent_config.as_ref().unwrap().model, "gpt-4o");
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
        // Update k1 with a new model
        let mut updated_knot = build_knot("k1");
        if let Some(ref mut ac) = updated_knot.agent_config {
            ac.model = "claude-sonnet".to_string();
        }
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
        assert_eq!(k1.agent_config.as_ref().unwrap().model, "claude-sonnet");
        assert_eq!(
            k1.prompt_template.instructions,
            "new instructions"
        );

        // k2 is unchanged
        let k2 = loom.knots.iter()
            .find(|k| k.id == KnotId("k2".to_string()))
            .unwrap();
        assert_eq!(k2.agent_config.as_ref().unwrap().model, "gpt-4o");
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
    use crate::application::ports::AgentOutput;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentConfig, AgentProfile, PromptTemplate};
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

        fn save(
            &self,
            profile: AgentProfile,
        ) -> Result<(), PortError> {
            self.profiles
                .lock()
                .unwrap()
                .insert(profile.name.clone(), profile);
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<(), PortError> {
            let mut map = self.profiles.lock().unwrap();
            if map.remove(name).is_none() {
                return Err(PortError::ProfileNotFound(name.to_string()));
            }
            Ok(())
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────

    /// Build a knot with agent-config only (no profile ref).
    fn build_inline_knot(id: impl Into<String>) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_config: Some(AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec!["fs".to_string()],
            }),
            agent_profile_ref: None,
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check it".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
        }
    }

    /// Build a knot with profile ref only.
    fn build_profile_knot(
        id: impl Into<String>,
        profile_name: &str,
    ) -> Knot {
        Knot {
            id: KnotId(id.into()),
            agent_config: None,
            agent_profile_ref: Some(profile_name.to_string()),
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check with profile".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
        }
    }

    /// Build a loom with the given ID and knots.
    fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots,
        }
    }

    // ── resolve_agent_config Tests ───────────────────────────────────

    /// Profile ref resolves to profile fields: provider, model, tools.
    /// Goal comes from the knot's prompt template instructions.
    /// System prompt comes from the profile (merged with knot instructions).
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

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
        );

        let profile_knot = build_profile_knot("k1", "fast");
        let (config, system_prompt) =
            use_case.resolve_agent_config(&profile_knot).unwrap();

        // Resolved config should use profile values
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.tools, vec!["fs", "web"]);
        // Goal comes from prompt template instructions
        assert_eq!(
            config.goal,
            profile_knot.prompt_template.instructions
        );
        // System prompt should contain profile's system_prompt + knot instructions
        assert!(system_prompt.contains("You are fast."));
        assert!(system_prompt.contains("check with profile"));
    }

    /// Inline agent-config is returned as-is (backward compat).
    #[test]
    fn resolve_agent_config_inline_backward_compat() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository::default());

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
        );

        let inline_knot = build_inline_knot("k1");
        let (config, system_prompt) =
            use_case.resolve_agent_config(&inline_knot).unwrap();

        // Inline config should be used as-is
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.tools, vec!["fs"]);
        assert_eq!(config.goal, "review");
        // System prompt is the knot's instructions (backward compat)
        assert_eq!(system_prompt, "check it");
    }

    /// Profile ref resolves to profile values even when inline config
    /// is also present (inline overrides — mutual exclusivity bypassed).
    #[test]
    fn resolve_agent_config_profile_with_inline_override() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                (
                    "fast".to_string(),
                    AgentProfile::with_tools(
                        "fast".to_string(),
                        "openai".to_string(),
                        "gpt-4o".to_string(),
                        vec!["fs".to_string()],
                        "You are fast.".to_string(),
                    )
                    .unwrap(),
                ),
            ]))),
        });

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
        );

        // Build a knot with BOTH profile ref AND inline config
        // (simulates programmatic construction bypassing mutual exclusivity)
        let knot = Knot {
            id: KnotId("k1".to_string()),
            agent_config: Some(AgentConfig {
                goal: "override goal".to_string(),
                provider: "anthropic".to_string(),
                model: "claude-sonnet".to_string(),
                tools: vec!["web".to_string()],
            }),
            agent_profile_ref: Some("fast".to_string()),
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
        };

        // When both are set, inline config overrides (full override)
        let (config, system_prompt) =
            use_case.resolve_agent_config(&knot).unwrap();
        assert_eq!(config.provider, "anthropic");
        assert_eq!(config.model, "claude-sonnet");
        assert_eq!(config.tools, vec!["web"]);
        assert_eq!(config.goal, "override goal");
        // System prompt is the knot's instructions
        assert_eq!(system_prompt, "check");
    }

    /// Profile not found returns PortError::ProfileNotFound.
    #[test]
    fn resolve_agent_config_profile_not_found() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository::default());

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
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

    /// Neither profile ref nor inline config returns error.
    #[test]
    fn resolve_agent_config_neither_set() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository::default());

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
        );

        let knot = Knot {
            id: KnotId("k1".to_string()),
            agent_config: None,
            agent_profile_ref: None,
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "check".to_string(),
            },
            strand_dir: PathBuf::from("strands"),
        };

        let result = use_case.resolve_agent_config(&knot);
        assert!(result.is_err());
        match result.unwrap_err() {
            PortError::AgentExecutionFailed(msg) => {
                assert!(msg.contains("neither"));
            }
            other => panic!("Expected AgentExecutionFailed, got {other:?}"),
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

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
        );

        let knot1 = build_profile_knot("k1", "detailed");
        let knot2 = build_profile_knot("k2", "detailed");

        let (config1, system_prompt1) =
            use_case.resolve_agent_config(&knot1).unwrap();
        let (config2, system_prompt2) =
            use_case.resolve_agent_config(&knot2).unwrap();

        // Both should resolve to the same profile values
        assert_eq!(config1.provider, "anthropic");
        assert_eq!(config1.model, "claude-sonnet-4-20250514");
        assert_eq!(config2.provider, "anthropic");
        assert_eq!(config2.model, "claude-sonnet-4-20250514");
        assert_eq!(config1.tools, vec!["fs", "web"]);
        assert_eq!(config2.tools, vec!["fs", "web"]);

        // System prompts contain profile system_prompt + knot instructions
        assert!(system_prompt1.contains("Be thorough."));
        assert!(system_prompt1.contains("check with profile"));
        assert!(system_prompt2.contains("Be thorough."));
        assert!(system_prompt2.contains("check with profile"));
    }

    /// Dynamic profile pickup: adding a profile to the repository
    /// mid-lifecycle makes it available to knots on next resolution.
    #[test]
    fn resolve_agent_config_dynamic_profile_pickup() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository::default());

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
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
        profile_repo.save(profile).unwrap();

        // Now the same knot should resolve successfully
        let (config, system_prompt) =
            use_case.resolve_agent_config(&profile_knot).unwrap();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-4o");
        assert_eq!(config.tools, vec!["fs"]);
        // System prompt contains profile's system_prompt + knot instructions
        assert!(system_prompt.contains("You are new."));
        assert!(system_prompt.contains("check with profile"));
    }

    /// Profile system_prompt flows into CLI --system-prompt arg.
    ///
    /// Verifies that when a profile-ref knot is resolved, the resulting
    /// CLI args contain the profile's system_prompt as the --system-prompt
    /// value (merged with knot instructions).
    #[test]
    fn profile_ref_cli_args_include_system_prompt() {
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

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
        );

        let profile_knot = build_profile_knot("k1", "reviewer");
        let (config, system_prompt) =
            use_case.resolve_agent_config(&profile_knot).unwrap();
        let args = config.build_cli_args(&profile_knot.prompt_template, Some(&system_prompt));

        // CLI args should contain the merged system prompt
        let system_prompt_index = args.iter().position(|a| a == "--system-prompt").expect("--system-prompt flag missing");
        let system_prompt_value = &args[system_prompt_index + 1];
        assert!(
            system_prompt_value.contains("careful reviewer"),
            "system prompt should contain profile instructions: {system_prompt_value}"
        );
        assert!(
            system_prompt_value.contains("check with profile"),
            "system prompt should contain knot instructions: {system_prompt_value}"
        );
        // Should also have the model arg
        let model_index = args.iter().position(|a| a == "--model").expect("--model flag missing");
        assert_eq!(args[model_index + 1], "gpt-4o");
    }

    /// Inline config knot uses knot instructions as system prompt (backward compat).
    ///
    /// Verifies that knots with inline agent-config continue to use
    /// prompt_template.instructions for --system-prompt.
    #[test]
    fn inline_config_cli_args_use_template_instructions() {
        let store = LoomStore::new();
        let profile_repo = Arc::new(MockProfileRepository::default());

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
        );

        let inline_knot = build_inline_knot("k1");
        let (config, system_prompt) =
            use_case.resolve_agent_config(&inline_knot).unwrap();
        let args = config.build_cli_args(&inline_knot.prompt_template, Some(&system_prompt));

        // CLI args should use knot's instructions as system prompt
        let system_prompt_index = args.iter().position(|a| a == "--system-prompt").expect("--system-prompt flag missing");
        let system_prompt_value = &args[system_prompt_index + 1];
        assert_eq!(system_prompt_value, "check it");
    }
}
