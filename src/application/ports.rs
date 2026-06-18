//! Application-layer port traits.
//!
//! Ports define the contracts that infrastructure adapters must satisfy.
//! The application layer orchestrates domain entities through these ports.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

use crate::domain::entities::{
    Knot, KnotId, Loom, LoomId, RigState, StrandPath, TieOff, TieOffPath,
};
use crate::domain::events::{LoomEvent, RigLogEvent};
use crate::domain::value_objects::AgentProfile;

// ── Error Types ────────────────────────────────────────────────────────────

/// Errors that can occur when calling port methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    /// A loom was not found in the repository.
    LoomNotFound(LoomId),
    /// Failed to scan a rig directory.
    RigScanFailed(String),
    /// Failed to save a loom to the repository.
    LoomSaveFailed(String),
    /// Failed to list registered looms.
    LoomListFailed(String),
    /// Failed to derive knot status from loom-log.
    KnotStatusDeriveFailed(String),
    /// Failed to open the loom activity log.
    LoomLogOpenFailed(String),
    /// Failed to append an event to the loom log.
    LoomLogAppendFailed(String),
    /// Failed to read events from the loom log.
    LoomLogReadFailed(String),
    /// Failed to watch a path for file events.
    EventWatchFailed(String),
    /// Failed to unwatch a path for file events.
    EventUnwatchFailed(String),
    /// Agent execution failed.
    AgentExecutionFailed(String),
    /// The agent CLI binary was not found.
    CommandNotFound(String),
    /// Agent execution exceeded the configured timeout.
    Timeout(String),
    /// Failed to write tie-off output.
    TieOffWriteFailed(String),
    /// An agent profile was not found.
    ProfileNotFound(String),
    /// Failed to scan the profiles directory.
    ProfileScanFailed(String),
    /// Failed to save a profile to disk.
    ProfileSaveFailed(String),
    /// Failed to write to the rig-log.
    RigLogWriteFailed(String),
    /// Failed to read from the rig-log.
    RigLogReadFailed(String),
    /// Failed to create a git commit.
    GitCommitFailed(String),
    /// Failed to write the state file.
    StateWriteFailed(String),
}

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortError::LoomNotFound(id) => {
                write!(f, "loom '{}' not found", id.0)
            }
            PortError::RigScanFailed(msg) => {
                write!(f, "rig scan failed: {msg}")
            }
            PortError::LoomSaveFailed(msg) => {
                write!(f, "loom save failed: {msg}")
            }
            PortError::LoomListFailed(msg) => {
                write!(f, "loom list failed: {msg}")
            }
            PortError::KnotStatusDeriveFailed(msg) => {
                write!(f, "knot status derive failed: {msg}")
            }
            PortError::LoomLogOpenFailed(msg) => {
                write!(f, "loom log open failed: {msg}")
            }
            PortError::LoomLogAppendFailed(msg) => {
                write!(f, "loom log append failed: {msg}")
            }
            PortError::LoomLogReadFailed(msg) => {
                write!(f, "loom log read failed: {msg}")
            }
            PortError::EventWatchFailed(msg) => {
                write!(f, "event watch failed: {msg}")
            }
            PortError::EventUnwatchFailed(msg) => {
                write!(f, "event unwatch failed: {msg}")
            }
            PortError::AgentExecutionFailed(msg) => {
                write!(f, "agent execution failed: {msg}")
            }
            PortError::CommandNotFound(msg) => {
                write!(f, "command not found: {msg}")
            }
            PortError::Timeout(msg) => {
                write!(f, "timeout: {msg}")
            }
            PortError::TieOffWriteFailed(msg) => {
                write!(f, "tie-off write failed: {msg}")
            }
            PortError::ProfileNotFound(name) => {
                write!(f, "agent profile '{name}' not found")
            }
            PortError::ProfileScanFailed(msg) => {
                write!(f, "profile scan failed: {msg}")
            }
            PortError::ProfileSaveFailed(msg) => {
                write!(f, "profile save failed: {msg}")
            }
            PortError::RigLogWriteFailed(msg) => {
                write!(f, "rig-log write failed: {msg}")
            }
            PortError::RigLogReadFailed(msg) => {
                write!(f, "rig-log read failed: {msg}")
            }
            PortError::GitCommitFailed(msg) => {
                write!(f, "git commit failed: {msg}")
            }
            PortError::StateWriteFailed(msg) => {
                write!(f, "state write failed: {msg}")
            }
        }
    }
}

