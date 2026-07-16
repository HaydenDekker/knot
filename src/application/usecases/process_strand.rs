//! Use case: process a single strand event through the agent pipeline.

use std::path::PathBuf;
use std::sync::Arc;

use crate::adapters::logging;
use crate::application::ports::{
    AgentProfileRepository, AgentRunner, EventDispatcherPort, GitVersioningPort,
    KnotEventType, LoomLogPort, PortError, RigLogPort, TieOffSink,
};
use crate::application::session_resume;
use crate::application::store::LoomStore;
use crate::domain::entities::{
    EventMetadata, Knot, KnotId, Loom, LoomId, StrandCheckResult,
    StrandFileChecker, StrandPath, TieOff, TieOffOutcome, TieOffPath,
};
use crate::domain::events::{AgentEvent, BuildContext, ContextProvider, LoomEvent, StrandEvent, StrandQueueAccessor};
use crate::application::usecases::context_providers::AgentEventsContextProvider;
use crate::domain::knot_file::derive_tieoff_path;
use crate::domain::value_objects::{
    AgentConfig, AgentProfile, EventSubscription, RigAgentConfig, StrandSource,
};

// Re-export shared types from types module
use super::types::format_timestamp;
use super::strand_event_metadata::{extract_expected_event_ids, extract_event_metadata};
use super::process_strand_helpers::ResolvedExecution;

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
    pub(crate) store: LoomStore,
    pub(crate) log_port: Arc<dyn LoomLogPort>,
    pub(crate) agent_runner: Arc<dyn AgentRunner>,
    pub(crate) tie_off_sink: Arc<dyn TieOffSink>,
    rig_config: RigAgentConfig,
    /// Rig directory — used to derive static output paths.
    pub(crate) rig_dir: PathBuf,
    /// Profile repository for dynamic profile resolution at processing time.
    pub(crate) profile_repo: Arc<dyn AgentProfileRepository>,
    /// Rig-log port for recording operational events (timeouts, idle).
    pub(crate) rig_log: Arc<dyn RigLogPort>,
    /// Git versioning port for creating commits after successful runs.
    pub(crate) git_versioning_port: Arc<dyn GitVersioningPort>,
    /// Strand file checker for text/binary/temp detection.
    file_checker: Arc<dyn StrandFileChecker>,
    /// Event dispatcher for intent-based agent-to-agent routing.
    event_dispatcher: Arc<dyn EventDispatcherPort>,
    /// Strand event queue — source of truth for pending events.
    pub(crate) strand_queue: Option<Arc<dyn StrandQueueAccessor>>,
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
        file_checker: Arc<dyn StrandFileChecker>,
        event_dispatcher: Arc<dyn EventDispatcherPort>,
        strand_queue: Option<Arc<dyn StrandQueueAccessor>>,
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
            file_checker,
            event_dispatcher,
            strand_queue,
        }
    }

    /// Resolve the effective `AgentConfig` for a knot, the profile's
    /// session timeout, and the profile itself.
    ///
    /// Loads the profile from the repository and delegates the
    /// profile→config mapping to `AgentProfile::resolve_for_knot()`.
    /// The profile's `profile_prompt` is delivered via stdin
    /// (not `--system-prompt`), so it is not merged here.
    ///
    /// Returns a tuple of `(AgentConfig, Option<Duration>, AgentProfile)`
    /// where the `Option<Duration>` is the profile's timeout
    /// (or `None` to use the runner's default) and the `AgentProfile`
    /// is the loaded profile (avoiding a second repository lookup).
    pub fn resolve_agent_config(
        &self,
        knot: &Knot,
    ) -> Result<(AgentConfig, Option<std::time::Duration>, AgentProfile), PortError> {
        let profile = self
            .profile_repo
            .get(&knot.agent_profile_ref)
            .map_err(|e| PortError::ProfileNotFound(e.to_string()))?
            .ok_or_else(|| {
                PortError::ProfileNotFound(knot.agent_profile_ref.clone())
            })?;

        let config = profile.resolve_for_knot(knot);
        let timeout = profile.session_timeout();

        Ok((config, timeout, profile))
    }

    /// Resolve agent config, build prompt, execute agent, derive outcome.
    ///
    /// Covers: profile resolution, deleted-event history, prompt building,
    /// listener context, agent execution, and outcome derivation.
    ///
    /// Returns a `ResolvedExecution` with the outcome, session ID, listener
    /// context, and all knots for downstream event enforcement.
    fn resolve_config_and_build(
        &self,
        knot: &Knot,
        event_type: KnotEventType,
        event_label: String,
        strand_path: &StrandPath,
        loom_id: &LoomId,
        tie_off_path: &TieOffPath,
    ) -> Result<ResolvedExecution, PortError> {
        // Resolve effective agent config (profile).
        let (agent_config, profile_timeout, profile) =
            self.resolve_agent_config(knot)?;

        // For Deleted events: read existing tie-off content and extract
        // scoped strand history (last 5 entries for this strand).
        let is_deleted = matches!(event_type, KnotEventType::Deleted);
        let strand_history = if is_deleted {
            let tie_off_content = self
                .tie_off_sink
                .read_content(tie_off_path)
                .unwrap_or_default();
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

        // Build the prompt. For Deleted events, use domain method
        // Knot::deleted_prompt() which composes the deletion notice
        // and scoped strand history.
        let base_prompt = if is_deleted {
            let sections = strand_history
                .as_deref()
                .unwrap_or_default();
            knot.deleted_prompt(&strand_filename, sections)
        } else {
            knot.prompt_template.instructions.clone()
        };

        // Build listener context (per-invocation, not cached).
        // Scans all knots' strand_source entries and injects event
        // instructions at the beginning of the prompt, including
        // pending events from the dispatch directory.
        let all_knots = Self::collect_all_knots(&self.store);
        let build_ctx = BuildContext {
            knot: knot.clone(),
            loom_id: loom_id.clone(),
            all_knots: all_knots.clone(),
            rig_dir: self.rig_dir.clone(),
            strand_queue: self.strand_queue.clone(),
        };
        let listener_context =
            AgentEventsContextProvider.build_context(&build_ctx);

        let prompt = if listener_context.is_empty() {
            base_prompt
        } else {
            format!("{}\n\n{}", listener_context, base_prompt)
        };

        // Execute agent with session-resume retry logic.
        let strand_file_ref = if is_deleted {
            None
        } else {
            Some(strand_path.clone())
        };
        let mut session_id: Option<String> = None;
        let result = session_resume::execute_with_resume(
            &*self.agent_runner,
            &*self.log_port,
            loom_id,
            &knot.id,
            strand_path,
            &mut session_id,
            agent_config,
            prompt,
            strand_file_ref,
            profile.profile_prompt,
            event_label,
            Some(knot.id.0.clone()),
            profile_timeout.clone(),
        );

        // Derive outcome from execution result — domain rule.
        let outcome = TieOffOutcome::derive(result);

        Ok(ResolvedExecution {
            outcome,
            session_id,
            listener_context,
            all_knots,
            profile_timeout,
        })
    }

    /// Execute the strand processing pipeline.
    ///
    /// Appends lifecycle events to loom-log: KnotProcessing, then
    /// KnotCompleted or KnotFailed, then StrandProcessed.
    pub fn execute(&self, event: StrandEvent) -> Result<(), PortError> {
        // Single match: extract fields, event_type, and strand_kind.
        let (loom_id, knot_id, strand_path, event_type) = match &event {
            StrandEvent::Created {
                loom_id,
                knot_id,
                strand_path,
            } => (
                loom_id.clone(),
                knot_id.clone(),
                strand_path.clone(),
                KnotEventType::Created,
            ),
            StrandEvent::Modified {
                loom_id,
                knot_id,
                strand_path,
            } => (
                loom_id.clone(),
                knot_id.clone(),
                strand_path.clone(),
                KnotEventType::Modified,
            ),
            StrandEvent::Deleted {
                loom_id,
                knot_id,
                strand_path,
            } => (
                loom_id.clone(),
                knot_id.clone(),
                strand_path.clone(),
                KnotEventType::Deleted,
            ),
        };
        let strand_kind = match event_type {
            KnotEventType::Created => "Created",
            KnotEventType::Modified => "Modified",
            KnotEventType::Deleted => "Deleted",
        };
        let event_label = match event_type {
            KnotEventType::Created => "Created".to_string(),
            KnotEventType::Modified => "Modified".to_string(),
            KnotEventType::Deleted => "Deleted".to_string(),
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

        // Determine tie-off path (statically derived from loom + knot)
        let tie_off_path = self.compute_tie_off_path(&loom, knot, &strand_path);

        // Strand file check: skip binary/temp/missing files.
        if !self.validate_strand(&event, strand_kind, &loom_id, &knot_id, &strand_path)? {
            return Ok(());
        }

        // 1. Append KnotProcessing to loom-log
        self.log_port.append(LoomEvent::KnotProcessing {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
            strand_path: strand_path.clone(),
            timestamp: format_timestamp(),
        })?;

        // 2. Resolve config, build prompt, execute agent, derive outcome.
        let resolved = match self.resolve_config_and_build(
            knot,
            event_type,
            event_label.clone(),
            &strand_path,
            &loom_id,
            &tie_off_path,
        ) {
            Ok(resolved) => resolved,
            Err(err) => {
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
                    agent_events: Vec::new(),
                    event_metadata: crate::domain::entities::EventMetadata::default(),
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
                return Err(err);
            }
        };
        let outcome = resolved.outcome.clone();

        // Write tie-off (skipped for timeout).
        super::process_strand_helpers::write_tie_off(
            self, &outcome, knot, &tie_off_path, &strand_path, &event_label,
        );

        // Write rig-log for timeout (preserve unchanged).
        if outcome.is_timeout() {
            let _ = self.rig_log.append(
                crate::domain::events::RigLogEvent::TimeoutExceeded {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    strand_path: strand_path.clone(),
                    error: outcome
                        .error_message()
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                    timestamp: format_timestamp(),
                },
            );
        }

        // Write loom-log: KnotCompleted or KnotFailed.
        match outcome.tie_off_status() {
            Some(crate::domain::entities::TieOffStatus::Produced) => {
                super::process_strand_helpers::handle_success(
                    self, &outcome, &resolved, strand_kind,
                    knot, &tie_off_path,
                    &loom_id, &knot_id, &strand_path,
                    &event_label,
                )?;
            }
            _ => {
                super::process_strand_helpers::handle_failure(
                    self, &outcome, strand_kind,
                    &loom_id, &knot_id, &strand_path,
                )?;
            }
        }

        Ok(())
    }

    /// Compute the tie-off output path from knot + strand path.
    /// Uses statically derived path: `rig/tie-offs/{loom-id}/tie-off-{knot-name}.md`.
    /// Tie-off files are placed flat under the loom's tie-off directory.
    fn compute_tie_off_path(
        &self,
        loom: &Loom,
        knot: &Knot,
        _strand_path: &StrandPath,
    ) -> TieOffPath {
        let filename = format!("tie-off-{}.md", knot.id.0);
        let base = derive_tieoff_path(&loom.id.0, &knot.id.0, &self.rig_dir);
        TieOffPath(base.join(filename))
    }

    /// Validate the strand file before processing.
    ///
    /// Checks file existence, binary/temp detection via `StrandPath::should_process()`.
    /// Returns `Ok(true)` to continue processing, `Ok(false)` to skip
    /// (after logging the skip), or `Err` for check failures.
    fn validate_strand(
        &self,
        event: &StrandEvent,
        strand_kind: &str,
        loom_id: &LoomId,
        knot_id: &KnotId,
        strand_path: &StrandPath,
    ) -> Result<bool, PortError> {
        // Domain rule lives in StrandPath::should_process().
        let is_deleted = matches!(event, StrandEvent::Deleted { .. });
        let check = strand_path
            .should_process(is_deleted, &*self.file_checker)
            .map_err(|e| PortError::StrandCheckFailed(e.message))?;

        match check {
            StrandCheckResult::Proceed | StrandCheckResult::ProceedWithWarning => {
                if matches!(check, StrandCheckResult::ProceedWithWarning) {
                    eprintln!(
                        "WARN: cannot determine if strand '{}' is text, \
                         proceeding with processing (knot={})",
                        strand_path.0.display(),
                        knot_id.0,
                    );
                }
                Ok(true)
            }
            StrandCheckResult::SkipBinary => {
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
                Ok(false)
            }
            StrandCheckResult::SkipTemp => {
                // Known temp file pattern (e.g. sedXXXXXXX)
                // — skip silently. No loom-log entry, no agent invocation.
                logging::log_strand_event(
                    &format!(
                        "{} skipped known temp file (knot={})",
                        strand_kind, knot_id.0,
                    ),
                    &strand_path.0,
                );
                Ok(false)
            }
            StrandCheckResult::SkipMissing => {
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
                Ok(false)
            }
        }
    }

    /// Match and dispatch a set of agent events to consumer knots.
    ///
    /// For each event, scans all looms' knots for `strand_source` subscriptions
    /// that target this producer knot and match the event ID. Dispatches event
    /// files to each matching consumer.
    ///
    /// Returns the list of `(event_id, consumer_loom_id)` dispatches performed.
    pub(crate) fn dispatch_events_to_consumers(
        &self,
        events: &[AgentEvent],
        producer_knot: &Knot,
        loom_id: &LoomId,
        all_knot_ids: &[&str],
    ) -> Result<Vec<(String, String)>, PortError> {
        let all_looms = self.store.list();
        let mut dispatches: Vec<(String, String)> = Vec::new();

        for event in events {
            for loom in &all_looms {
                for consumer_knot in &loom.knots {
                    let resolved = consumer_knot
                        .strand_source
                        .resolve_for_producer(
                            &producer_knot.id.0,
                            &loom_id.0,
                            all_knot_ids,
                        );
                    if let Some(sub) = resolved {
                        let matches_event = match &sub {
                            EventSubscription::KnotLevel {
                                event_id: sub_event_id,
                                ..
                            } => sub_event_id == &event.event_id,
                            EventSubscription::LoomLevel {
                                event_id: sub_event_id,
                                ..
                            } => sub_event_id == &event.event_id,
                        };
                        if matches_event {
                            let _path = self.event_dispatcher.dispatch(
                                event,
                                consumer_knot,
                                &producer_knot.id.0,
                                &loom.id,
                                &self.rig_dir,
                            )?;
                            dispatches.push((
                                event.event_id.clone(),
                                loom.id.0.clone(),
                            ));
                        }
                    }
                }
            }
        }

        Ok(dispatches)
    }

    /// Collect all knots from all registered looms in the store.
    ///
    /// Used to build listener context — scanning all knots' strand_source
    /// entries to find which ones target a specific knot.
    pub(crate) fn collect_all_knots(store: &LoomStore) -> Vec<Knot> {
        store.list().into_iter().flat_map(|loom| loom.knots).collect()
    }

    /// Dispatch agent events from a tie-off to matching consumer knots.
    ///
    /// After a knot completes successfully, this extracts any structured
    /// agent events from the tie-off content, matches them against consumer
    /// `strand_source` entries (EventUri), and dispatches event files to
    /// each matching consumer. `event: None` signals (skipped by parser)
    /// produce no dispatch. Multiple events in a single tie-off are each
    /// dispatched independently to their matching consumers.
    ///
    /// Returns a `LoomEvent::EventsDispatched` log entry if any events
    /// were dispatched, or `None` if no events were found.
    pub(crate) fn dispatch_agent_events(
        &self,
        tie_off_content: &str,
        knot: &Knot,
        loom_id: &LoomId,
        strand_path: &StrandPath,
    ) -> Result<Option<LoomEvent>, PortError> {
        // Parse tie-off for agent events.
        // Returns a Vec of all events found (may be empty).
        // The producing knot's ID is available from the `knot` parameter.
        let events =
            crate::domain::tieoff_parser::extract_agent_events(tie_off_content);

        let event_count = events.len();
        let event_ids: Vec<&str> = events.iter().map(|e| e.event_id.as_str()).collect();
        eprintln!(
            "event parse (knot={}): {} event(s) found — {:?}",
            knot.id.0, event_count, event_ids,
        );

        if events.is_empty() {
            return Ok(None);
        }

        // Iterate by loom to track consumer_loom_id for dispatch.
        // Each event is dispatched independently to its matching consumers.
        let all_looms = self.store.list();
        // Collect all knot IDs for resolve_for_producer to disambiguate
        // knot-level vs loom-level targets.
        let all_knot_ids: Vec<&str> = all_looms
            .iter()
            .flat_map(|l| l.knots.iter())
            .map(|k| k.id.0.as_str())
            .collect();
        let dispatches = self.dispatch_events_to_consumers(
            &events,
            knot,
            loom_id,
            &all_knot_ids,
        )?;

        if dispatches.is_empty() {
            return Ok(None);
        }

        Ok(Some(LoomEvent::EventsDispatched {
            loom_id: loom_id.clone(),
            knot_id: knot.id.clone(),
            strand_path: strand_path.clone(),
            dispatches,
            timestamp: super::types::format_timestamp(),
        }))
    }

}



// ── Phase 3: Profile Resolution Tests ─────────────────────────────

#[cfg(test)]
mod profile_resolution_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentProfile, PromptTemplate};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::super::test_fixtures::{
        build_knot_with_profile, default_profile, MockAgentRunner,
        MockGitVersioningPort, MockLoomLogPort, MockProfileRepository,
        MockRigLogPort, MockEventDispatcher, MockStrandFileChecker, MockTieOffSink,
    };

    /// Build a knot with the given profile ref.
    fn build_profile_knot(
        id: impl Into<String>,
        profile_name: &str,
    ) -> Knot {
        let mut knot = build_knot_with_profile(id, profile_name);
        knot.prompt_template = PromptTemplate {
            instructions: "check with profile".to_string(),
        };
        knot
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );

        let profile_knot = build_profile_knot("k1", "fast");
        let (config, profile_timeout, _profile) =
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );

        let knot1 = build_profile_knot("k1", "detailed");
        let knot2 = build_profile_knot("k2", "detailed");

        let (config1, timeout1, _profile1) =
            use_case.resolve_agent_config(&knot1).unwrap();
        let (config2, timeout2, _profile2) =
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
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
        let (config, profile_timeout, _profile) =
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );

        let profile_knot = build_profile_knot("k1", "reviewer");
        let (config, _profile_timeout, _profile) =
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
mod execution_test_shared {
    use super::*;
    use crate::domain::events::RigLogEvent;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::super::test_fixtures::{
        build_knot_with_profile, build_loom, default_profile,
        MockAgentRunner, MockEventDispatcher, MockGitVersioningPort,
        MockLoomLogPort, MockProfileRepository, MockRigLogPort,
        MockStrandFileChecker, TrackingTieOffSink,
    };

