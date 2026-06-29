//! Use case: process a single strand event through the agent pipeline.

use std::path::PathBuf;
use std::sync::Arc;

use crate::adapters::logging;
use crate::application::ports::{
    AgentProfileRepository, AgentRunner, GitVersioningPort, KnotEventType,
    LoomLogPort, PortError, RigLogPort, TieOffSink,
};
use crate::application::session_resume;
use crate::application::store::LoomStore;
use crate::domain::entities::{
    Knot, KnotId, Loom, LoomId, StrandCheckResult, StrandFileChecker,
    StrandPath, TieOff, TieOffOutcome, TieOffPath,
};
use crate::domain::events::{LoomEvent, StrandEvent};
use crate::domain::knot_file::derive_tieoff_path;
use crate::domain::value_objects::{AgentConfig, RigAgentConfig};

// Re-export shared types from types module
use super::types::format_timestamp;

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
    /// Strand file checker for text/binary/temp detection.
    file_checker: Arc<dyn StrandFileChecker>,
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
        }
    }

    /// Resolve the effective `AgentConfig` for a knot and the profile's
    /// session timeout.
    ///
    /// Loads the profile from the repository and delegates the
    /// profile→config mapping to `AgentProfile::resolve_for_knot()`.
    /// The profile's `profile_prompt` is delivered via stdin
    /// (not `--system-prompt`), so it is not merged here.
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

        let config = profile.resolve_for_knot(knot);
        let timeout = profile.session_timeout();

        Ok((config, timeout))
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

        // Strand file check: skip binary/temp/missing files.
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
                return Ok(());
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
                return Ok(());
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
                return Ok(());
            }
        }

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

        // Build the prompt. For Deleted events, use domain method
        // Knot::deleted_prompt() which composes the deletion notice
        // and scoped strand history.
        let prompt = if is_deleted {
            let sections = strand_history
                .as_deref()
                .unwrap_or_default();
            knot.deleted_prompt(&strand_filename, sections)
        } else {
            knot.prompt_template.instructions.clone()
        };

        // 3. Execute agent with session-resume retry logic.
        // Build CLI args here (same as execute_with_config default impl)
        // so session_resume can append --session-id on retry.
        let strand_file_ref = if is_deleted {
            None
        } else {
            Some(strand_path.clone())
        };
        let mut session_id: Option<String> = None;
        let result = session_resume::execute_with_resume(
            &*self.agent_runner,
            &*self.log_port,
            &loom_id,
            &knot_id,
            &strand_path,
            &mut session_id,
            agent_config,
            prompt,
            strand_file_ref,
            profile.profile_prompt,
            event_label.clone(),
            Some(knot.id.0.clone()),
            profile_timeout,
        );

        // Derive outcome from execution result — domain rule.
        let outcome = TieOffOutcome::derive(result);

        // Write tie-off (skipped for timeout).
        if outcome.should_write_tie_off() {
            let tie_off = TieOff {
                content: outcome.tie_off_content().unwrap_or_default(),
                path: tie_off_path.clone(),
                status: outcome
                    .tie_off_status()
                    .unwrap_or(crate::domain::entities::TieOffStatus::Produced),
                knot_name: Some(knot.id.0.clone()),
                event_type: Some(event_label.clone()),
                strand_path: Some(strand_path.0.display().to_string()),
                timestamp: None,
            };
            let _ = self.tie_off_sink.append(tie_off);
        }

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
                // Git versioning commit (best-effort, non-fatal).
                // Runs AFTER loom-log appends so the commit captures
                // the tie-off, KnotCompleted, and StrandProcessed entries.
                if knot.git_versioned {
                    if let Some(ref content) = outcome.tie_off_content() {
                        let commit_result = self.git_versioning_port.commit(
                            &loom_id,
                            &knot_id,
                            &strand_path,
                            &event_label,
                            content,
                        );
                        if let Err(ref e) = commit_result {
                            logging::log_strand_event(
                                &format!("git commit warning: {}", e),
                                &strand_path.0,
                            );
                        }
                    }
                }

                // Append KnotCompleted to loom-log.
                self.log_port.append(LoomEvent::KnotCompleted {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    strand_path: strand_path.clone(),
                    tie_off_path: tie_off_path.clone(),
                    timestamp: format_timestamp(),
                })?;

                // Append StrandProcessed to loom-log.
                self.log_port.append(LoomEvent::StrandProcessed {
                    loom_id: loom_id.clone(),
                    strand_path: strand_path.clone(),
                    error: None,
                    timestamp: format_timestamp(),
                })?;

                logging::log_strand_event(
                    &format!("{} completed (knot={})", strand_kind, knot_id.0),
                    &strand_path.0,
                );
            }
            _ => {
                // KnotFailed (error or timeout).
                let error_msg = outcome
                    .error_message()
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                self.log_port.append(LoomEvent::KnotFailed {
                    loom_id: loom_id.clone(),
                    knot_id: knot_id.clone(),
                    strand_path: strand_path.clone(),
                    error: error_msg.clone(),
                    timestamp: format_timestamp(),
                })?;

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
            }
        }

        Ok(())
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
        MockRigLogPort, MockStrandFileChecker, MockTieOffSink,
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
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
mod execution_test_shared {
    use super::*;
    use crate::domain::events::RigLogEvent;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::super::test_fixtures::{
        build_knot_with_profile, build_loom, default_profile,
        MockAgentRunner, MockGitVersioningPort, MockLoomLogPort,
        MockProfileRepository, MockRigLogPort, MockStrandFileChecker,
        TrackingTieOffSink,
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
        let runner = Arc::new(MockAgentRunner::new_sequence(responses));

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
        MockRigLogPort, MockStrandFileChecker, MockTieOffSink,
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(MockAgentRunner::default()),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo.clone(),
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
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
            Arc::new(MockLoomLogPort::default()),
            Arc::new(runner),
            Arc::new(MockTieOffSink::default()),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            Arc::new(MockGitVersioningPort::default()),
            Arc::new(MockStrandFileChecker::new()),
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
        MockRigLogPort, MockStrandFileChecker, MockTieOffSink,
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
        MockAgentRunner, MockGitVersioningPort, MockLoomLogPort,
        MockProfileRepository, MockRigLogPort, MockStrandFileChecker,
        MockTieOffSink, TrackingAgentRunner,
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
            strand_dir: PathBuf::from("strands"),
            git_versioned: true,
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
        MockRigLogPort, MockStrandFileChecker, MockTieOffSink,
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
        MockRigLogPort, MockStrandFileChecker, TrackingTieOffSink,
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
