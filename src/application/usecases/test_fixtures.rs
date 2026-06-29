//! Shared test fixtures for use case tests.
//!
//! Contains mock implementations and domain builders that are reused
//! across multiple test modules. Extracted to eliminate duplication.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use crate::adapters::outbound::event_source::WatchType;
use crate::application::ports::{
    AgentOutput, AgentProfileRepository, AgentRunner,
    ExecutionContext, EventSource, GitVersioningPort, LoomLogPort,
    LoomRepository, PortError, RigLogPort, TieOffSink,
};
use crate::domain::entities::{
    Knot, KnotId, Loom, LoomId, StrandPath, TieOff, TieOffPath,
};
use crate::domain::events::{LoomEvent, RigLogEvent};
use crate::domain::value_objects::{AgentConfig, AgentProfile, PromptTemplate};

// ── Tracking EventSource ───────────────────────────────────────────────────

/// A mock [`EventSource`] that records all `watch()`, `unwatch()`,
/// and `set_loom_ids()` calls for inspection in tests.
pub struct TrackingEventSource {
    watch_calls: Arc<Mutex<Vec<PathBuf>>>,
    unwatch_calls: Arc<Mutex<Vec<PathBuf>>>,
    set_ids_calls: Arc<Mutex<Vec<(PathBuf, LoomId, KnotId)>>>,
}

impl TrackingEventSource {
    #[allow(clippy::type_complexity)]
    pub fn new(
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
    fn watch(&self, path: &Path) -> Result<(), PortError> {
        self.watch_calls
            .lock()
            .unwrap()
            .push(path.to_path_buf());
        Ok(())
    }

    fn unwatch(&self, path: &Path) -> Result<(), PortError> {
        self.unwatch_calls
            .lock()
            .unwrap()
            .push(path.to_path_buf());
        Ok(())
    }

    fn unwatch_with_type(
        &self,
        path: &Path,
        _watch_type: WatchType,
    ) -> Result<(), PortError> {
        self.unwatch(path)
    }

    fn set_loom_ids(&self, source_dir: &Path, loom_id: &LoomId, knot_id: &KnotId) {
        self.set_ids_calls
            .lock()
            .unwrap()
            .push((source_dir.to_path_buf(), loom_id.clone(), knot_id.clone()));
    }
}

// ── Mock LoomLogPort ───────────────────────────────────────────────────────

/// A mock [`LoomLogPort`] that records all appended events.
///
/// No-op for `open()`. `read_all()` returns all recorded events.
pub struct MockLoomLogPort {
    events: Arc<Mutex<Vec<LoomEvent>>>,
}

impl MockLoomLogPort {
    pub fn new() -> (Self, Arc<Mutex<Vec<LoomEvent>>>) {
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

    fn read_all(&self, _loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
        Ok(self.events.lock().unwrap().clone())
    }
}

// ── Mock LoomRepository ────────────────────────────────────────────────────

/// A mock [`LoomRepository`] with configurable scan results.
///
/// Defaults to empty scan. Set `scan_looms`, `scan_warnings`, and
/// `scan_knots` via the provided `Arc<Mutex<...>>` handles.
pub struct MockLoomRepository {
    scan_looms: Arc<Mutex<Vec<Loom>>>,
    scan_warnings: Arc<Mutex<Vec<String>>>,
    scan_knots: Arc<Mutex<Vec<Knot>>>,
}

impl MockLoomRepository {
    #[allow(clippy::type_complexity)]
    pub fn new(
    ) -> (
        Self,
        Arc<Mutex<Vec<Loom>>>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Vec<Knot>>>,
    ) {
        let scan_looms = Arc::new(Mutex::new(vec![]));
        let scan_warnings = Arc::new(Mutex::new(vec![]));
        let scan_knots = Arc::new(Mutex::new(vec![]));
        let repo = Self {
            scan_looms: scan_looms.clone(),
            scan_warnings: scan_warnings.clone(),
            scan_knots: scan_knots.clone(),
        };
        (repo, scan_looms, scan_warnings, scan_knots)
    }
}

impl LoomRepository for MockLoomRepository {
    fn scan(
        &self,
        _rig: &Path,
    ) -> Result<(Vec<Loom>, Vec<String>), PortError> {
        Ok((
            self.scan_looms.lock().unwrap().clone(),
            self.scan_warnings.lock().unwrap().clone(),
        ))
    }