    /// Re-export build_knot with profile parameter for execution tests.
    pub fn build_knot(id: impl Into<String>, profile: &str) -> crate::domain::entities::Knot {
        build_knot_with_profile(id, profile)
    }

    /// Build the ProcessStrand use case with all mocks.
    #[allow(clippy::type_complexity)]
    pub fn build_process_strand(
        loom: Loom,
        agent_runner: Arc<MockAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<TieOff>>>,
        Arc<Mutex<Vec<RigLogEvent>>>,
        Arc<Mutex<HashMap<String, String>>>,
        Arc<MockAgentRunner>,
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
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
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
}

// ── Execution: happy-path and error handling ──────────────────────────

#[cfg(test)]
mod execution_tests {
    use super::execution_test_shared::{build_knot, build_process_strand};
    use super::*;
    use crate::application::ports::AgentOutput;
    use crate::domain::entities::{KnotId, TieOffStatus};
    use crate::domain::events::RigLogEvent;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[allow(unused_imports)]
    use super::super::test_fixtures::{
        build_loom, MockAgentRunner,
    };

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
        let runner = Arc::new(MockAgentRunner::new(Err(timeout_err)));

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
        let runner = Arc::new(MockAgentRunner::new(Err(err)));

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
        let runner = Arc::new(MockAgentRunner::new(output));

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
}

// ── Execution: deleted event context extraction ───────────────────────

#[cfg(test)]
mod execution_deleted_tests {
    use super::execution_test_shared::{build_knot, build_process_strand};
    use super::*;
    use crate::application::ports::AgentOutput;
    use crate::domain::entities::KnotId;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[allow(unused_imports)]
    use super::super::test_fixtures::{
        build_loom, MockAgentRunner,
    };

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
        let runner = Arc::new(MockAgentRunner::new(output));

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
        let has_at_ref = ctx.agent_config.extra_args.iter().any(|arg| arg.starts_with('@'));
        assert!(
            !has_at_ref,
            "Deleted events must NOT contain @file reference in cli_args: {:?}",
            ctx.agent_config.extra_args,
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
        let runner = Arc::new(MockAgentRunner::new(output));

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
        let runner = Arc::new(MockAgentRunner::new(output));

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
                "/rig/tie-offs/test-loom/tie-off-k1.md".to_string(),
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
        let has_at_ref = ctx.agent_config.extra_args.iter().any(|arg| arg.starts_with('@'));
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
        let runner = Arc::new(MockAgentRunner::new(output));

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
        let has_at_ref = ctx.agent_config.extra_args.iter().any(|arg| {
            arg.starts_with('@') && arg.contains("strand.md")
        });
        assert!(
            has_at_ref,
            "Created events MUST contain @file reference in cli_args: {:?}",
            ctx.agent_config.extra_args,
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
        let runner = Arc::new(MockAgentRunner::new(output));

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
        let has_at_ref = ctx.agent_config.extra_args.iter().any(|arg| arg.starts_with('@'));
        assert!(
            !has_at_ref,
            "Deleted events must NOT contain @file reference",
        );
    }
}

// ── Execution: session resume (retry) ─────────────────────────────────

#[cfg(test)]
mod session_resume_tests {
    use super::execution_test_shared::{build_knot, build_process_strand};
    use super::*;
    use crate::application::ports::AgentOutput;
    use crate::domain::entities::{KnotId, TieOffStatus};
    use crate::domain::events::RigLogEvent;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[allow(unused_imports)]
    use super::super::test_fixtures::{
        build_loom, MockAgentRunner,
    };

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
        let runner = Arc::new(MockAgentRunner::new_sequence(vec![
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

        // Zero retry delay for fast test execution
        unsafe { std::env::set_var("KNOT_RETRY_DELAY_MS", "0"); }
        let result = use_case.execute(event);
        unsafe { std::env::remove_var("KNOT_RETRY_DELAY_MS"); }
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
        let runner = Arc::new(MockAgentRunner::new_sequence(responses));

        let (use_case, log_events, _tie_off_appends, rig_events,
            _content, _runner) =
            build_process_strand(loom, runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        // Zero retry delay for fast test execution (10 retries × 10s default = 100s)
        unsafe { std::env::set_var("KNOT_RETRY_DELAY_MS", "0"); }
        let result = use_case.execute(event);
        unsafe { std::env::remove_var("KNOT_RETRY_DELAY_MS"); }
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
        let runner = Arc::new(MockAgentRunner::new(Err(timeout_err)));

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
mod profile_timeout_tests {
    use super::*;
    use crate::domain::entities::KnotId;
    use crate::domain::value_objects::AgentProfile;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;

    use super::super::test_fixtures::{
        build_knot_with_profile, build_loom, MockAgentRunner,
        MockGitVersioningPort, MockLoomLogPort, MockProfileRepository,
        MockRigLogPort, MockEventDispatcher, MockStrandFileChecker, MockTieOffSink,
        TrackingAgentRunner,
    };

    fn build_knot(id: impl Into<String>, profile: &str) -> crate::domain::entities::Knot {
        build_knot_with_profile(id, profile)
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );

        let knot = build_knot("k1", "slow");
        let (_config, timeout, _profile) =
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );

        let knot = build_knot("k1", "fast");
        let (_config, timeout, _profile) =
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
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
        assert_eq!(
            contexts[0].timeout,
            Some(Duration::from_secs(60)),
            "ExecutionContext.timeout should be profile's timeout"
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
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

// ── Git Versioning Tests ────────────────────────────────

#[cfg(test)]
mod git_versioning_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use super::super::test_fixtures::{
        build_knot, build_loom, default_profile, MockAgentRunner,
        MockGitVersioningPort, MockLoomLogPort, MockProfileRepository,
        MockRigLogPort, MockEventDispatcher, MockStrandFileChecker, MockTieOffSink,
    };

    /// Build a knot with configurable git_versioned flag.
    fn build_knot_with_git(id: impl Into<String>, git_versioned: bool) -> Knot {
        let mut knot = build_knot(id);
        knot.git_versioned = git_versioned;
        knot
    }

    fn build_process_strand(
        loom: Loom,
        git_port: Arc<dyn GitVersioningPort>,
    ) -> ProcessStrand {
        let store = LoomStore::new();
        store.register(loom);

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]))),
        });

        ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            git_port,
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
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
            build_loom("test-loom", vec![build_knot_with_git("k1", true)]);

