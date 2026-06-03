//! Application-layer port traits.
//!
//! Ports define the contracts that infrastructure adapters must satisfy.
//! The application layer orchestrates domain entities through these ports.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::domain::entities::{KnotId, Loom, LoomId, StrandPath, TieOff, TieOffPath};
use crate::domain::events::LoomEvent;

// ── Error Types ────────────────────────────────────────────────────────────

/// Errors that can occur when calling port methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    /// A loom was not found in the repository.
    LoomNotFound(LoomId),
    /// Failed to scan a workspace directory.
    WorkspaceScanFailed(String),
    /// Failed to save a loom to the repository.
    LoomSaveFailed(String),
    /// Failed to list registered looms.
    LoomListFailed(String),
    /// Failed to create knot processing state.
    KnotStateCreateFailed(String),
    /// Failed to update knot processing state.
    KnotStateUpdateFailed(String),
    /// Failed to read knot processing state.
    KnotStateGetFailed(String),
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
}

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortError::LoomNotFound(id) => {
                write!(f, "loom '{}' not found", id.0)
            }
            PortError::WorkspaceScanFailed(msg) => {
                write!(f, "workspace scan failed: {msg}")
            }
            PortError::LoomSaveFailed(msg) => {
                write!(f, "loom save failed: {msg}")
            }
            PortError::LoomListFailed(msg) => {
                write!(f, "loom list failed: {msg}")
            }
            PortError::KnotStateCreateFailed(msg) => {
                write!(f, "knot state create failed: {msg}")
            }
            PortError::KnotStateUpdateFailed(msg) => {
                write!(f, "knot state update failed: {msg}")
            }
            PortError::KnotStateGetFailed(msg) => {
                write!(f, "knot state get failed: {msg}")
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
        }
    }
}

impl std::error::Error for PortError {}

// ── Supporting Types ──────────────────────────────────────────────────────

/// Status of a knot's processing lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    /// Path to the agent CLI binary.
    pub cli_path: String,
    /// Arguments passed to the CLI.
    pub cli_args: Vec<String>,
    /// Prompt to send to the agent.
    pub prompt: String,
    /// Path to the strand being processed.
    pub strand_path: StrandPath,
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
/// An adapter must be able to scan a workspace for looms, retrieve individual
/// looms, list all registered looms, and save loom definitions.
pub trait LoomRepository {
    /// Scan a workspace directory and return all discovered looms.
    fn scan(&self, workspace: &Path) -> Result<Vec<Loom>, PortError>;

    /// Get a single loom by its ID.
    fn get(&self, id: &LoomId) -> Result<Option<Loom>, PortError>;

    /// List all registered looms.
    fn list(&self) -> Result<Vec<Loom>, PortError>;

    /// Save a loom definition.
    fn save(&self, loom: Loom) -> Result<(), PortError>;
}

/// Port for managing per-knot processing state.
///
/// Tracks the lifecycle of each knot as it processes strands: creation,
/// state transitions, and error recording.
pub trait KnotStatePort {
    /// Create initial state for a knot.
    fn create(&self, knot_id: &KnotId) -> Result<(), PortError>;

    /// Update the processing state of an existing knot.
    fn update(&self, state: KnotState) -> Result<(), PortError>;

    /// Get the current state for a knot.
    fn get(&self, knot_id: &KnotId) -> Result<Option<KnotState>, PortError>;
}

/// Port for appending and querying loom activity logs.
///
/// Records high-level loom events such as knot registration, loom
/// start/stop, and strand processing.
pub trait LoomLogPort {
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
pub trait EventSource {
    /// Start watching a directory for file events.
    fn watch(&self, path: &Path) -> Result<(), PortError>;