    fn scan_knot_files(
        &self,
        _loom_dir: &Path,
    ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
        Ok((
            self.scan_knots.lock().unwrap().clone(),
            self.scan_warnings.lock().unwrap().clone(),
        ))
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

// ── Mock AgentRunner ───────────────────────────────────────────────────────

/// Configurable mock [`AgentRunner`] that returns a programmable result
/// and captures [`ExecutionContext`] for inspection.
///
/// Supports two modes:
/// - Single result (via [`new()`]) — returns the same result for every call.
/// - Sequence (via [`new_sequence()`]) — pops results from a queue for each
///   call, useful for testing session-resume retry behaviour.
pub struct MockAgentRunner {
    result: Arc<Mutex<Result<AgentOutput, PortError>>>,
    sequence: Arc<Mutex<Option<VecDeque<Result<AgentOutput, PortError>>>>>,
    captured_ctx: Arc<Mutex<Vec<ExecutionContext>>>,
}

impl MockAgentRunner {
    /// Create a runner that always returns the given result.
    pub fn new(result: Result<AgentOutput, PortError>) -> Self {
        Self {
            result: Arc::new(Mutex::new(result)),
            sequence: Arc::new(Mutex::new(None)),
            captured_ctx: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Create a runner that returns results from the queue in order.
    pub fn new_sequence(results: Vec<Result<AgentOutput, PortError>>) -> Self {
        Self {
            result: Arc::new(Mutex::new(Ok(AgentOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
                metadata: None,
            }))),
            sequence: Arc::new(Mutex::new(Some(results.into_iter().collect()))),
            captured_ctx: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Override the single-result mode at runtime.
    pub fn set_result(&self, result: Result<AgentOutput, PortError>) {
        *self.result.lock().unwrap() = result;
    }

    /// Return the last captured execution context (if any).
    pub fn get_captured_ctx(&self) -> Option<ExecutionContext> {
        self.captured_ctx.lock().unwrap().last().cloned()
    }

    /// Return all captured execution contexts in order.
    pub fn get_captured_contexts(&self) -> Vec<ExecutionContext> {
        self.captured_ctx.lock().unwrap().clone()
    }
}

impl AgentRunner for MockAgentRunner {
    fn execute(&self, ctx: ExecutionContext) -> Result<AgentOutput, PortError> {
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
        let mut config = agent_config.clone();
        let strand_filename = strand_path
            .0
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let session_title = format!(
            "{} triggered by {} on {}",
            knot_name.as_deref().unwrap_or("unknown"),
            event_type,
            strand_filename,
        );
        config.extra_args.push("--name".to_string());
        config.extra_args.push(session_title);
        if let Some(ref file_path) = strand_file_ref {
            config.extra_args.push(format!("@{}", file_path.0.display()));
        }
        let ctx = ExecutionContext {
            agent_config: config,
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

// ── Mock TieOffSink ────────────────────────────────────────────────────────

/// A mock [`TieOffSink`] that records content by path.
///
/// Supports `write`, `append`, and `read_content`. Both `write` and
/// `append` store content keyed by the tie-off path display string.
pub struct MockTieOffSink {
    content: RwLock<HashMap<String, String>>,
}

impl MockTieOffSink {
    pub fn new() -> Self {
        Self {
            content: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MockTieOffSink {
    fn default() -> Self {
        Self::new()
    }
}

impl TieOffSink for MockTieOffSink {
    fn write(&self, tie_off: TieOff) -> Result<(), PortError> {
        self.content
            .write()
            .unwrap()
            .insert(tie_off.path.0.display().to_string(), tie_off.content);
        Ok(())
    }

    fn append(&self, tie_off: TieOff) -> Result<(), PortError> {
        self.write(tie_off)
    }

    fn read_content(&self, path: &TieOffPath) -> Result<String, PortError> {
        Ok(self
            .content
            .read()
            .unwrap()
            .get(&path.0.display().to_string())
            .cloned()
            .unwrap_or_default())
    }
}

// ── Mock RigLogPort ────────────────────────────────────────────────────────

/// A mock [`RigLogPort`] that records all appended events.
pub struct MockRigLogPort {
    events: Arc<Mutex<Vec<RigLogEvent>>>,
}

impl MockRigLogPort {
    pub fn new() -> (Self, Arc<Mutex<Vec<RigLogEvent>>>) {
        let events = Arc::new(Mutex::new(vec![]));
        (Self { events: events.clone() }, events)
    }
}

impl RigLogPort for MockRigLogPort {
    fn append(&self, event: RigLogEvent) -> Result<(), PortError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<RigLogEvent>, PortError> {
        Ok(self.events.lock().unwrap().clone())
    }
}

// ── Mock AgentProfileRepository ────────────────────────────────────────────

/// A mock [`AgentProfileRepository`] backed by a map of name → profile.
pub struct MockProfileRepository {
    profiles: Arc<Mutex<HashMap<String, AgentProfile>>>,
}

impl MockProfileRepository {
    pub fn new(profiles: HashMap<String, AgentProfile>) -> Self {
        Self {
            profiles: Arc::new(Mutex::new(profiles)),
        }
    }
}

impl Default for MockProfileRepository {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

impl AgentProfileRepository for MockProfileRepository {
    fn get(&self, name: &str) -> Result<Option<AgentProfile>, PortError> {
        Ok(self.profiles.lock().unwrap().get(name).cloned())
    }

    fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
        Ok(self.profiles.lock().unwrap().values().cloned().collect())
    }
}

// ── Mock GitVersioningPort ─────────────────────────────────────────────────

/// A mock [`GitVersioningPort`] that records all commit calls.
///
/// Supports forcing an error via `set_error()` for error-path testing.
pub struct MockGitVersioningPort {
    commits: Arc<Mutex<Vec<(LoomId, KnotId, String, String, String)>>>,
    force_error: Arc<Mutex<Option<PortError>>>,
}

impl MockGitVersioningPort {
    pub fn new(
    ) -> (
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

    /// Force the next (and all subsequent) `commit()` calls to return an error.
    pub fn set_error(&self, error: PortError) {
        *self.force_error.lock().unwrap() = Some(error);
    }
}

impl Default for MockGitVersioningPort {
    fn default() -> Self {
        let (self_, _commits) = Self::new();
        self_
    }
}

impl GitVersioningPort for MockGitVersioningPort {
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

// ── Domain Builders ────────────────────────────────────────────────────────

/// Build a knot with the given ID and default values.
///
/// Defaults: `agent_profile_ref: "fast"`, `prompt_template.instructions:
/// "check it"`, `strand_dir: "strands"`, `git_versioned: true`.
pub fn build_knot(id: impl Into<String>) -> Knot {
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
pub fn build_knot_with_strand_dir(id: impl Into<String>, strand_dir: PathBuf) -> Knot {
    let mut knot = build_knot(id);
    knot.strand_dir = strand_dir;
    knot
}

/// Build a loom with the given ID and knots.
pub fn build_loom(id: impl Into<String>, knots: Vec<Knot>) -> Loom {
    Loom {
        id: LoomId(id.into()),
        knots,
    }
}