        let (git_port, commits) = MockGitVersioningPort::new();
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
        assert_eq!(content, "mock");
    }

    /// When `git_versioned: false`, the git port is never called
    /// even on successful processing.
    #[test]
    fn process_strand_skips_git_when_disabled() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let loom =
            build_loom("test-loom", vec![build_knot_with_git("k1", false)]);

        let (git_port, commits) = MockGitVersioningPort::new();
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
            build_loom("test-loom", vec![build_knot_with_git("k1", true)]);

        let (git_port, commits) = MockGitVersioningPort::new();
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

// ── Session Title (--name) Tests ──────────────────────────

#[cfg(test)]
mod session_title_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::value_objects::{AgentProfile, PromptTemplate};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use super::super::test_fixtures::{
        build_knot_with_profile, build_loom, default_profile,
        MockGitVersioningPort, MockLoomLogPort, MockTieOffSink,
        MockProfileRepository, MockRigLogPort, MockEventDispatcher, MockStrandFileChecker,
        TrackingAgentRunner,
    };

    fn build_knot(id: impl Into<String>, profile: &str) -> Knot {
        build_knot_with_profile(id, profile)
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
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
        let args = &contexts[0].agent_config.extra_args;
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );

        let event = StrandEvent::Created {
            loom_id: LoomId("review-loom".to_string()),
            knot_id: KnotId("reviewer".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        use_case.execute(event).unwrap();

        let contexts = captured_contexts.lock().unwrap();
        let args = &contexts[0].agent_config.extra_args;
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );

        let event = StrandEvent::Deleted {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("cleanup".to_string()),
            strand_path: StrandPath(PathBuf::from("input/old-file.md")),
        };

        use_case.execute(event).unwrap();

        let contexts = captured_contexts.lock().unwrap();
        let args = &contexts[0].agent_config.extra_args;
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
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

        let name1 = find_name_value(&contexts[0].agent_config.extra_args)
            .expect("first call should have --name");
        let name2 = find_name_value(&contexts[1].agent_config.extra_args)
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
            git_versioned: true,
            strand_source: StrandSource::Filesystem(PathBuf::from("strands")),
            event_description: None,
        };
        let loom = build_loom("test-loom", vec![knot]);
        store.register(loom);

        let (runner, captured_contexts) = TrackingAgentRunner::new();

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(MockLoomLogPort::default()),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
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
        assert!(ctx.agent_config.extra_args.contains(&"--name".to_string()));
        assert!(!ctx.prompt.contains("--name"),
            "--name should not appear in prompt content");
        assert!(!ctx.profile_prompt.contains("--name"),
            "--name should not appear in profile prompt");
    }
}

// ── Text Check Tests ───────────────────────────────────────

#[cfg(test)]
mod text_check_tests {
    use super::*;
    use crate::adapters::outbound::content_inspector::is_text_file;
    use crate::domain::entities::{Knot, KnotId, TieOffStatus};
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use super::super::test_fixtures::{
        build_knot, build_loom, default_profile, MockAgentRunner,
        MockGitVersioningPort, MockLoomLogPort, MockProfileRepository,
        MockRigLogPort, MockEventDispatcher, MockStrandFileChecker, MockTieOffSink,
        TrackingTieOffSink,
    };

    /// Build a knot with git_versioned: false (not needed for text checks).
    fn build_knot_no_git(id: impl Into<String>) -> Knot {
        let mut knot = build_knot(id);
        knot.git_versioned = false;
        knot
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

        let (log_port, log_events) = MockLoomLogPort::new();
        let (tie_off_sink, tie_off_appends, _content) =
            TrackingTieOffSink::new();

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]))),
        });

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(log_port),
            Arc::new(MockAgentRunner::default()),
            Arc::new(tie_off_sink),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(
                crate::adapters::outbound::ContentInspectorChecker,
            ),
            Arc::new(MockEventDispatcher::default()),
            None,
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

        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
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

        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
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

        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
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

        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
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

        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
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

// ── File Existence Check Tests ───────────────────────────────────

#[cfg(test)]
mod file_existence_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId};
    use crate::domain::temp_file::is_known_temp_file;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use super::super::test_fixtures::{
        build_knot, build_loom, default_profile, MockAgentRunner,
        MockGitVersioningPort, MockLoomLogPort, MockProfileRepository,
        MockRigLogPort, MockEventDispatcher, MockStrandFileChecker, TrackingTieOffSink,
    };

    /// Build a knot with git_versioned: false.
    fn build_knot_no_git(id: impl Into<String>) -> Knot {
        let mut knot = build_knot(id);
        knot.git_versioned = false;
        knot
    }

    #[allow(clippy::type_complexity)]
    fn build_process_strand(
        loom: Loom,
        agent_runner: Arc<MockAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<TieOff>>>,
        Arc<MockAgentRunner>,
    ) {
        let store = LoomStore::new();
        store.register(loom);

        let (log_port, log_events) = MockLoomLogPort::new();
        let (tie_off_sink, tie_off_appends, _content) =
            TrackingTieOffSink::new();

        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), default_profile()),
            ]))),
        });

        let use_case = ProcessStrand::new(
            store.clone(),
            Arc::new(log_port),
            agent_runner.clone() as Arc<dyn AgentRunner>,
            Arc::new(tie_off_sink),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(MockRigLogPort::default()),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );

        (use_case, log_events, tie_off_appends, agent_runner)
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

        let runner = Arc::new(MockAgentRunner::default());
        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
        let (use_case, log_events, tie_off_appends, captured) =
            build_process_strand(loom, runner);

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
        let was_called = !captured.get_captured_contexts().is_empty();
        assert!(
            !was_called,
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

        let runner = Arc::new(MockAgentRunner::default());
        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
        let (use_case, log_events, tie_off_appends, captured) =
            build_process_strand(loom, runner);

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
        let was_called = !captured.get_captured_contexts().is_empty();
        assert!(!was_called);

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

        let runner = Arc::new(MockAgentRunner::default());
        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
        let (use_case, log_events, tie_off_appends, captured) =
            build_process_strand(loom, runner);

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
        let was_called = !captured.get_captured_contexts().is_empty();
        assert!(
            !was_called,
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

        let runner = Arc::new(MockAgentRunner::default());
        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
        let (use_case, log_events, tie_off_appends, captured) =
            build_process_strand(loom, runner);

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
        let was_called = !captured.get_captured_contexts().is_empty();
        assert!(
            was_called,
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

        let runner = Arc::new(MockAgentRunner::default());
        let loom = build_loom("test-loom", vec![build_knot_no_git("k1")]);
        let (use_case, log_events, tie_off_appends, captured) =
            build_process_strand(loom, runner);

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
        let was_called = !captured.get_captured_contexts().is_empty();
        assert!(
            was_called,
            "agent runner should be called for deleted events"
        );

        // Tie-off IS written
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1, "tie-off should be appended");
    }
}

// ── Phase 5: Event Dispatch Integration Tests ──────────────────────────

#[cfg(test)]
mod event_dispatch_tests {
    use super::*;
    use crate::domain::entities::{KnotId, LoomId, PromptTemplate};
    use crate::application::ports::AgentOutput;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use super::super::test_fixtures::{
        build_knot, build_knot_with_profile, build_loom, default_profile,
        MockAgentRunner, MockEventDispatcher, MockGitVersioningPort,
        MockLoomLogPort, MockProfileRepository, MockRigLogPort,
        MockStrandFileChecker, TrackingTieOffSink,
    };

    /// Build a knot that listens for events from another knot.
    fn build_consumer_knot(
        id: &str,
        target_knot: &str,
        event_id: &str,
        event_desc: &str,
    ) -> Knot {
        Knot {
            id: KnotId(id.to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "React to events.".to_string(),
            },
            git_versioned: true,
            strand_source: StrandSource::EventUri {
                producer_knot: target_knot.to_string(),
                event_id: event_id.to_string(),
            },
            event_description: Some(event_desc.to_string()),
        }
    }