impl std::error::Error for PortError {}

// ── Supporting Types ──────────────────────────────────────────────────────

/// Status of a knot's processing lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProcessingStatus {
    /// The knot is registered but not yet processing.
    Idle,
    /// The knot is currently processing a strand.
    Processing,
    /// Processing completed successfully.
    Completed,
    /// Processing failed with an error.
    Failed,
}

/// The type of event recorded in knot state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum KnotEventType {
    /// A new strand was created.
    Created,
    /// An existing strand was modified.
    Modified,
    /// A strand was deleted.
    Deleted,
}

/// Per-knot processing state.
///
/// Records the current status of a knot as it processes strands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct KnotState {
    /// The knot this state belongs to.
    pub knot_id: KnotId,
    /// The type of event that triggered processing.
    pub event_type: KnotEventType,
    /// Path to the strand being processed.
    pub strand_path: StrandPath,
    /// Path to the tie-off produced (if any).
    pub tie_off_path: Option<TieOffPath>,
    /// Current processing status.
    pub status: ProcessingStatus,
    /// Error message if processing failed.
    pub error: Option<String>,
    /// Timestamp of the last state update (stored as an ISO string).
    pub last_updated: String,
}

/// Context passed to the agent runner when executing a knot.
///
/// The optional `timeout` field allows per-knot timeout overrides.
/// When `None`, the runner's global default timeout is used.
/// When `Some(d)`, the agent session deadline is `d`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    /// Path to the agent CLI binary.
    pub cli_path: String,
    /// Arguments passed to the CLI.
    pub cli_args: Vec<String>,
    /// Prompt to send to the agent (knot instructions).
    pub prompt: String,
    /// Profile-level prompt segment (agent persona).
    ///
    /// Prepend to stdin before knot instructions and trigger line.
    pub profile_prompt: String,
    /// Path to the strand being processed.
    pub strand_path: StrandPath,
    /// The type of strand event (e.g. "Created", "Modified", "Deleted").
    pub event_type: String,
    /// Knot name for the trigger line in the prompt.
    pub knot_name: Option<String>,
    /// Per-context timeout override.
    ///
    /// When `Some(d)`, the agent runner uses `d` as the session deadline.
    /// When `None`, the runner falls back to its own global default timeout.
    pub timeout: Option<Duration>,
}

/// Output captured from agent execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOutput {
    /// Standard output from the agent.
    pub stdout: String,
    /// Standard error from the agent.
    pub stderr: String,
    /// Exit code from the agent process.
    pub exit_code: i32,
}

// ── Port Traits ────────────────────────────────────────────────────────────

/// Port for discovering and persisting looms.
///
/// An adapter must be able to scan a rig for looms, retrieve individual
/// looms, list all registered looms, and save loom definitions.
pub trait LoomRepository: Send + Sync {
    /// Scan a rig directory and return all discovered looms along with
    /// any knot parse warnings (unknown YAML properties in knot files).
    fn scan(&self, rig: &Path) -> Result<(Vec<Loom>, Vec<String>), PortError>;

    /// Scan a single loom directory for `.md` knot definition files.
    ///
    /// Returns parsed `Knot` instances with unresolved `strand_dir` paths
    /// (caller must resolve them relative to the project root), plus any
    /// parse warnings for unknown YAML properties.
    fn scan_knot_files(
        &self,
        loom_dir: &Path,
    ) -> Result<(Vec<Knot>, Vec<String>), PortError>;

    /// Get a single loom by its ID.
    fn get(&self, id: &LoomId) -> Result<Option<Loom>, PortError>;

    /// List all registered looms.
    fn list(&self) -> Result<Vec<Loom>, PortError>;

    /// Save a loom definition.
    fn save(&self, loom: Loom) -> Result<(), PortError>;
}



/// Port for appending and querying loom activity logs.
///
/// Records high-level loom events such as knot registration, loom
/// start/stop, and strand processing.
pub trait LoomLogPort: Send + Sync {
    /// Open or create the activity log for a loom.
    fn open(&self, loom_id: &LoomId) -> Result<(), PortError>;

    /// Append an event to the loom activity log.
    fn append(&self, event: LoomEvent) -> Result<(), PortError>;