    /// Stop watching a directory.
    fn unwatch(&self, path: &Path) -> Result<(), PortError>;
}

/// Port for executing the agent CLI and capturing its output.
pub trait AgentRunner {
    /// Execute the agent CLI with the given context.
    fn execute(&self, ctx: ExecutionContext) -> Result<AgentOutput, PortError>;
}

/// Port for writing tie-off content to disk.
pub trait TieOffSink {
    /// Write tie-off output to its target location.
    fn write(&self, tie_off: TieOff) -> Result<(), PortError>;
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
        fn scan(&self, _workspace: &Path) -> Result<Vec<Loom>, PortError> {
            Ok(self.looms.values().cloned().collect())
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

    /// In-memory mock of `KnotStatePort`.
    #[derive(Default)]
    struct MockKnotStatePort {
        _states: HashMap<KnotId, KnotState>,
    }

    impl KnotStatePort for MockKnotStatePort {
        fn create(&self, _knot_id: &KnotId) -> Result<(), PortError> {
            Ok(())
        }

        fn update(&self, _state: KnotState) -> Result<(), PortError> {
            Ok(())
        }

        fn get(&self, _knot_id: &KnotId) -> Result<Option<KnotState>, PortError> {
            Ok(None)
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
    struct MockTieOffSink;

    impl TieOffSink for MockTieOffSink {
        fn write(&self, _tie_off: TieOff) -> Result<(), PortError> {
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
        let workspace = Path::new("/tmp/workspace");
        let scan_result = repo.scan(workspace);
        assert!(scan_result.is_ok());

        let loom_id = LoomId("test".to_string());
        let get_result = repo.get(&loom_id);
        assert!(get_result.is_ok());
        assert!(get_result.unwrap().is_none());

        let list_result = repo.list();
        assert!(list_result.is_ok());
        assert!(list_result.unwrap().is_empty());

        let loom = Loom {
            id: LoomId("save-test".to_string()),
            source_dir: PathBuf::from("src"),
            tie_off_dir: PathBuf::from("out"),
            knots: vec![],
        };
        let save_result = repo.save(loom);
        assert!(save_result.is_ok());
    }

    #[test]
    fn knot_state_port_contract() {
        let port = MockKnotStatePort::default();

        // Verify trait is object-safe
        let _obj: &dyn KnotStatePort = &port;

        // Verify all trait methods compile and are callable
        let knot_id = KnotId("k1".to_string());

        let create_result = port.create(&knot_id);
        assert!(create_result.is_ok());

        let state = KnotState {
            knot_id: knot_id.clone(),
            event_type: KnotEventType::Created,
            strand_path: StrandPath(PathBuf::from("input.md")),
            tie_off_path: Some(TieOffPath(PathBuf::from("output.md"))),
            status: ProcessingStatus::Idle,
            error: None,
            last_updated: "2026-01-01T00:00:00Z".to_string(),
        };
        let update_result = port.update(state);
        assert!(update_result.is_ok());

        let get_result = port.get(&knot_id);
        assert!(get_result.is_ok());
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
        };
        let append_result = port.append(event);
        assert!(append_result.is_ok());

        let read_result = port.read_all(&loom_id);
        assert!(read_result.is_ok());
    }

    #[test]
    fn agent_runner_contract() {
        let runner = MockAgentRunner::default();

        // Verify trait is object-safe
        let _obj: &dyn AgentRunner = &runner;

        // Verify ExecutionContext and AgentOutput types exist and work
        let ctx = ExecutionContext {
            cli_path: "/usr/bin/pi".to_string(),
            cli_args: vec!["--verbose".to_string()],
            prompt: "Review this document".to_string(),
            strand_path: StrandPath(PathBuf::from("doc.md")),
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
        };
        let result = sink.write(tie_off);
        assert!(result.is_ok());
    }

    #[test]
    fn event_source_contract() {
        let source = MockEventSource::default();

        // Verify trait is object-safe
        let _obj: &dyn EventSource = &source;

        // Verify watch/unwatch methods compile and are callable
        let path = Path::new("/tmp/watched");
        assert!(source.watch(path).is_ok());
        assert!(source.unwatch(path).is_ok());
    }

    // ── Supporting Type Tests ───────────────────────────────────────────

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
            strand_path: StrandPath(PathBuf::from("src/main.rs")),
        };

        assert_eq!(ctx.cli_path, "pi");
        assert_eq!(ctx.cli_args.len(), 2);
        assert!(!ctx.prompt.is_empty());
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

        let err = PortError::WorkspaceScanFailed("permission denied".to_string());
        assert_eq!(err.to_string(), "workspace scan failed: permission denied");

        let err = PortError::KnotStateCreateFailed("db error".to_string());
        assert_eq!(err.to_string(), "knot state create failed: db error");

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