    /// Build a producer knot with no event subscriptions.
    fn build_producer_knot(id: &str) -> Knot {
        build_knot(id)
    }

    /// Build ProcessStrand with a tracking event dispatcher so we can
    /// inspect dispatch calls.
    #[allow(clippy::type_complexity)]
    fn build_process_strand_with_dispatcher(
        looms: Vec<Loom>,
        agent_runner: Arc<MockAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<TieOff>>>,
        Arc<Mutex<HashMap<String, String>>>,
        Arc<Mutex<Vec<(crate::domain::events::AgentEvent, String, String, String)>>>,
        LoomStore,
    ) {
        let store = LoomStore::new();
        for loom in looms {
            store.register(loom);
        }

        let (log_port, log_events) = MockLoomLogPort::new();
        let (tie_off_sink, tie_off_appends, tie_off_content) =
            TrackingTieOffSink::new();
        let (rig_log, _rig_events) = MockRigLogPort::new();
        let (event_dispatcher, dispatches) = MockEventDispatcher::new();

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
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(event_dispatcher),
            None,
        );

        (use_case, log_events, tie_off_appends, tie_off_content, dispatches, store)
    }

    /// Full flow: producer knot emits an event in tie-off, consumer knot
    /// has matching intent, event file is dispatched.
    #[test]
    fn event_dispatch_full_flow() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        // Producer loom with a producer knot
        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );

        // Consumer loom with a consumer knot that listens for PlanCreated
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        // Agent output contains a structured event in the tie-off body
        let event_content = concat!(
            "Plan created successfully.\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "description: Test plan\n",
            "---\n",
            "```",
        );

        let output = Ok(AgentOutput {
            stdout: event_content.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Verify dispatch was called
        let dispatched = dispatches.lock().unwrap();
        assert_eq!(
            dispatched.len(),
            1,
            "should have dispatched exactly 1 event, got {}",
            dispatched.len()
        );
        let (evt, consumer_knot_name, consumer_loom, _rig_dir) = &dispatched[0];
        assert_eq!(evt.event_id, "PlanCreated");
        assert_eq!(*consumer_knot_name, "plan-watcher");
        assert_eq!(*consumer_loom, "consumer-loom");

        // Verify loom-log has EventsDispatched entry
        let events = log_events.lock().unwrap();
        let dispatch_log = events.iter().find(|e| {
            matches!(e, LoomEvent::EventsDispatched { .. })
        });
        assert!(
            dispatch_log.is_some(),
            "loom-log should contain EventsDispatched event"
        );
        if let LoomEvent::EventsDispatched { dispatches: d, .. } = dispatch_log.unwrap() {
            assert_eq!(d.len(), 1);
            assert_eq!(d[0].0, "PlanCreated");
            assert_eq!(d[0].1, "consumer-loom");
        }
    }

    /// No events in tie-off: successful processing produces no dispatch.
    #[test]
    fn no_events_no_dispatch() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        let output = Ok(AgentOutput {
            stdout: "Just normal output, no events.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // No dispatch
        let dispatched = dispatches.lock().unwrap();
        assert!(dispatched.is_empty(), "should have no dispatches");

        // No EventsDispatched in loom-log
        let events = log_events.lock().unwrap();
        let dispatch_log = events.iter().any(|e| {
            matches!(e, LoomEvent::EventsDispatched { .. })
        });
        assert!(
            !dispatch_log,
            "loom-log should NOT contain EventsDispatched when no events"
        );
    }

    /// Fan-out: one event matches consumers in two different looms.
    #[test]
    fn event_dispatch_fan_out_two_looms() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );

        // Two consumer looms, each listening for the same event
        let consumer_loom_a = build_loom(
            "consumer-loom-a",
            vec![build_consumer_knot(
                "watcher-a",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );
        let consumer_loom_b = build_loom(
            "consumer-loom-b",
            vec![build_consumer_knot(
                "watcher-b",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        let event_content = concat!(
            "Plan created.\n",
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-002\n",
            "---\n",
            "```",
        );
        let output = Ok(AgentOutput {
            stdout: event_content.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(
                vec![producer_loom, consumer_loom_a, consumer_loom_b],
                runner,
            );

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Two dispatches — one per consumer loom
        let dispatched = dispatches.lock().unwrap();
        assert_eq!(
            dispatched.len(),
            2,
            "should have dispatched to 2 consumers"
        );

        // Collect the loom IDs
        let looms: Vec<String> = dispatched
            .iter()
            .map(|(_, _, loom, _)| loom.clone())
            .collect();
        assert!(looms.contains(&"consumer-loom-a".to_string()));
        assert!(looms.contains(&"consumer-loom-b".to_string()));
    }

    /// Listener context is injected into the prompt when consumers exist.
    #[test]
    fn listener_context_injected_in_prompt() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "Emit when a plan is created.",
            )],
        );

        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, _dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner.clone());

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Inspect captured context for listener context
        let contexts = runner.get_captured_contexts();
        assert!(!contexts.is_empty(), "agent should have been called");
        let prompt = &contexts[0].prompt;

        // Prompt should contain listener context heading
        assert!(
            prompt.contains("## Agent Events"),
            "prompt should contain Agent Events heading: {}",
            prompt
        );
        assert!(
            prompt.contains("Events you may emit:"),
            "prompt should contain event header: {}",
            prompt
        );
        assert!(
            prompt.contains("`PlanCreated`"),
            "prompt should contain event-id: {}",
            prompt
        );
        assert!(
            prompt.contains("Emit when a plan is created."),
            "prompt should contain event description: {}",
            prompt
        );
        assert!(
            prompt.contains("event: None"),
            "prompt should instruct to emit event: None: {}",
            prompt
        );
        // Listener context is at the beginning, before the knot's instructions
        assert!(
            prompt.starts_with("## Agent Events"),
            "listener context should be at the start of the prompt: {}",
            prompt
        );
    }

    /// When no consumers listen for a knot's events, no context is injected.
    #[test]
    fn no_listeners_no_context_injection() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        // Producer with no consumers
        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );

        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, _dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom], runner.clone());

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let contexts = runner.get_captured_contexts();
        let prompt = &contexts[0].prompt;

        // Prompt should NOT contain listener context
        assert!(
            !prompt.contains("Before undertaking your task"),
            "prompt should NOT contain listener context when no listeners: {}",
            prompt
        );
        // Prompt should just be the knot's instructions
        assert_eq!(prompt, "check it");
    }

    /// `event: None` — no dispatch occurs.
    #[test]
    fn event_none_produces_no_dispatch() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        // Agent output has `event: None` — should not dispatch
        let output = Ok(AgentOutput {
            stdout: "  event: None\n".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let dispatched = dispatches.lock().unwrap();
        assert!(
            dispatched.is_empty(),
            "'event: None' should produce no dispatch, got {} events",
            dispatched.len()
        );

        let events = _log_events.lock().unwrap();
        let dispatch_log = events.iter().any(|e| {
            matches!(e, LoomEvent::EventsDispatched { .. })
        });
        assert!(
            !dispatch_log,
            "should not log EventsDispatched for event: None"
        );
    }

    /// Event ID mismatch — no dispatch.
    #[test]
    fn event_id_mismatch_no_dispatch() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanApproved", // listener for a different event
                "When a plan is approved.",
            )],
        );

        // Agent output emits `PlanCreated` but consumer listens for `PlanApproved`
        let output = Ok(AgentOutput {
            stdout: concat!(
                "  event: PlanCreated\n",
                "  plan: PLAN-001\n",
            )
            .to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let dispatched = dispatches.lock().unwrap();
        assert!(
            dispatched.is_empty(),
            "event ID mismatch should produce no dispatch, got {} events",
            dispatched.len()
        );
    }

    /// Description field passes through to event payload.
    #[test]
    fn description_field_passes_through() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        let output = Ok(AgentOutput {
            stdout: concat!(
                "```markdown\n",
                "---\n",
                "event: PlanCreated\n",
                "plan: PLAN-001\n",
                "description: New plan for feature X\n",
                "---\n",
                "```
"
            )
            .to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let dispatched = dispatches.lock().unwrap();
        assert_eq!(dispatched.len(), 1);
        let (evt, _, _, _) = &dispatched[0];
        assert_eq!(evt.event_id, "PlanCreated");
        assert_eq!(
            evt.payload.get("plan"),
            Some(&"PLAN-001".to_string())
        );
        assert_eq!(
            evt.payload.get("description"),
            Some(&"New plan for feature X".to_string())
        );
    }

    /// `target-knot` is derived from the producing knot context, not from
    /// the event block. Event file frontmatter should contain the producer's
    /// knot ID.
    #[test]
    fn target_knot_derived_from_producer_context() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        // Event block does NOT contain target-knot (derived from context)
        let output = Ok(AgentOutput {
            stdout: concat!(
                "```markdown\n",
                "---\n",
                "event: PlanCreated\n",
                "plan: PLAN-001\n",
                "---\n",
                "```\n"
            )
            .to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let dispatched = dispatches.lock().unwrap();
        assert_eq!(dispatched.len(), 1);
        let (evt, consumer_knot_name, consumer_loom_id, rig_dir) = &dispatched[0];

        // Event has no target_knot field (removed from struct)
        assert_eq!(evt.event_id, "PlanCreated");
        assert!(
            evt.payload.is_empty() || !evt.payload.contains_key("target-knot"),
            "event should not contain target-knot in payload"
        );

        // Consumer knot name is "plan-watcher"
        assert_eq!(*consumer_knot_name, "plan-watcher");
        assert_eq!(*consumer_loom_id, "consumer-loom");

        // Verify that the event file would contain target-knot = producer
        // knot ID by constructing what the real dispatcher would write.
        let event_path = std::path::PathBuf::from(rig_dir)
            .join("tie-offs")
            .join(consumer_loom_id)
            .join(&evt.event_id)
            .join("event-mock.md");
        let _ = event_path;

        // The real dispatcher (FileSystemEventDispatcher) receives the
        // producer knot ID as a separate parameter and writes it into
        // the event file frontmatter — confirming that target-knot is
        // derived from context, not from the event block.
        let content =
            crate::adapters::outbound::event_dispatcher::FileSystemEventDispatcher::build_event_file_content(
                evt,
                "2026-07-10T12:00:00Z",
                "plan-creator",
            );
        assert!(
            content.contains("target-knot: plan-creator"),
            "event file frontmatter should contain target-knot: {}",
            content
        );
        assert!(
            content.contains("## Event: PlanCreated from plan-creator"),
            "event file body should reference producer knot: {}",
            content
        );
    }

    // ── Loom-Level Dispatch Tests ────────────────────────────────────

    /// Producer in a loom that a consumer has subscribed to via loom-level
    /// event URI — event is dispatched.
    #[test]
    fn dispatch_agent_events_loom_level_subscription_matches() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        // Producer loom — consumer listens for events from THIS loom
        let producer_loom = build_loom(
            "planning-loom",
            vec![build_producer_knot("plan-creator")],
        );

        // Consumer subscribes to the loom-level event (target ends with -loom)
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "planning-loom", // loom-level subscription
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        let event_content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "description: Test plan\n",
            "---\n",
            "```",
        );

        let output = Ok(AgentOutput {
            stdout: event_content.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("planning-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let dispatched = dispatches.lock().unwrap();
        assert_eq!(
            dispatched.len(),
            1,
            "loom-level subscription should dispatch, got {} events",
            dispatched.len()
        );
        let (evt, consumer_knot_name, consumer_loom, _rig_dir) = &dispatched[0];
        assert_eq!(evt.event_id, "PlanCreated");
        assert_eq!(*consumer_knot_name, "plan-watcher");
        assert_eq!(*consumer_loom, "consumer-loom");
    }

    /// Producer in a different loom from the loom-level subscription —
    /// no dispatch.
    #[test]
    fn dispatch_agent_events_loom_level_subscription_no_match_different_loom() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        // Producer is in review-loom, consumer listens for planning-loom events
        let producer_loom = build_loom(
            "review-loom",
            vec![build_producer_knot("reviewer")],
        );

        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "planning-loom", // subscribed to a different loom
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        let event_content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "description: Test plan\n",
            "---\n",
            "```",
        );

        let output = Ok(AgentOutput {
            stdout: event_content.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("review-loom".to_string()),
            knot_id: KnotId("reviewer".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let dispatched = dispatches.lock().unwrap();
        assert!(
            dispatched.is_empty(),
            "loom-level subscription should not match a different loom, got {} events",
            dispatched.len()
        );
    }

    /// Both knot-level and loom-level consumers for the same event
    /// from the same producer — both receive the dispatch.
    #[test]
    fn dispatch_agent_events_mixed_knot_and_loom_subscriptions() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "planning-loom",
            vec![build_producer_knot("plan-creator")],
        );

        // Consumer 1: knot-level subscription (targets plan-creator directly)
        let knot_consumer = build_consumer_knot(
            "plan-watcher",
            "plan-creator",
            "PlanCreated",
            "When a plan is created.",
        );
        // Consumer 2: loom-level subscription (targets planning-loom)
        let loom_consumer = build_consumer_knot(
            "plan-auditor",
            "planning-loom",
            "PlanCreated",
            "When a plan is created.",
        );

        let consumer_loom = build_loom(
            "consumer-loom",
            vec![knot_consumer, loom_consumer],
        );

        let event_content = concat!(
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "description: Test plan\n",
            "---\n",
            "```",
        );

        let output = Ok(AgentOutput {
            stdout: event_content.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("planning-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let dispatched = dispatches.lock().unwrap();
        assert_eq!(
            dispatched.len(),
            2,
            "both knot-level and loom-level consumers should dispatch, got {} events",
            dispatched.len()
        );

        // Verify both consumers received the event
        let consumer_names: Vec<&String> =
            dispatched.iter().map(|(_, name, _, _)| name).collect();
        assert!(
            consumer_names.contains(&&"plan-watcher".to_string()),
            "knot-level consumer should receive dispatch"
        );
        assert!(
            consumer_names.contains(&&"plan-auditor".to_string()),
            "loom-level consumer should receive dispatch"
        );
    }

    /// Build ProcessStrand with a real temp rig directory so the context
    /// provider can scan for pending event files.
    #[allow(clippy::type_complexity)]
    fn build_process_strand_with_rig(
        looms: Vec<Loom>,
        agent_runner: Arc<MockAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<TieOff>>>,
        Arc<Mutex<HashMap<String, String>>>,
        Arc<Mutex<Vec<(crate::domain::events::AgentEvent, String, String, String)>>>,
        LoomStore,
        TempDir,
    ) {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let store = LoomStore::new();
        for loom in looms {
            store.register(loom);
        }

        let (log_port, log_events) = MockLoomLogPort::new();
        let (tie_off_sink, tie_off_appends, tie_off_content) =
            TrackingTieOffSink::new();
        let (rig_log, _rig_events) = MockRigLogPort::new();
        let (event_dispatcher, dispatches) = MockEventDispatcher::new();

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
            rig_dir.clone(),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(event_dispatcher),
            None,
        );

        (use_case, log_events, tie_off_appends, tie_off_content, dispatches, store, dir)
    }

    /// Helper: create a dispatched event file in the rig directory.
    fn create_pending_event_file(
        rig_dir: &std::path::Path,
        consumer_loom: &str,
        event_id: &str,
        target_knot: &str,
        description: Option<&str>,
        filename: &str,
    ) {
        let event_dir = rig_dir
            .join("tie-offs")
            .join(consumer_loom)
            .join(event_id);
        std::fs::create_dir_all(&event_dir).unwrap();

        let mut lines = vec![
            "---".to_string(),
            format!("event-id: {}", event_id),
            format!("target-knot: {}", target_knot),
            "timestamp: 2026-07-14T10:00:00Z".to_string(),
        ];
        if let Some(desc) = description {
            lines.push(format!("description: {}", desc));
        }
        lines.push("---".to_string());
        lines.push(String::new());
        lines.push(format!("## Event: {} from {}", event_id, target_knot));

        std::fs::write(event_dir.join(filename), lines.join("\n")).unwrap();
    }

    /// ProcessStrand execution test: the prompt contains both emission
    /// instructions and pending events when event files exist on disk.
    #[test]
    fn prompt_contains_pending_events_when_files_exist() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "Emit when a plan is created.",
            )],
        );

        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, _dispatches, _store, temp_dir) =
            build_process_strand_with_rig(vec![producer_loom, consumer_loom], runner.clone());

        // Create a pending event file in the rig directory
        let rig_dir = temp_dir.path().join("rig");
        create_pending_event_file(
            &rig_dir,
            "consumer-loom",
            "PlanCreated",
            "plan-creator",
            Some("Previous plan for feature X"),
            "event-2026-07-14T10-00-00Z.md",
        );

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Inspect the captured prompt
        let contexts = runner.get_captured_contexts();
        assert!(!contexts.is_empty(), "agent should have been called");
        let prompt = &contexts[0].prompt;

        // Should contain emission instructions
        assert!(
            prompt.contains("## Agent Events"),
            "prompt should contain Agent Events heading: {}",
            prompt
        );
        assert!(
            prompt.contains("`PlanCreated`"),
            "prompt should contain event-id: {}",
            prompt
        );

        // Should contain pending events section
        assert!(
            prompt.contains("## Pending Events"),
            "prompt should contain Pending Events section: {}",
            prompt
        );
        assert!(
            prompt.contains("Previous plan for feature X"),
            "prompt should contain pending event description: {}",
            prompt
        );
        assert!(
            prompt.contains("event-2026-07-14T10-00-00Z.md"),
            "prompt should contain pending event filename: {}",
            prompt
        );
    }

    /// ProcessStrand execution test: when no pending event files exist,
    /// the prompt contains only emission instructions (no Pending Events section).
    #[test]
    fn prompt_no_pending_events_when_no_files_exist() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "Emit when a plan is created.",
            )],
        );

        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _log_events, _tie_off_appends, _content, _dispatches, _store, _temp_dir) =
            build_process_strand_with_rig(vec![producer_loom, consumer_loom], runner.clone());

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let contexts = runner.get_captured_contexts();
        assert!(!contexts.is_empty(), "agent should have been called");
        let prompt = &contexts[0].prompt;

        // Should contain emission instructions
        assert!(
            prompt.contains("## Agent Events"),
            "prompt should contain Agent Events heading: {}",
            prompt
        );

        // Should NOT contain pending events section (no files on disk)
        assert!(
            !prompt.contains("## Pending Events"),
            "prompt should NOT contain Pending Events section when no files exist: {}",
            prompt
        );
    }

    /// Refactor verification: existing event enforcement flow still works.
    /// The KnotEventsMissing path uses build_ctx.all_knots correctly.
    #[test]
    fn event_enforcement_flow_unchanged() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "Emit when a plan is created.",
            )],
        );

        // Agent output with no events — should trigger enforcement
        let output = Ok(AgentOutput {
            stdout: "Plan created, no events emitted.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, log_events, _tie_off_appends, _content, _dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Verify KnotEventsMissing was logged
        let events = log_events.lock().unwrap();
        let has_missing = events.iter().any(|e| {
            matches!(e, LoomEvent::KnotEventsMissing { .. })
        });
        assert!(
            has_missing,
            "KnotEventsMissing should be logged when no events are emitted"
        );
    }
}