    /// Read all events for a loom.
    fn read_all(&self, loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError>;
}

/// Port for watching directories for file system events.
///
/// Events flow through a channel (managed by the adapter), not via this
/// port. This port only registers and unregisters watched paths.
pub trait EventSource: Send + Sync {
    /// Start watching a directory for file events.
    fn watch(&self, path: &Path) -> Result<(), PortError>;

    /// Stop watching a directory.
    fn unwatch(&self, path: &Path) -> Result<(), PortError>;

    /// Associate loom and knot IDs with a source directory.
    ///
    /// Call this before `watch()` so emitted events carry the correct
    /// `loom_id` and `knot_id`. No-op for mock implementations.
    fn set_loom_ids(
        &self,
        _source_dir: &Path,
        _loom_id: &LoomId,
        _knot_id: &KnotId,
    ) {
    }

    /// Register a watch type for a directory path.
    ///
    /// Tells the adapter how to interpret events from this directory
    /// (strand, rig, or loom config events). No-op for mock
    /// implementations.
    fn register_watch(
        &self,
        _path: std::path::PathBuf,
        _watch_type: crate::adapters::outbound::event_source::WatchType,
    ) {
    }
}

/// Port for executing the agent CLI and capturing its output.
///
/// The runner enforces a session deadline. If `ExecutionContext::timeout`
/// is `Some(d)`, that value is used. Otherwise the runner falls back to
/// its own global default timeout configured at construction time.
pub trait AgentRunner: Send + Sync {
    /// Execute the agent CLI with the given context.
    ///
    /// The session deadline is determined as follows:
    /// 1. If `ctx.timeout` is `Some(d)`, use `d`.
    /// 2. Otherwise, use the runner's global default timeout.
    /// If the agent exceeds the deadline, it is killed and
    /// `PortError::Timeout` is returned.
    fn execute(&self, ctx: ExecutionContext) -> Result<AgentOutput, PortError>;
}

/// Port for writing tie-off content to disk.
pub trait TieOffSink: Send + Sync {
    /// Write tie-off output to its target location (overwrites existing file).
    fn write(&self, tie_off: TieOff) -> Result<(), PortError>;

    /// Append tie-off content as a new section with metadata header.
    ///
    /// If the file exists, a `---` delimiter and metadata header are
    /// prepended before the new content. If the file does not exist,
    /// it is created with the metadata header and content.
    fn append(&self, tie_off: TieOff) -> Result<(), PortError>;

    /// Read existing tie-off content at the given path.
    ///
    /// Returns an empty string if the file does not exist.
    fn read_content(&self, path: &TieOffPath) -> Result<String, PortError>;
}

/// Port for appending and querying the rig-log.
///
/// The rig-log is an append-only JSONL file at `rig/.rig-log` that records
/// serious operational events (timeouts, queue idle) so the user or an
/// external watcher can monitor and react.
pub trait RigLogPort: Send + Sync {
    /// Append a rig-log event.
    fn append(&self, event: RigLogEvent) -> Result<(), PortError>;

    /// Read all rig-log events.
    fn read_all(&self) -> Result<Vec<RigLogEvent>, PortError>;
}

/// Port for discovering and persisting agent profiles.
///
/// Profiles are stored as `.md` files in `{rig}/profiles/` with YAML
/// frontmatter. This port provides read/write/list/delete operations
/// for the shared agent profile entity.
pub trait AgentProfileRepository: Send + Sync {
    /// Get a single agent profile by name.
    ///
    /// Returns `Ok(None)` if the profile does not exist.
    fn get(&self, name: &str) -> Result<Option<AgentProfile>, PortError>;

    /// List all registered agent profiles.
    ///
    /// Returns an empty vector if no profiles exist.
    fn list(&self) -> Result<Vec<AgentProfile>, PortError>;

    /// Save an agent profile to disk.
    ///
    /// Creates the profiles directory if it does not exist.
    fn save(&self, profile: AgentProfile) -> Result<(), PortError>;