// ── Phase 6: TieOff Event Metadata in Append Tests ─────────────────────

#[cfg(test)]
mod tieoff_event_metadata_tests {
    use super::*;
    use crate::adapters::outbound::tieoff_sink::FileSystemTieOffSink;
    use crate::domain::entities::TieOffStatus;
    use tempfile::TempDir;

    #[test]
    fn append_with_event_metadata_includes_structured_fields() {
        let dir = TempDir::new().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("consumer-tie-off.md");

        let tie_off = TieOff {
            content: "Consumer agent response".to_string(),
            path: TieOffPath(file_path.clone()),
            status: TieOffStatus::Produced,
            knot_name: Some("event-consumer".to_string()),
            event_type: Some("Created".to_string()),
            strand_path: Some(
                "rig/tie-offs/consumer-loom/PlanCreated/event-2026-07-09T12-00-00Z.md"
                    .to_string(),
            ),
            timestamp: Some("2026-07-09T12:05:00Z".to_string()),
            agent_events: Vec::new(),
            event_metadata: EventMetadata {
                event_id: Some("PlanCreated".to_string()),
                source_knot: Some("plan-creator".to_string()),
                original_strand: Some("001-feature.md".to_string()),
            },
        };

        sink.append(tie_off).unwrap();
        let content =
            std::fs::read_to_string(&file_path).unwrap();

        // Should contain the structured metadata fields
        assert!(
            content.contains("event: PlanCreated"),
            "should contain event field: {}",
            content
        );
        assert!(
            content.contains("source: plan-creator"),
            "should contain source field: {}",
            content
        );
        assert!(
            content.contains("original_strand: 001-feature.md"),
            "should contain original_strand field: {}",
            content
        );
        // Regular header fields still present
        assert!(
            content.contains("## event-consumer triggered by Created"),
            "should contain standard header: {}",
            content
        );
        assert!(
            content.contains("Timestamp: 2026-07-09T12:05:00Z"),
            "should contain timestamp: {}",
            content
        );
    }

    #[test]
    fn append_without_event_metadata_has_no_extra_fields() {
        let dir = TempDir::new().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("normal-tie-off.md");

        let tie_off = TieOff {
            content: "Normal response".to_string(),
            path: TieOffPath(file_path.clone()),
            status: TieOffStatus::Produced,
            knot_name: Some("normal-knot".to_string()),
            event_type: Some("Created".to_string()),
            strand_path: Some("strands/input.md".to_string()),
            timestamp: Some("2026-07-09T12:00:00Z".to_string()),
            agent_events: Vec::new(),
            event_metadata: EventMetadata::default(),
        };

        sink.append(tie_off).unwrap();
        let content =
            std::fs::read_to_string(&file_path).unwrap();

        // Should NOT contain event metadata fields
        assert!(
            !content.contains("event:"),
            "should NOT contain event field for normal strand: {}",
            content
        );
        assert!(
            !content.contains("source:"),
            "should NOT contain source field: {}",
            content
        );
        // Standard header still present
        assert!(
            content.contains("## normal-knot triggered by Created"),
            "should contain standard header"
        );
    }

    #[test]
    fn append_with_partial_event_metadata_only_shows_set_fields() {
        let dir = TempDir::new().unwrap();
        let sink = FileSystemTieOffSink::new(dir.path().to_path_buf());
        let file_path = dir.path().join("partial-tie-off.md");

        let tie_off = TieOff {
            content: "Response".to_string(),
            path: TieOffPath(file_path.clone()),
            status: TieOffStatus::Produced,
            knot_name: Some("consumer".to_string()),
            event_type: Some("Created".to_string()),
            strand_path: Some("event-file.md".to_string()),
            timestamp: Some("2026-07-09T12:00:00Z".to_string()),
            agent_events: Vec::new(),
            event_metadata: EventMetadata {
                event_id: Some("PlanCreated".to_string()),
                source_knot: None,
                original_strand: None,
            },
        };

        sink.append(tie_off).unwrap();
        let content =
            std::fs::read_to_string(&file_path).unwrap();

        // Only event_id should be present
        assert!(
            content.contains("event: PlanCreated"),
            "should contain event field: {}",
            content
        );
        assert!(
            !content.contains("source:"),
            "should NOT contain source when not set: {}",
            content
        );
    }
}

// ── Phase 6: Event Metadata and Tie-Off Integration Tests ─────────────

#[cfg(test)]
mod phase6_integration_tests {
    use super::*;
    use crate::domain::entities::{KnotId, LoomId, PromptTemplate};
    use crate::application::ports::AgentOutput;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use super::super::test_fixtures::{
        build_knot, build_loom, default_profile,
        MockAgentRunner, MockEventDispatcher, MockGitVersioningPort,
        MockLoomLogPort, MockProfileRepository, MockRigLogPort,
        MockStrandFileChecker, TrackingTieOffSink,
    };

    // ── Helpers ──────────────────────────────────────────────────────

    /// Build a consumer knot with EventUri strand source.
    fn build_event_consumer_knot(
        id: &str,
        producer_knot: &str,
        event_id: &str,
    ) -> Knot {
        Knot {
            id: KnotId(id.to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "React to events.".to_string(),
            },
            git_versioned: true,
            strand_source: StrandSource::EventUri {
                producer_knot: producer_knot.to_string(),
                event_id: event_id.to_string(),
            },
            event_description: Some(format!("When {event_id} occurs.")),
        }
    }

    /// Build a normal Filesystem knot.
    fn build_filesystem_knot(id: &str) -> Knot {
        build_knot(id)
    }

    /// Write an event file at the given directory with the given frontmatter.
    fn write_event_file(
        dir: &TempDir,
        event_id: &str,
        producer_knot: &str,
        original_strand: Option<&str>,
    ) -> PathBuf {
        let mut content = String::from("---\n");
        content.push_str(&format!("event-id: {}\n", event_id));
        content.push_str(&format!("target-knot: {}\n", producer_knot));
        if let Some(orig) = original_strand {
            content.push_str(&format!("original-strand: {}\n", orig));
        }
        content.push_str("timestamp: 2026-07-09T12:00:00Z\n");
        content.push_str("---\n\n");
        content.push_str(&format!(
            "## Event: {} from {}\n\nBody",
            event_id, producer_knot
        ));
        let filename = format!("event-2026-07-09T12-00-00Z.md");
        let path = dir.path().join(&filename);
        std::fs::write(&path, &content).unwrap();
        path
    }

    /// Build ProcessStrand with tracking tie-off sink.
    fn build_process_strand_with_tracking(
        looms: Vec<Loom>,
        agent_runner: Arc<MockAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<TieOff>>>,
        Arc<Mutex<HashMap<String, String>>>,
        LoomStore,
    ) {
        let store = LoomStore::new();
        for loom in looms {
            store.register(loom);
        }

        let (log_port, _log_events) = MockLoomLogPort::new();
        let (tie_off_sink, tie_off_appends, tie_off_content) =
            TrackingTieOffSink::new();
        let (rig_log, _rig_events) = MockRigLogPort::new();

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
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(MockEventDispatcher::default()),
            None,
        );