    /// Delete an agent profile by name.
    ///
    /// Returns `ProfileNotFound` if the profile does not exist.
    fn delete(&self, name: &str) -> Result<(), PortError>;
}

/// Port for creating git commits to version agent work.
///
/// After a successful knot run, the application layer calls this port
/// to create a commit in the project root. The commit message identifies
/// the loom, knot, strand, and event type. The commit body contains the
/// tie-off output. The port must gracefully handle non-git directories
/// (e.g., return `Ok(())` or a non-fatal error).
pub trait GitVersioningPort: Send + Sync {
    /// Create a git commit for a knot run.
    ///
    /// Arguments:
    /// - `loom_id` — identifier of the loom
    /// - `knot_id` — identifier of the knot
    /// - `strand_path` — path to the strand that was processed
    /// - `event_type` — type of strand event (Created/Modified/Deleted)
    /// - `tie_off_content` — the current response / tie-off output
    fn commit(
        &self,
        loom_id: &LoomId,
        knot_id: &KnotId,
        strand_path: &StrandPath,
        event_type: &str,
        tie_off_content: &str,
    ) -> Result<(), PortError>;
}

/// Port for writing the rig state snapshot file.
///
/// The state writer atomically writes `RigState` JSON to
/// `{rig_dir}/state.json` (write to `.state.json.tmp`, then rename).
/// This provides a file-first replacement for the HTTP interface.
pub trait StateWriterPort: Send + Sync {
    /// Write the given `RigState` to disk atomically.
    fn write_state(&self, state: &RigState) -> Result<(), PortError>;
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    // ── Mock Implementations ────────────────────────────────────────────

    /// In-memory mock of `LoomRepository`.
    #[derive(Default)]
    struct MockLoomRepository {
        looms: HashMap<LoomId, Loom>,
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(&self, _rig: &Path) -> Result<(Vec<Loom>, Vec<String>), PortError> {
            Ok((self.looms.values().cloned().collect(), Vec::new()))
        }

        fn scan_knot_files(
            &self,
            _loom_dir: &Path,
        ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
            Ok((vec![], vec![]))
        }

        fn get(&self, id: &LoomId) -> Result<Option<Loom>, PortError> {
            Ok(self.looms.get(id).cloned())
        }

        fn list(&self) -> Result<Vec<Loom>, PortError> {
            Ok(self.looms.values().cloned().collect())
        }

        fn save(&self, _loom: Loom) -> Result<(), PortError> {
            Ok(())
        }
    }



    /// In-memory mock of `LoomLogPort`.
    #[derive(Default)]
    struct MockLoomLogPort {
        events: Vec<LoomEvent>,
    }

    impl LoomLogPort for MockLoomLogPort {
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

    /// Mock of `EventSource` that never errors.
    #[derive(Default)]
    struct MockEventSource;

    impl EventSource for MockEventSource {
        fn watch(&self, _path: &Path) -> Result<(), PortError> {
            Ok(())
        }

        fn unwatch(&self, _path: &Path) -> Result<(), PortError> {
            Ok(())
        }
    }

    /// Mock of `AgentRunner` that returns deterministic output.
    #[derive(Default)]
    struct MockAgentRunner;

    impl AgentRunner for MockAgentRunner {
        fn execute(&self, _ctx: ExecutionContext) -> Result<AgentOutput, PortError> {
            Ok(AgentOutput {
                stdout: "mock output".to_string(),
                stderr: String::new(),
                exit_code: 0,
            })
        }
    }

    /// Mock of `TieOffSink` that never errors.
    #[derive(Default)]
    struct MockTieOffSink {
        content: std::sync::RwLock<std::collections::HashMap<String, String>>,
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
            Ok(self.content
                .read()
                .unwrap()
                .get(&path.0.display().to_string())
                .cloned()
                .unwrap_or_default())
        }
    }

    /// In-memory mock of `RigLogPort`.
    #[derive(Default)]
    struct MockRigLogPort {
        events: std::sync::Mutex<Vec<RigLogEvent>>,
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

    /// In-memory mock of `AgentProfileRepository`.
    #[derive(Default)]
    struct MockAgentProfileRepository {
        profiles: std::sync::RwLock<HashMap<String, AgentProfile>>,
    }

    impl AgentProfileRepository for MockAgentProfileRepository {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .read()
                .unwrap()
                .get(name)
                .cloned())
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .read()
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
                .write()
                .unwrap()
                .insert(profile.name.clone(), profile);
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<(), PortError> {
            let mut map = self.profiles.write().unwrap();
            if map.remove(name).is_none() {
                return Err(PortError::ProfileNotFound(name.to_string()));
            }
            Ok(())
        }
    }