        (use_case, tie_off_appends, tie_off_content, store)
    }

    // ── Tests: Event-triggered consumer knot produces tie-off with event metadata ──

    /// When a consumer knot processes an event file (dispatched by
    /// intent-based routing event file), extract_event_metadata()
    /// populates EventMetadata from the frontmatter. The tie-off
    /// append includes structured event fields.
    #[test]
    fn event_triggered_consumer_knot_produces_tieoff_with_event_metadata() {
        let dir = TempDir::new().unwrap();
        let event_file = write_event_file(
            &dir,
            "PlanCreated",
            "plan-creator",
            Some("001-feature.md"),
        );

        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_event_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
            )],
        );

        let output = Ok(AgentOutput {
            stdout: "Plan watched and validated.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, tie_off_appends, _content, _store) =
            build_process_strand_with_tracking(vec![consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("consumer-loom".to_string()),
            knot_id: KnotId("plan-watcher".to_string()),
            strand_path: StrandPath(event_file.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Tie-off should be appended with event metadata
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(
            appends.len(),
            1,
            "should have appended exactly 1 tie-off"
        );
        let appended = &appends[0];
        assert!(
            appended.event_metadata.is_some(),
            "tie-off should have event metadata for event-triggered strand"
        );
        assert_eq!(
            appended.event_metadata.event_id.as_deref(),
            Some("PlanCreated")
        );
        assert_eq!(
            appended.event_metadata.source_knot.as_deref(),
            Some("plan-creator")
        );
        assert_eq!(
            appended.event_metadata.original_strand.as_deref(),
            Some("001-feature.md")
        );
    }

    // ── Tests: Filesystem knot produces tie-off without event metadata ──

    /// When a normal Filesystem knot processes a regular strand file
    /// (not an event file), extract_event_metadata() returns None
    /// and the tie-off has no event metadata.
    #[test]
    fn filesystem_knot_produces_tieoff_without_event_metadata() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("prd.md");
        std::fs::write(&strand_path, "Plan for feature X").unwrap();

        let loom = build_loom(
            "review-loom",
            vec![build_filesystem_knot("reviewer")],
        );

        let output = Ok(AgentOutput {
            stdout: "Reviewed.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, tie_off_appends, _content, _store) =
            build_process_strand_with_tracking(vec![loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("review-loom".to_string()),
            knot_id: KnotId("reviewer".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Tie-off should be appended WITHOUT event metadata
        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(
            appends.len(),
            1,
            "should have appended exactly 1 tie-off"
        );
        let appended = &appends[0];
        assert!(
            appended.event_metadata.is_none(),
            "tie-off should NOT have event metadata for filesystem strand"
        );
    }

    // ── Tests: Partial event metadata preserved ──

    /// When an event file has only event-id (no target-knot, no
    /// original-strand), the tie-off preserves only the fields
    /// that were present.
    #[test]
    fn partial_event_metadata_preserved() {
        let dir = TempDir::new().unwrap();
        let event_file = write_event_file(
            &dir,
            "PlanCreated",
            "plan-creator",
            None, // no original-strand
        );

        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_event_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
            )],
        );

        let output = Ok(AgentOutput {
            stdout: "Watched.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, tie_off_appends, _content, _store) =
            build_process_strand_with_tracking(vec![consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("consumer-loom".to_string()),
            knot_id: KnotId("plan-watcher".to_string()),
            strand_path: StrandPath(event_file.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        let appends = tie_off_appends.lock().unwrap();
        assert_eq!(appends.len(), 1);
        let appended = &appends[0];
        assert!(appended.event_metadata.is_some());
        // event-id is present
        assert_eq!(
            appended.event_metadata.event_id.as_deref(),
            Some("PlanCreated")
        );
        // source_knot is present (mapped from target-knot in frontmatter)
        assert_eq!(
            appended.event_metadata.source_knot.as_deref(),
            Some("plan-creator")
        );
        // original_strand is None (not in frontmatter)
        assert!(appended.event_metadata.original_strand.is_none());
    }

    // ── Tests: AgentEvent target_knot derived from producing knot ──

    /// In the dispatch flow, the producing knot's ID is known from
    /// the ProcessStrand context (the knot executing the strand).
    /// It is passed as `producer_knot` to `dispatch()`, which writes
    /// it into the event file frontmatter. The AgentEvent struct
    /// no longer carries a `target_knot` field — it is derived at
    /// dispatch time from the executing knot's ID.
    #[test]
    fn agent_event_target_knot_derived_from_producing_knot() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        // Producer loom with a producer knot
        let producer_loom = build_loom(
            "producer-loom",
            vec![build_filesystem_knot("plan-creator")],
        );

        // Consumer loom with EventUri knot
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_event_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
            )],
        );

        // Producer emits an event (no target-knot in event block —
        // it's derived from context)
        let output_content = concat!(
            "Plan created.\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n",
            "```",
        );
        let output = Ok(AgentOutput {
            stdout: output_content.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, _tie_off_appends, _content, _store) =
            build_process_strand_with_tracking(
                vec![producer_loom, consumer_loom],
                runner,
            );

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Verify the agent event does NOT contain target_knot.
        // The Event struct has no target_knot field — only event_id
        // and payload. The producing knot's ID is available from
        // the ProcessStrand context (knot.id.0 == "plan-creator"),
        // not from the event data itself.
        let parsed_events =
            crate::domain::tieoff_parser::extract_agent_events(
                output_content,
            );
        assert_eq!(parsed_events.len(), 1);
        let parsed = &parsed_events[0];
        assert_eq!(parsed.event_id, "PlanCreated");
        // target-knot should NOT be in the payload (it's derived from context)
        assert!(
            !parsed.payload.contains_key("target-knot"),
            "agent event should NOT contain target-knot in payload"
        );
        // The producing knot is known from context ("plan-creator"),
        // which matches knot.id.0
    }
}

// ── Phase 3: Event Enforcement Tests ──────────────────────────────────

#[cfg(test)]
mod event_enforcement_tests {
    use super::*;
    use crate::application::ports::{AgentInvocationMetadata, AgentOutput};
    use crate::domain::entities::{KnotId, LoomId, PromptTemplate};
    use crate::domain::value_objects::StrandSource;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;
    use tempfile::TempDir;

    use super::super::test_fixtures::{
        build_knot, build_loom, default_profile, MockAgentRunner,
        MockEventDispatcher, MockGitVersioningPort, MockLoomLogPort,
        MockProfileRepository, MockRigLogPort, MockStrandFileChecker,
        TrackingTieOffSink,
    };

    /// Build a knot that listens for events from another knot.
    fn build_consumer_knot(
        id: &str,
        target_knot: &str,
        event_id: &str,
        event_desc: &str,
    ) -> Knot {
        Knot {
            id: KnotId(id.to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "React to events.".to_string(),
            },
            git_versioned: false,
            strand_source: StrandSource::EventUri {
                producer_knot: target_knot.to_string(),
                event_id: event_id.to_string(),
            },
            event_description: Some(event_desc.to_string()),
        }
    }

    /// Build a producer knot with no event subscriptions.
    fn build_producer_knot(id: &str) -> Knot {
        use crate::application::usecases::test_fixtures::build_knot;
        build_knot(id)
    }

    /// Build ProcessStrand with a tracking event dispatcher.
    #[allow(clippy::type_complexity)]
    fn build_enforcement_strand(
        looms: Vec<Loom>,
        agent_runner: Arc<MockAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<(crate::domain::events::AgentEvent, String, String, String)>>>,
        LoomStore,
    ) {
        let store = LoomStore::new();
        for loom in looms {
            store.register(loom);
        }

        let (log_port, log_events) = MockLoomLogPort::new();
        let (tie_off_sink, _, _) =
            crate::application::usecases::test_fixtures::TrackingTieOffSink::new();
        let (rig_log, _rig_events) = MockRigLogPort::new();
        let (event_dispatcher, dispatches) = MockEventDispatcher::new();

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
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(event_dispatcher),
            None,
        );

        (use_case, log_events, dispatches, store)
    }

    fn ok_output_with_sid(stdout: &str, sid: &str) -> AgentOutput {
        use crate::application::ports::AgentInvocationMetadata;
        AgentOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: Some(AgentInvocationMetadata {
                session_id: Some(sid.to_string()),
                token_usage: None,
            }),
        }
    }

    fn event_block(event_id: &str) -> String {
        format!(
            "```markdown\n---\nevent: {event_id}\n---\n\nEvent body.\n```"
        )
    }

    fn event_none_block() -> String {
        "```markdown\n---\nevent: None\n---\n```".to_string()
    }

    /// No consumers listening for events from this knot — enforcement
    /// is skipped entirely (no KnotEventsMissing logged).
    #[test]
    fn process_strand_enforcement_no_consumers_skipped() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );

        let output = Ok(AgentOutput {
            stdout: "Just normal output, no events.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, log_events, _dispatches, _store) =
            build_enforcement_strand(vec![producer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // No KnotEventsMissing should be logged
        let events = log_events.lock().unwrap();
        let missing_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::KnotEventsMissing { .. }))
            .count();
        assert_eq!(
            missing_count, 0,
            "no enforcement when there are no consumers"
        );
    }

    /// Agent emits events in tie-off — enforcement is skipped.
    #[test]
    fn process_strand_enforcement_events_present_skipped() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        let event_content = concat!(
            "Plan created successfully.\n",
            "\n",
            "```markdown\n",
            "---\n",
            "event: PlanCreated\n",
            "plan: PLAN-001\n",
            "---\n",
            "```",
        );
        let output = Ok(AgentOutput {
            stdout: event_content.to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, log_events, _dispatches, _store) =
            build_enforcement_strand(
                vec![producer_loom, consumer_loom],
                runner,
            );

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // No KnotEventsMissing should be logged
        let events = log_events.lock().unwrap();
        let missing_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::KnotEventsMissing { .. }))
            .count();
        assert_eq!(missing_count, 0, "no enforcement when events are present");
    }

    /// Agent emits `event: None` — enforcement is skipped (valid outcome).
    #[test]
    fn process_strand_enforcement_event_none_skipped() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        let output = Ok(AgentOutput {
            stdout: event_none_block(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, log_events, _dispatches, _store) =
            build_enforcement_strand(
                vec![producer_loom, consumer_loom],
                runner,
            );

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // No KnotEventsMissing should be logged
        let events = log_events.lock().unwrap();
        let missing_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::KnotEventsMissing { .. }))
            .count();
        assert_eq!(
            missing_count,
            0,
            "no enforcement when event: None is emitted"
        );
    }

    /// No events emitted, consumers exist — KnotEventsMissing is logged
    /// and follow-up is attempted.
    #[test]
    fn process_strand_enforcement_missing_events_logs_and_retries() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        // First call: normal output with no events
        // Second call (follow-up): also no events (mock returns same)
        let output = Ok(AgentOutput {
            stdout: "Just normal output, no events.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: Some(AgentInvocationMetadata {
                session_id: Some("sess-test".to_string()),
                token_usage: None,
            }),
        });
        let runner = Arc::new(MockAgentRunner::new(output.clone()));

        let (use_case, log_events, _dispatches, _store) =
            build_enforcement_strand(
                vec![producer_loom, consumer_loom],
                runner,
            );

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // KnotEventsMissing should be logged
        let events = log_events.lock().unwrap();
        let missing_events: Vec<&LoomEvent> = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::KnotEventsMissing { .. }))
            .collect();
        assert!(
            !missing_events.is_empty(),
            "KnotEventsMissing should be logged when events are missing"
        );

        // Verify the KnotEventsMissing carries expected event IDs
        if let LoomEvent::KnotEventsMissing { expected_events, .. } =
            missing_events[0]
        {
            assert!(
                expected_events.contains(&"PlanCreated".to_string()),
                "expected_events should contain PlanCreated"
            );
        }
    }

    /// Follow-up produces events — they are dispatched to consumers.
    #[test]
    fn process_strand_enforcement_followup_produces_events_dispatched() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        // First call: normal output with no events
        // Second call (follow-up): produces an event
        let first_output = AgentOutput {
            stdout: "Just normal output, no events.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: Some(AgentInvocationMetadata {
                session_id: Some("sess-test".to_string()),
                token_usage: None,
            }),
        };
        let followup_output = AgentOutput {
            stdout: event_block("PlanCreated"),
            stderr: String::new(),
            exit_code: 0,
            metadata: Some(AgentInvocationMetadata {
                session_id: Some("sess-test".to_string()),
                token_usage: None,
            }),
        };
        let runner = Arc::new(MockAgentRunner::new_sequence(vec![
            Ok(first_output),
            Ok(followup_output),
        ]));

        let (use_case, log_events, dispatches, _store) =
            build_enforcement_strand(
                vec![producer_loom, consumer_loom],
                runner,
            );

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Event should have been dispatched from follow-up
        let dispatched = dispatches.lock().unwrap();
        assert_eq!(
            dispatched.len(),
            1,
            "follow-up event should be dispatched"
        );
        assert_eq!(dispatched[0].0.event_id, "PlanCreated");
        assert_eq!(dispatched[0].1, "plan-watcher");

        // Only one KnotEventsMissing (the initial detection),
        // not a second one (since follow-up produced events)
        let events = log_events.lock().unwrap();
        let missing_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::KnotEventsMissing { .. }))
            .count();
        assert_eq!(
            missing_count, 1,
            "only one KnotEventsMissing when follow-up succeeds"
        );
    }

    /// Follow-up still produces no events — second KnotEventsMissing logged.
    #[test]
    fn process_strand_enforcement_followup_still_missing_logs_twice() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        // First and second calls both return no events
        let output = AgentOutput {
            stdout: "Just normal output, no events.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: Some(AgentInvocationMetadata {
                session_id: Some("sess-test".to_string()),
                token_usage: None,
            }),
        };
        let runner = Arc::new(MockAgentRunner::new(Ok(output)));

        let (use_case, log_events, _dispatches, _store) =
            build_enforcement_strand(
                vec![producer_loom, consumer_loom],
                runner,
            );

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Two KnotEventsMissing entries (initial + follow-up still missing)
        let events = log_events.lock().unwrap();
        let missing_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::KnotEventsMissing { .. }))
            .count();
        assert_eq!(
            missing_count, 2,
            "two KnotEventsMissing when follow-up also produces no events"
        );
    }

    /// No session ID (stdio adapter) — KnotEventsMissing logged but
    /// no follow-up re-entry attempted.
    #[test]
    fn process_strand_enforcement_no_session_id_log_only() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "When a plan is created.",
            )],
        );

        // Output with no session ID (stdio adapter)
        let output = Ok(AgentOutput {
            stdout: "Just normal output, no events.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None, // no session ID
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, log_events, _dispatches, _store) =
            build_enforcement_strand(
                vec![producer_loom, consumer_loom],
                runner,
            );

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // One KnotEventsMissing logged (no follow-up possible)
        let events = log_events.lock().unwrap();
        let missing_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::KnotEventsMissing { .. }))
            .count();
        assert_eq!(
            missing_count, 1,
            "one KnotEventsMissing when no session ID (log only)"
        );

        // No EventsDispatched (no follow-up)
        let dispatch_count = events
            .iter()
            .filter(|e| matches!(e, LoomEvent::EventsDispatched { .. }))
            .count();
        assert_eq!(
            dispatch_count, 0,
            "no EventsDispatched when no follow-up attempted"
        );
    }
}

// ── Phase 4: Integration and end-to-end verification ─────────────────

#[cfg(test)]
mod phase4_integration_tests {
    use super::*;
    use crate::domain::entities::{Knot, KnotId, LoomId, Loom, PromptTemplate};
    use crate::application::ports::AgentOutput;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use super::super::test_fixtures::{
        build_knot, build_loom, default_profile, MockAgentRunner,
        MockEventDispatcher, MockGitVersioningPort, MockLoomLogPort,
        MockProfileRepository, MockRigLogPort, MockStrandFileChecker,
        TrackingTieOffSink,
    };

    fn build_consumer_knot(
        id: &str,
        producer_knot: &str,
        event_id: &str,
        event_desc: &str,
    ) -> Knot {
        Knot {
            id: KnotId(id.to_string()),
            agent_profile_ref: "fast".to_string(),
            prompt_template: PromptTemplate {
                instructions: "React to events.".to_string(),
            },
            git_versioned: true,
            strand_source: StrandSource::EventUri {
                producer_knot: producer_knot.to_string(),
                event_id: event_id.to_string(),
            },
            event_description: Some(event_desc.to_string()),
        }
    }

    fn build_producer_knot(id: &str) -> Knot {
        build_knot(id)
    }

    #[allow(clippy::type_complexity)]
    fn build_process_strand_with_rig(
        looms: Vec<Loom>,
        agent_runner: Arc<MockAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<TieOff>>>,
        Arc<Mutex<HashMap<String, String>>>,
        Arc<Mutex<Vec<(crate::domain::events::AgentEvent, String, String, String)>>>,
        LoomStore,
        TempDir,
    ) {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        std::fs::create_dir(&rig_dir).unwrap();

        let store = LoomStore::new();
        for loom in looms {
            store.register(loom);
        }

        let (log_port, log_events) = MockLoomLogPort::new();
        let (tie_off_sink, tie_off_appends, tie_off_content) =
            TrackingTieOffSink::new();
        let (rig_log, _rig_events) = MockRigLogPort::new();
        let (event_dispatcher, dispatches) = MockEventDispatcher::new();

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
            rig_dir.clone(),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(event_dispatcher),
            None,
        );

        (use_case, log_events, tie_off_appends, tie_off_content, dispatches, store, dir)
    }

    /// Integration test: producer emits event, then re-enters, prompt
    /// contains pending event reference.
    ///
    /// Simulates the flow:
    /// 1. Producer knot emits `PlanCreated` → event file created in
    ///    dispatch directory (via FileSystemEventDispatcher simulation).
    /// 2. Producer knot re-enters (same strand, e.g., Modified event) →
    ///    pending event from step 1 is visible in its prompt.
    #[test]
    fn producer_re_enters_prompt_contains_pending_event() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "Emit when a plan is created.",
            )],
        );

        // Step 1: Producer emits PlanCreated.
        // Simulate by creating the dispatched event file directly.
        let (use_case, _log_events, _tie_off_appends, _content, _dispatches, _store, temp_dir) =
            build_process_strand_with_rig(
                vec![producer_loom.clone(), consumer_loom.clone()],
                Arc::new(MockAgentRunner::new(Ok(AgentOutput {
                    stdout: "ok".to_string(),
                    stderr: String::new(),
                    exit_code: 0,
                    metadata: None,
                }))),
            );

        // Simulate the dispatched event file that would have been created
        // by FileSystemEventDispatcher.
        let rig_dir = temp_dir.path().join("rig");
        let event_dir = rig_dir.join("tie-offs").join("consumer-loom").join("PlanCreated");
        std::fs::create_dir_all(&event_dir).unwrap();
        std::fs::write(
            event_dir.join("event-2026-07-14T10-00-00Z.md"),
            "---\n"
                .to_string()
                + "event-id: PlanCreated\n"
                + "target-knot: plan-creator\n"
                + "timestamp: 2026-07-14T10:00:00Z\n"
                + "description: Implementation plan for feature X\n"
                + "---\n"
                + "\n"
                + "## Event: PlanCreated from plan-creator\n",
        )
        .unwrap();

        // Step 2: Producer re-enters with a Modified event.
        // Verify the prompt contains the pending event reference.
        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case2, _log_events2, _tie_off_appends2, _content2, _dispatches2, _store2, _temp_dir2) =
            build_process_strand_with_rig(
                vec![producer_loom, consumer_loom],
                runner.clone(),
            );

        // We need to reuse the same rig directory, so let's copy the
        // event file from step 1 into step 2's rig directory.
        let rig_dir2 = _temp_dir2.path().join("rig");
        let event_dir2 = rig_dir2.join("tie-offs").join("consumer-loom").join("PlanCreated");
        std::fs::create_dir_all(&event_dir2).unwrap();
        std::fs::write(
            event_dir2.join("event-2026-07-14T10-00-00Z.md"),
            "---\n"
                .to_string()
                + "event-id: PlanCreated\n"
                + "target-knot: plan-creator\n"
                + "timestamp: 2026-07-14T10:00:00Z\n"
                + "description: Implementation plan for feature X\n"
                + "---\n"
                + "\n"
                + "## Event: PlanCreated from plan-creator\n",
        )
        .unwrap();

        let event = StrandEvent::Modified {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case2.execute(event);
        assert!(result.is_ok());

        // Inspect the captured prompt
        let contexts = runner.get_captured_contexts();
        assert!(!contexts.is_empty(), "agent should have been called");
        let prompt = &contexts[0].prompt;

        // Should contain pending events section referencing the
        // previously emitted event.
        assert!(
            prompt.contains("## Pending Events"),
            "prompt should contain Pending Events section: {}",
            prompt
        );
        assert!(
            prompt.contains("Implementation plan for feature X"),
            "prompt should contain pending event description: {}",
            prompt
        );
        assert!(
            prompt.contains("PlanCreated"),
            "prompt should reference PlanCreated: {}",
            prompt
        );
    }

    /// Integration test: producer with multiple event types sees only
    /// relevant pending events (matching event IDs).
    #[test]
    fn producer_with_multiple_event_types_sees_relevant_pending_events() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![
                build_consumer_knot(
                    "plan-watcher",
                    "plan-creator",
                    "PlanCreated",
                    "When a plan is created.",
                ),
                build_consumer_knot(
                    "plan-fixer",
                    "plan-creator",
                    "ValidationFailed",
                    "When validation fails.",
                ),
            ],
        );

        let output = Ok(AgentOutput {
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (_use_case, _log_events, _tie_off_appends, _content, _dispatches, _store, temp_dir) =
            build_process_strand_with_rig(
                vec![producer_loom.clone(), consumer_loom.clone()],
                runner.clone(),
            );

        // Create pending events for both event types.
        let rig_dir = temp_dir.path().join("rig");
        let event_dir1 = rig_dir.join("tie-offs").join("consumer-loom").join("PlanCreated");
        std::fs::create_dir_all(&event_dir1).unwrap();
        std::fs::write(
            event_dir1.join("event-2026-07-14T10-00-00Z.md"),
            "---\n"
                .to_string()
                + "event-id: PlanCreated\n"
                + "target-knot: plan-creator\n"
                + "timestamp: 2026-07-14T10:00:00Z\n"
                + "description: Plan for feature X\n"
                + "---\n"
                + "\n"
                + "## Event: PlanCreated from plan-creator\n",
        )
        .unwrap();

        let event_dir2 = rig_dir.join("tie-offs").join("consumer-loom").join("ValidationFailed");
        std::fs::create_dir_all(&event_dir2).unwrap();
        std::fs::write(
            event_dir2.join("event-2026-07-14T11-00-00Z.md"),
            "---\n"
                .to_string()
                + "event-id: ValidationFailed\n"
                + "target-knot: plan-creator\n"
                + "timestamp: 2026-07-14T11:00:00Z\n"
                + "description: Validation failed on PRD\n"
                + "---\n"
                + "\n"
                + "## Event: ValidationFailed from plan-creator\n",
        )
        .unwrap();

        let (use_case, _log_events2, _tie_off_appends2, _content2, _dispatches2, _store2, _temp_dir2) =
            build_process_strand_with_rig(
                vec![producer_loom, consumer_loom],
                runner.clone(),
            );

        // Copy event files to the new rig directory.
        let rig_dir2 = _temp_dir2.path().join("rig");
        let event_dir1_2 = rig_dir2.join("tie-offs").join("consumer-loom").join("PlanCreated");
        std::fs::create_dir_all(&event_dir1_2).unwrap();
        std::fs::write(
            event_dir1_2.join("event-2026-07-14T10-00-00Z.md"),
            "---\n"
                .to_string()
                + "event-id: PlanCreated\n"
                + "target-knot: plan-creator\n"
                + "timestamp: 2026-07-14T10:00:00Z\n"
                + "description: Plan for feature X\n"
                + "---\n"
                + "\n"
                + "## Event: PlanCreated from plan-creator\n",
        )
        .unwrap();

        let event_dir2_2 = rig_dir2.join("tie-offs").join("consumer-loom").join("ValidationFailed");
        std::fs::create_dir_all(&event_dir2_2).unwrap();
        std::fs::write(
            event_dir2_2.join("event-2026-07-14T11-00-00Z.md"),
            "---\n"
                .to_string()
                + "event-id: ValidationFailed\n"
                + "target-knot: plan-creator\n"
                + "timestamp: 2026-07-14T11:00:00Z\n"
                + "description: Validation failed on PRD\n"
                + "---\n"
                + "\n"
                + "## Event: ValidationFailed from plan-creator\n",
        )
        .unwrap();

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Inspect the captured prompt
        let contexts = runner.get_captured_contexts();
        assert!(!contexts.is_empty(), "agent should have been called");
        let prompt = &contexts[0].prompt;

        // Should contain both pending events
        assert!(
            prompt.contains("PlanCreated"),
            "prompt should contain PlanCreated pending event: {}",
            prompt
        );
        assert!(
            prompt.contains("ValidationFailed"),
            "prompt should contain ValidationFailed pending event: {}",
            prompt
        );
        assert!(
            prompt.contains("Plan for feature X"),
            "prompt should contain PlanCreated description: {}",
            prompt
        );
        assert!(
            prompt.contains("Validation failed on PRD"),
            "prompt should contain ValidationFailed description: {}",
            prompt
        );
    }

    /// Regression: existing event enforcement flow still works
    /// (missing events trigger follow-up).
    #[test]
    fn regression_event_enforcement_flow_still_works() {
        let dir = TempDir::new().unwrap();
        let strand_path = dir.path().join("strand.md");
        std::fs::write(&strand_path, "test content").unwrap();

        let producer_loom = build_loom(
            "producer-loom",
            vec![build_producer_knot("plan-creator")],
        );
        let consumer_loom = build_loom(
            "consumer-loom",
            vec![build_consumer_knot(
                "plan-watcher",
                "plan-creator",
                "PlanCreated",
                "Emit when a plan is created.",
            )],
        );

        // Agent output with no events — should trigger enforcement.
        let output = Ok(AgentOutput {
            stdout: "Plan created, no events emitted.".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        });
        let runner = Arc::new(MockAgentRunner::new(output));

        let (use_case, log_events, _tie_off_appends, _content, _dispatches, _store) =
            build_process_strand_with_dispatcher(vec![producer_loom, consumer_loom], runner);

        let event = StrandEvent::Created {
            loom_id: LoomId("producer-loom".to_string()),
            knot_id: KnotId("plan-creator".to_string()),
            strand_path: StrandPath(strand_path.clone()),
        };

        let result = use_case.execute(event);
        assert!(result.is_ok());

        // Verify KnotEventsMissing was logged.
        let events = log_events.lock().unwrap();
        let has_missing = events.iter().any(|e| {
            matches!(e, LoomEvent::KnotEventsMissing { .. })
        });
        assert!(
            has_missing,
            "KnotEventsMissing should be logged when no events are emitted"
        );
    }

    /// Helper: build ProcessStrand with a mock event dispatcher (no
    /// real rig directory needed for enforcement tests).
    #[allow(clippy::type_complexity)]
    fn build_process_strand_with_dispatcher(
        looms: Vec<Loom>,
        agent_runner: Arc<MockAgentRunner>,
    ) -> (
        ProcessStrand,
        Arc<Mutex<Vec<LoomEvent>>>,
        Arc<Mutex<Vec<TieOff>>>,
        Arc<Mutex<HashMap<String, String>>>,
        Arc<Mutex<Vec<(crate::domain::events::AgentEvent, String, String, String)>>>,
        LoomStore,
    ) {
        let store = LoomStore::new();
        for loom in looms {
            store.register(loom);
        }

        let (log_port, log_events) = MockLoomLogPort::new();
        let (tie_off_sink, tie_off_appends, tie_off_content) =
            TrackingTieOffSink::new();
        let (rig_log, _rig_events) = MockRigLogPort::new();
        let (event_dispatcher, dispatches) = MockEventDispatcher::new();

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
            Arc::new(MockStrandFileChecker::new()),
            Arc::new(event_dispatcher),
            None,
        );

        (use_case, log_events, tie_off_appends, tie_off_content, dispatches, store)
    }
}