    /// In-memory mock of `GitVersioningPort`.
    ///
    /// Records all commit calls for inspection in tests.
    #[derive(Default)]
    struct MockGitVersioningPort {
        commits: std::sync::Mutex<Vec<(LoomId, KnotId, String, String, String)>>,
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
            Ok(())
        }
    }

    /// In-memory mock of `StateWriterPort`.
    ///
    /// Records all write calls for inspection in tests.
    #[derive(Default)]
    struct MockStateWriter {
        writes: std::sync::Mutex<Vec<RigState>>,
    }

    impl StateWriterPort for MockStateWriter {
        fn write_state(&self, state: &RigState) -> Result<(), PortError> {
            self.writes.lock().unwrap().push(state.clone());
            Ok(())
        }
    }

    // ── Contract Tests ──────────────────────────────────────────────────

    #[test]
    fn loom_repository_contract() {
        let repo = MockLoomRepository::default();

        // Verify trait is object-safe by using a trait object
        let _obj: &dyn LoomRepository = &repo;

        // Verify all trait methods compile and are callable
        let rig = Path::new("/tmp/rig");
        let (looms, warnings) = repo.scan(rig).unwrap();
        assert!(looms.is_empty());
        assert!(warnings.is_empty());

        let loom_id = LoomId("test".to_string());
        let get_result = repo.get(&loom_id);
        assert!(get_result.is_ok());
        assert!(get_result.unwrap().is_none());

        let list_result = repo.list();
        assert!(list_result.is_ok());
        assert!(list_result.unwrap().is_empty());

        let loom = Loom {
            id: LoomId("save-test-loom".to_string()),
            knots: vec![],
        };
        let save_result = repo.save(loom);
        assert!(save_result.is_ok());
    }



    #[test]
    fn loom_log_port_contract() {
        let port = MockLoomLogPort::default();

        // Verify trait is object-safe
        let _obj: &dyn LoomLogPort = &port;

        // Verify all trait methods compile and are callable
        let loom_id = LoomId("log-test".to_string());

        let open_result = port.open(&loom_id);
        assert!(open_result.is_ok());

        let event = LoomEvent::LoomStarted {
            loom_id: loom_id.clone(),
            timestamp: "2026-06-10T12:00:00Z".to_string(),
        };
        let append_result = port.append(event);
        assert!(append_result.is_ok());

        let read_result = port.read_all(&loom_id);
        assert!(read_result.is_ok());
    }

    #[test]
    fn agent_runner_contract() {
        let runner = MockAgentRunner;

        // Verify trait is object-safe
        let _obj: &dyn AgentRunner = &runner;

        // Verify ExecutionContext and AgentOutput types exist and work
        let ctx = ExecutionContext {
            cli_path: "/usr/bin/pi".to_string(),
            cli_args: vec!["--verbose".to_string()],
            prompt: "Review this document".to_string(),
            profile_prompt: "You are a reviewer.".to_string(),
            strand_path: StrandPath(PathBuf::from("doc.md")),
            event_type: "Created".to_string(),
            knot_name: Some("review".to_string()),
            timeout: None,
        };
        let result = runner.execute(ctx);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert_eq!(output.exit_code, 0);
        assert!(!output.stdout.is_empty());
    }

    #[test]
    fn tieoff_sink_contract() {
        let sink = MockTieOffSink::default();

        // Verify trait is object-safe
        let _obj: &dyn TieOffSink = &sink;

        // Verify write method compiles and is callable
        let tie_off = TieOff {
            content: "Generated content".to_string(),
            path: TieOffPath(PathBuf::from("output/review.md")),
            status: crate::domain::entities::TieOffStatus::Produced,
            knot_name: None,
            event_type: None,
            strand_path: None,
            timestamp: None,
        };
        let result = sink.write(tie_off);
        assert!(result.is_ok());
    }

    #[test]
    fn event_source_contract() {
        let source = MockEventSource;

        // Verify trait is object-safe
        let _obj: &dyn EventSource = &source;

        // Verify watch/unwatch methods compile and are callable
        let path = Path::new("/tmp/watched");
        assert!(source.watch(path).is_ok());
        assert!(source.unwatch(path).is_ok());
    }

    #[test]
    fn rig_log_port_contract() {
        let port = MockRigLogPort::default();

        // Verify trait is object-safe
        let _obj: &dyn RigLogPort = &port;

        // Verify append and read_all work
        let event = RigLogEvent::QueueIdle {
            timestamp: "2026-06-14T10:00:00Z".to_string(),
        };
        assert!(port.append(event.clone()).is_ok());

        let events = port.read_all().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], event);

        // Append a second event
        let event2 = RigLogEvent::TimeoutExceeded {
            loom_id: LoomId("test".to_string()),
            knot_id: KnotId("k1".to_string()),
            strand_path: StrandPath(PathBuf::from("input.md")),
            error: "deadline exceeded".to_string(),
            timestamp: "2026-06-14T10:01:00Z".to_string(),
        };
        assert!(port.append(event2).is_ok());

        let events = port.read_all().unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn port_error_rig_log_variants_display() {
        let err = PortError::RigLogWriteFailed("disk full".to_string());
        assert_eq!(err.to_string(), "rig-log write failed: disk full");

        let err = PortError::RigLogReadFailed("file not found".to_string());
        assert_eq!(err.to_string(), "rig-log read failed: file not found");
    }

    #[test]
    fn port_error_rig_log_variants_are_std_error() {
        let err = PortError::RigLogWriteFailed("io".to_string());
        let _: &dyn std::error::Error = &err;

        let err = PortError::RigLogReadFailed("io".to_string());
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn agent_profile_repository_contract() {
        let repo = MockAgentProfileRepository::default();

        // Verify trait is object-safe
        let _obj: &dyn AgentProfileRepository = &repo;

        // Verify all trait methods compile and are callable
        let get_result = repo.get("nonexistent");
        assert!(get_result.is_ok());
        assert!(get_result.unwrap().is_none());

        let list_result = repo.list();
        assert!(list_result.is_ok());
        assert!(list_result.unwrap().is_empty());

        // Save and retrieve
        let profile = AgentProfile::new(
            "test-profile".to_string(),
            "openai".to_string(),
            "gpt-4o".to_string(),
            "You are a test.".to_string(),
        )
        .unwrap();
        let save_result = repo.save(profile.clone());
        assert!(save_result.is_ok());

        let get_result = repo.get("test-profile");
        assert!(get_result.is_ok());
        assert!(get_result.as_ref().unwrap().is_some());
        assert_eq!(
            get_result.unwrap().unwrap().name,
            "test-profile"
        );

        // List should return the saved profile
        let list_result = repo.list();
        assert!(list_result.is_ok());
        assert_eq!(list_result.unwrap().len(), 1);

        // Delete the profile
        let delete_result = repo.delete("test-profile");
        assert!(delete_result.is_ok());

        // Verify it's gone
        let get_result = repo.get("test-profile");
        assert!(get_result.is_ok());
        assert!(get_result.unwrap().is_none());

        // Delete non-existent profile returns error
        let delete_result = repo.delete("nonexistent");
        assert!(delete_result.is_err());
        assert_eq!(
            delete_result.unwrap_err(),
            PortError::ProfileNotFound("nonexistent".to_string())
        );
    }

    // ── Supporting Type Tests ───────────────────────────────────────────

    #[test]
    fn git_versioning_port_contract() {
        let port = MockGitVersioningPort::default();

        // Verify trait is object-safe
        let _obj: &dyn GitVersioningPort = &port;

        // Verify commit method compiles and is callable
        let loom_id = LoomId("test-loom".to_string());
        let knot_id = KnotId("k1".to_string());
        let strand = StrandPath(PathBuf::from("input/strand.md"));
        let result = port.commit(
            &loom_id,
            &knot_id,
            &strand,
            "Created",
            "tie-off output here",
        );
        assert!(result.is_ok());

        // Verify the commit was recorded
        let commits = port.commits.lock().unwrap();
        assert_eq!(commits.len(), 1);
        let (lid, kid, sp, et, content) = &commits[0];
        assert_eq!(*lid, loom_id);
        assert_eq!(*kid, knot_id);
        assert_eq!(sp, "input/strand.md");
        assert_eq!(et, "Created");
        assert_eq!(content, "tie-off output here");
    }

    #[test]
    fn port_error_git_commit_display() {
        let err = PortError::GitCommitFailed("not a git repo".to_string());
        assert_eq!(err.to_string(), "git commit failed: not a git repo");
    }

    #[test]
    fn state_writer_port_contract() {
        let writer = MockStateWriter::default();

        // Verify trait is object-safe
        let _obj: &dyn StateWriterPort = &writer;

        // Verify write_state compiles and is callable
        let state = RigState {
            rig_path: "/tmp/rig".to_string(),
            looms: vec![],
            profiles: vec![],
            updated_at: "2026-06-18T00:00:00Z".to_string(),
        };
        let result = writer.write_state(&state);
        assert!(result.is_ok());

        // Verify the write was recorded
        let writes = writer.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].rig_path, "/tmp/rig");
    }

    #[test]
    fn port_error_state_write_display() {
        let err = PortError::StateWriteFailed("permission denied".to_string());
        assert_eq!(err.to_string(), "state write failed: permission denied");
    }

    #[test]
    fn knot_state_fields() {
        let state = KnotState {
            knot_id: KnotId("k1".to_string()),
            event_type: KnotEventType::Modified,
            strand_path: StrandPath(PathBuf::from("input.md")),
            tie_off_path: None,
            status: ProcessingStatus::Processing,
            error: Some("timeout".to_string()),
            last_updated: "2026-06-03T12:00:00Z".to_string(),
        };

        assert_eq!(state.knot_id, KnotId("k1".to_string()));
        assert_eq!(state.event_type, KnotEventType::Modified);
        assert_eq!(state.status, ProcessingStatus::Processing);
        assert_eq!(state.error.as_deref(), Some("timeout"));
        assert!(state.tie_off_path.is_none());
    }

    #[test]
    fn execution_context_fields() {
        let ctx = ExecutionContext {
            cli_path: "pi".to_string(),
            cli_args: vec!["--mode".to_string(), "stream".to_string()],
            prompt: "Process this file".to_string(),
            profile_prompt: "You are an agent.".to_string(),
            strand_path: StrandPath(PathBuf::from("src/main.rs")),
            event_type: "Created".to_string(),
            knot_name: Some("review".to_string()),
            timeout: None,
        };

        assert_eq!(ctx.cli_path, "pi");
        assert_eq!(ctx.cli_args.len(), 2);
        assert!(!ctx.prompt.is_empty());
        assert!(!ctx.profile_prompt.is_empty());
        assert_eq!(ctx.event_type, "Created");
        assert_eq!(ctx.knot_name.as_deref(), Some("review"));
        assert!(ctx.timeout.is_none());
    }

    #[test]
    fn agent_output_fields() {
        let output = AgentOutput {
            stdout: "done".to_string(),
            stderr: "warning: slow".to_string(),
            exit_code: 0,
        };

        assert_eq!(output.exit_code, 0);
        assert!(!output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }

    #[test]
    fn port_error_display() {
        let loom_id = LoomId("missing".to_string());
        let err = PortError::LoomNotFound(loom_id);
        assert_eq!(err.to_string(), "loom 'missing' not found");

        let err = PortError::RigScanFailed("permission denied".to_string());
        assert_eq!(err.to_string(), "rig scan failed: permission denied");

        let err = PortError::KnotStatusDeriveFailed("log empty".to_string());
        assert_eq!(err.to_string(), "knot status derive failed: log empty");

        let err = PortError::AgentExecutionFailed("crash".to_string());
        assert_eq!(err.to_string(), "agent execution failed: crash");

        let err = PortError::TieOffWriteFailed("disk full".to_string());
        assert_eq!(err.to_string(), "tie-off write failed: disk full");
    }

    #[test]
    fn port_error_is_std_error() {
        let err = PortError::LoomNotFound(LoomId("x".to_string()));
        // Verify it implements std::error::Error
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn processing_status_variants() {
        assert!(matches!(
            ProcessingStatus::Idle,
            ProcessingStatus::Idle
        ));
        assert!(matches!(
            ProcessingStatus::Processing,
            ProcessingStatus::Processing
        ));
        assert!(matches!(
            ProcessingStatus::Completed,
            ProcessingStatus::Completed
        ));
        assert!(matches!(
            ProcessingStatus::Failed,
            ProcessingStatus::Failed
        ));
    }

    #[test]
    fn knot_event_type_variants() {
        assert!(matches!(
            KnotEventType::Created,
            KnotEventType::Created
        ));
        assert!(matches!(
            KnotEventType::Modified,
            KnotEventType::Modified
        ));
        assert!(matches!(KnotEventType::Deleted, KnotEventType::Deleted));
    }
}
