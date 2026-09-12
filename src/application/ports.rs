//! Application-layer port traits.
//!
//! Ports define the contracts that infrastructure adapters must satisfy.
//! The application layer orchestrates domain entities through these ports.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::entities::{
    Knot, KnotId, Loom, LoomId, RigState, StrandPath, TieOff, TieOffPath,
};
use crate::domain::events::{LoomEvent, RigLogEvent};
use crate::domain::value_objects::{AgentConfig, AgentProfile, ModelRegistry};

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
    /// Failed to derive knot status from the loom's run activity.
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
    AgentExecutionFailed {
        message: String,
        session_id: Option<String>,
    },
    /// The agent CLI binary was not found.
    CommandNotFound(String),
    /// Agent execution exceeded the configured timeout.
    Timeout {
        message: String,
        session_id: Option<String>,
    },
    /// The agent session ended without producing a final response.
    ///
    /// Distinct from `Timeout`: no deadline was exceeded — the session
    /// simply stopped (e.g. provider returned immediately, turn ended with
    /// only intermediate tool-use messages). Resumable when a session ID
    /// was captured (plan 078 re-enters the session to request the final
    /// response).
    AgentNoResponse {
        message: String,
        session_id: Option<String>,
    },
    /// The session's context exceeds the model window and pi's own
    /// compact-and-retry could not recover it — the kept context
    /// itself cannot fit. Terminal: session-resume re-entry cannot
    /// help, so this is NOT resumable.
    ///
    /// Plan 089 refined this: an *interrupted* auto-compact (an `overflow`
    /// `compaction_start` with no following `compaction_end` — the pi
    /// process died mid-compaction) is NOT terminal; it is surfaced as
    /// [`Self::CompactionInterrupted`] instead, because an out-of-band
    /// manual compact can still shrink the context. This variant is
    /// reserved for the completed-but-could-not-fit case (a `compaction_end`
    /// exists) and the compaction-never-ran case (plan 080 fail-fast).
    ContextLimitReached {
        message: String,
        session_id: Option<String>,
    },
    /// Plan 089: pi's in-process overflow recovery was interrupted — an
    /// `overflow` `compaction_start` was observed but the process exited
    /// before a `compaction_end`. Distinct from
    /// [`Self::ContextLimitReached`] (terminal): here the compaction never
    /// completed, so the context is not yet known to be over-full and an
    /// out-of-band manual compact on the same session can still save it.
    /// Resumable **only** when a `session_id` was captured (the manual
    /// compact needs a session to open via `--session-id`).
    CompactionInterrupted {
        /// Human-readable description (Display / activity-log line text).
        message: String,
        /// pi's compaction reason (the interrupted start's reason —
        /// always `"overflow"` for the resumable case).
        reason: String,
        /// The captured session id, when present.
        session_id: Option<String>,
    },
    /// Plan 089: the out-of-band manual `compact` on an interrupted
    /// session could not reduce the context below the window. Terminal:
    /// the context is over-full even after an explicit compact, so a
    /// re-entry would overflow again — NOT resumable.
    ManualCompactionFailed {
        /// Human-readable description (Display / activity-log line text).
        message: String,
        /// The session the manual compact was attempted on.
        session_id: Option<String>,
    },
    /// The agent session produced no output for the inactivity window —
    /// killed by the watchdog, most likely a blocked tool call or
    /// stalled provider (plan 081). Resumable; unlike `Timeout` it is
    /// resumable **without** a session ID (a fresh restart with the
    /// blocking-call note is meaningful — knots are idempotent).
    AgentInactivity {
        /// Human-readable description (Display / activity-log line text).
        message: String,
        /// How long the session was silent (seconds).
        silent_secs: u64,
        /// The configured inactivity window (seconds).
        window_secs: u64,
        /// The blocked call, when derivable from the stream
        /// (e.g. `bash("npm run build")`).
        blocked_call: Option<String>,
        session_id: Option<String>,
    },
    /// Plan 086: the pi-json runner's water-mark stop-resume — the
    /// session's context usage crossed `ctx-wrap-up-limit` and the
    /// process was SIGINT'd. Resumable: the session-resume retry
    /// re-invokes with `--session-id` + the `HANDOFF_NOTE`.
    WaterMarkStop {
        /// The captured session ID (from the JSON stream).
        session_id: Option<String>,
    },
    /// Failed to write tie-off output.
    TieOffWriteFailed(String),
    /// An agent profile was not found.
    ProfileNotFound(String),
    /// A profile's `model-ref` alias was not found in the model registry.
    ModelRefNotFound(String),
    /// Failed to scan the profiles directory.
    ProfileScanFailed(String),
    /// Failed to record a rig-level event (run activity).
    RigLogWriteFailed(String),
    /// Failed to read rig-level events (run activity).
    RigLogReadFailed(String),
    /// Failed to create a git commit.
    GitCommitFailed(String),
    /// Failed to write the state file.
    StateWriteFailed(String),
    /// Failed to check strand file validity.
    StrandCheckFailed(String),
    /// Failed to dispatch an agent event to a consumer.
    EventDispatchFailed(String),
    /// Failed to read/write a persisted event file.
    EventStoreFailed(String),
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
            PortError::AgentExecutionFailed { message, .. } => {
                write!(f, "agent execution failed: {message}")
            }
            PortError::CommandNotFound(msg) => {
                write!(f, "command not found: {msg}")
            }
            PortError::Timeout { message, .. } => {
                write!(f, "timeout: {message}")
            }
            PortError::AgentNoResponse { message, .. } => {
                write!(f, "no final response: {message}")
            }
            PortError::ContextLimitReached { message, .. } => {
                write!(f, "context limit reached: {message}")
            }
            PortError::CompactionInterrupted { message, .. } => {
                write!(f, "compaction interrupted: {message}")
            }
            PortError::ManualCompactionFailed { message, .. } => {
                write!(f, "manual compaction failed: {message}")
            }
            PortError::AgentInactivity { message, .. } => {
                write!(f, "inactivity: {message}")
            }
            PortError::WaterMarkStop { .. } => {
                write!(f, "water-mark stop (context usage crossed limit)")
            }
            PortError::TieOffWriteFailed(msg) => {
                write!(f, "tie-off write failed: {msg}")
            }
            PortError::ProfileNotFound(name) => {
                write!(f, "agent profile '{name}' not found")
            }
            PortError::ModelRefNotFound(alias) => {
                write!(f, "model-ref '{alias}' not found in rig/models.yml")
            }
            PortError::ProfileScanFailed(msg) => {
                write!(f, "profile scan failed: {msg}")
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
            PortError::StrandCheckFailed(msg) => {
                write!(f, "strand check failed: {msg}")
            }
            PortError::EventDispatchFailed(msg) => {
                write!(f, "event dispatch failed: {msg}")
            }
            PortError::EventStoreFailed(msg) => {
                write!(f, "event store failed: {msg}")
            }
        }
    }
}

impl std::error::Error for PortError {}

impl PortError {
    /// Extract session_id from errors that carry one.
    pub fn session_id(&self) -> Option<&String> {
        match self {
            PortError::Timeout { session_id, .. }
            | PortError::AgentExecutionFailed { session_id, .. }
            | PortError::AgentNoResponse { session_id, .. }
            | PortError::ContextLimitReached { session_id, .. }
            | PortError::CompactionInterrupted { session_id, .. }
            | PortError::ManualCompactionFailed { session_id, .. }
            | PortError::AgentInactivity { session_id, .. }
            | PortError::WaterMarkStop { session_id, .. } => {
                session_id.as_ref()
            }
            _ => None,
        }
    }

    /// Classify error as resumable (session can be retried) or fatal.
    ///
    /// `ContextLimitReached` and `ManualCompactionFailed` are deliberately
    /// excluded: the context does not fit the model window (even after pi's
    /// own compact-and-retry, or after an out-of-band manual compact), so
    /// session-resume re-entry cannot help (plans 079 / 089).
    ///
    /// `CompactionInterrupted` (plan 089) is resumable **only** when a
    /// session id was captured — the recovery (a manual compact + re-entry)
    /// needs a session to open via `--session-id`.
    pub fn is_resumable(&self) -> bool {
        if let PortError::CompactionInterrupted { session_id, .. } = self {
            return session_id.is_some();
        }
        matches!(
            self,
            PortError::Timeout { .. }
                | PortError::AgentExecutionFailed { .. }
                | PortError::AgentNoResponse { .. }
                | PortError::AgentInactivity { .. }
                | PortError::WaterMarkStop { .. }
        )
    }
}

/// Determine if a failed invocation should trigger a session-resume retry.
///
/// Returns `true` only when both conditions are met:
/// 1. A `session_id` was captured from the agent invocation.
/// 2. The error is resumable (`Timeout`, `AgentExecutionFailed`,
///    `AgentNoResponse`, or — plan 089 — `CompactionInterrupted` with a
///    session id).
///
/// If either condition is not met (no session ID, or a fatal error like
/// `CommandNotFound` / `ContextLimitReached` / `ManualCompactionFailed`),
/// the invocation is not retryable via session resume.
pub fn is_session_resumable(
    session_id: &Option<String>,
    error: &PortError,
) -> bool {
    session_id.is_some() && error.is_resumable()
}

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
///
/// The adapter builds CLI arguments from `agent_config` internally,
/// so the application layer does not construct `cli_args`.
///
/// The optional `timeout` field allows per-knot timeout overrides.
/// When `None`, the runner's global default timeout is used.
/// When `Some(d)`, the agent session deadline is `d`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    /// Agent configuration (provider, model, tools, extra args).
    ///
    /// The adapter builds CLI arguments from this internally.
    /// `extra_args` may contain `--session-id` from a retry attempt.
    pub agent_config: AgentConfig,
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

/// Token usage reported by the agent LLM provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input tokens (system prompt + user messages).
    pub input: u64,
    /// Output tokens (agent response).
    pub output: u64,
    /// Tokens read from cache.
    pub cache_read: u64,
    /// Tokens written to cache.
    pub cache_write: u64,
    /// Total tokens consumed.
    pub total: u64,
}

/// One `compaction_end` event observed in the agent's JSON stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionRecord {
    /// pi's compaction reason: `"overflow"` (context limit hit),
    /// `"threshold"` (proactive), or `"manual"`.
    pub reason: String,
    /// Context tokens before compaction
    /// (`compaction_end.result.tokensBefore`); `None` when the
    /// compaction failed (no result).
    pub tokens_before: Option<u64>,
    /// True when pi auto-retries the prompt after compaction.
    pub will_retry: bool,
    /// pi's error message when compaction failed (`errorMessage`).
    pub error: Option<String>,
    /// pi's `aborted` flag on `compaction_end` (plan 088): the
    /// compaction was started but aborted (e.g. user interrupt /
    /// inactivity) rather than succeeding or failing with an error
    /// message. Serde-defaulted so pre-088 records deserialize.
    #[serde(default)]
    pub aborted: bool,
}

/// One compaction span boundary observed **live** in the agent's JSON
/// stream (plan 088) — emitted as the stream produces it, not after the
/// invocation.
///
/// Multiple spans per session are the normal case: each
/// `compaction_start`/`compaction_end` pair fires its own observation in
/// stream order (threshold compactions repeat as the context refills,
/// and overflow compact-and-retry fires per user message). There is no
/// pairing assumption between starts and ends; the `session_id` is
/// `None` when it has not yet been captured from the stream (JSON mode:
/// the `session` header line; RPC mode: the `get_state` response).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionObservation {
    /// A `compaction_start` event — the span begins.
    Started {
        /// Session id captured so far from the stream, if any.
        session_id: Option<String>,
        /// pi's compaction reason (`"threshold"` / `"overflow"` /
        /// `"manual"`).
        reason: String,
    },
    /// A `compaction_end` event — the span ends. Success, failure, or
    /// abort is discriminated by the record's `error` / `aborted`
    /// fields.
    Ended {
        /// Session id captured so far from the stream, if any.
        session_id: Option<String>,
        /// The parsed `compaction_end` record.
        record: CompactionRecord,
    },
    /// Plan 089 (D6): Knot asked the **same** session to continue in place
    /// after a compaction ended its turn with no final answer — one
    /// `prompt` command on the still-open `pi-rpc` channel, no new process.
    /// Carried on this enum because it is emitted from the same place the
    /// compaction spans are observed (the runner's driver thread).
    Continued {
        /// Session id captured so far from the stream, if any.
        session_id: Option<String>,
        /// The reason of the compaction whose turn ended without an answer
        /// (`"threshold"` in the normal case).
        reason: String,
    },
}

/// One context wrap-up steer observed in an invocation (plan 084).
///
/// Recorded by the `pi-rpc` runner when the session context crosses the
/// alias's `ctx-wrap-up-limit` and Knot steers the agent to wrap up
/// gracefully (commit, update progress, note the incomplete, final
/// tie-off). At most one per invocation (fire-once).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrapUpRecord {
    /// `contextUsage.tokens` at the moment the steer was queued.
    pub context_tokens: u64,
    /// The configured `ctx-wrap-up-limit`.
    pub limit: u64,
    /// How the handoff note was delivered (plan 086): `"steer"`
    /// (the `pi-rpc` live-steer path) or `"stop-resume"` (the
    /// `pi-json` SIGINT + re-invoke path).
    pub mechanism: String,
}

/// Metadata captured from an agent invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInvocationMetadata {
    /// Session ID from the agent CLI (for session resume).
    pub session_id: Option<String>,
    /// Token usage from the LLM provider.
    pub token_usage: Option<TokenUsage>,
    /// Compactions observed in the agent's JSON stream (plan 079) —
    /// one record per `compaction_end`, in stream order. Empty when
    /// the adapter does not report them (e.g. the stdio adapter).
    #[serde(default)]
    pub compactions: Vec<CompactionRecord>,
    /// Compaction start reasons observed in the agent's JSON stream
    /// (plan 088) — one per `compaction_start`, in stream order.
    /// Still captured by the shared post-hoc parse (useful for tests
    /// and debugging); the live source of the compaction events is the
    /// observer ([`AgentRunner::execute_with_config_and_observer`]).
    /// Serde-defaulted so pre-088 metadata deserializes.
    #[serde(default)]
    pub compaction_starts: Vec<String>,
    /// The context wrap-up steer observed in this invocation (plan 084).
    ///
    /// `None` when the `pi-rpc` runner did not steer (no limit set, the
    /// limit was never crossed, or a non-rpc adapter — the others cannot
    /// steer mid-run). At most one record per invocation (fire-once).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_up: Option<WrapUpRecord>,
}

/// Output captured from agent execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentOutput {
    /// Standard output from the agent.
    pub stdout: String,
    /// Standard error from the agent.
    pub stderr: String,
    /// Exit code from the agent process.
    pub exit_code: i32,
    /// Invocation metadata (session ID, token usage).
    ///
    /// `None` when using the stdio adapter or when metadata could
    /// not be extracted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AgentInvocationMetadata>,
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

    /// Append an event to the loom's current-run activity.
    ///
    /// Events are kept in memory for the life of the run (plan 083)
    /// and logged as `[KNOT][EVENT]` lines; nothing is persisted.
    fn append(&self, event: LoomEvent) -> Result<(), PortError>;

    /// Read all events for a loom (current run, in memory).
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

    /// Stop watching a specific watch type for a directory.
    ///
    /// Removes only the entry matching `(path, watch_type)`, leaving
    /// other watch types for the same path intact. This is the
    /// targeted version of `unwatch()` — use it when multiple knots
    /// may share the same directory.
    ///
    /// Default implementation: delegates to `unwatch(path)` for
    /// backward compatibility with mock implementations and existing
    /// adapters that do not need targeted unwatching.
    fn unwatch_with_type(
        &self,
        path: &Path,
        _watch_type: crate::adapters::outbound::event_source::WatchType,
    ) -> Result<(), PortError> {
        self.unwatch(path)
    }

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

    /// Execute the agent using an `AgentConfig` to build CLI arguments.
    ///
    /// Each adapter owns its own binary path and builds CLI args from
    /// `agent_config` internally, so the caller (ProcessStrand) does not
    /// construct `cli_args`.
    ///
    /// `strand_file_ref` is `Some(path)` for Created/Modified events
    /// (appended as `@{path}`) and `None` for Deleted events (file gone).
    ///
    /// Default implementation delegates to `execute()` with a synthetic
    /// context — mock runners override `execute()` only.
    fn execute_with_config(
        &self,
        agent_config: &AgentConfig,
        strand_path: StrandPath,
        strand_file_ref: Option<StrandPath>,
        prompt: String,
        profile_prompt: String,
        event_type: String,
        knot_name: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<AgentOutput, PortError> {
        let ctx = ExecutionContext {
            agent_config: agent_config.clone(),
            prompt,
            profile_prompt,
            strand_path,
            event_type,
            knot_name,
            timeout,
        };
        let _ = strand_file_ref; // unused by default impl (adapter handles @file)
        self.execute(ctx)
    }

    /// Return a human-readable name for this runner implementation.
    /// Used by composition tests to verify the correct adapter is wired.
    fn runner_type(&self) -> &str {
        "unknown"
    }

    /// Execute the agent with a **live compaction observer** (plan 088).
    ///
    /// The observer is invoked, in stream order, for each
    /// `compaction_start` and `compaction_end` event **as the runner's
    /// stream reader observes it** — live, not after the invocation:
    /// a long run that compacts several times reports each span the
    /// moment it happens. The observer runs on the runner's reader /
    /// driver thread, so it must be cheap, non-blocking-with-respect-to
    /// the main thread, and `Send + Sync` (the pi-json loom-log append
    /// + system-event emission satisfies both — best-effort, no locks
    /// held across the append).
    ///
    /// `None` → no observation (the common case: mock runners and
    /// adapters without a JSON stream). Default implementation ignores
    /// the observer and delegates to [`Self::execute_with_config`] —
    /// mock runners override this to fire the observer from their mock
    /// output's compaction records, keeping usecase-level test parity.
    fn execute_with_config_and_observer(
        &self,
        agent_config: &AgentConfig,
        strand_path: StrandPath,
        strand_file_ref: Option<StrandPath>,
        prompt: String,
        profile_prompt: String,
        event_type: String,
        knot_name: Option<String>,
        timeout: Option<Duration>,
        observer: Option<Arc<dyn Fn(&CompactionObservation) + Send + Sync>>,
    ) -> Result<AgentOutput, PortError> {
        let _ = observer; // ignored by the default implementation
        self.execute_with_config(
            agent_config,
            strand_path,
            strand_file_ref,
            prompt,
            profile_prompt,
            event_type,
            knot_name,
            timeout,
        )
    }

    /// Manually compact an existing session out-of-band (plan 089).
    ///
    /// Drives pi's `compact` RPC command on the session identified by
    /// `session_id` (opened via `--session-id`), reducing its context below
    /// the model window. Returns the `CompactionRecord` on success, or
    /// [`PortError::ManualCompactionFailed`] on failure (an error message,
    /// a timeout, or an abort).
    ///
    /// This is the remedy for an interrupted auto-compact
    /// ([`PortError::CompactionInterrupted`]): it runs a clean, top-level
    /// compaction that always emits `compaction_end { reason: "manual" }`,
    /// and its summarisation call sends only the *older* portion of the
    /// context (well under the window) — so it fits where the in-flight
    /// retry's context did not.
    ///
    /// `custom_instructions` is a fixed, operator-tunable instruction for
    /// the summarisation (e.g. keep task state and open work items).
    ///
    /// Default: not supported. Only the `pi-rpc` adapter can drive the
    /// `compact` RPC over a persistent channel; the one-shot JSON/stdio
    /// runners return [`PortError::ManualCompactionFailed`].
    fn manual_compact(
        &self,
        _ctx: &ExecutionContext,
        session_id: &str,
        _custom_instructions: &str,
    ) -> Result<CompactionRecord, PortError> {
        Err(PortError::ManualCompactionFailed {
            message: format!(
                "manual compact on session '{session_id}' requires the pi-rpc adapter"
            ),
            session_id: Some(session_id.to_string()),
        })
    }
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

/// Port for appending and querying rig-level operational events.
///
/// Events are in-memory run activity (plan 083 — the retired
/// `.rig-log` JSONL file is gone): each append is rendered as a
/// single-line `[KNOT][EVENT]` record on stderr (the service log).
pub trait RigLogPort: Send + Sync {
    /// Append a rig-level event (current run, in memory — plan 083).
    fn append(&self, event: RigLogEvent) -> Result<(), PortError>;

    /// Read all rig-level events (current run, in memory).
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

}

/// Port for loading the rig-level model registry (`rig/models.yml`).
///
/// The registry maps model aliases to concrete `{provider, model}`
/// pairs. Implementations must read the registry **fresh on every
/// `load()` call** — no caching — so that editing `rig/models.yml`
/// swaps the model behind an alias on the next strand processed, without
/// a restart (the profile lifetime, not the startup lifetime).
pub trait ModelRegistryPort: Send + Sync {
    /// Load the model registry.
    ///
    /// A missing registry file yields an empty registry. Malformed
    /// content degrades to an empty registry (with a warning) so the
    /// registry can never block strand processing.
    fn load(&self) -> Result<ModelRegistry, PortError>;
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

    /// Ensure the rig directory is its own git repository and — when
    /// the project root (parent of `rig_dir`) is inside a git repo —
    /// excluded from that repo.
    ///
    /// Idempotent and non-fatal: runs `git init` in `rig_dir` when
    /// `rig/.git` is absent (no `.gitignore` is written into the rig),
    /// and appends a marked `<rig-basename>/` entry to the parent
    /// `.gitignore` — unless the rig is already tracked by the parent,
    /// in which case the manual `git rm -r --cached` command is logged
    /// instead of editing the file. All failure modes (no git binary,
    /// git init failure, no parent repo, already initialised) degrade
    /// to a warning and `Ok(())`.
    fn ensure_rig_repo(&self, rig_dir: &std::path::Path) -> Result<(), PortError>;
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

/// Port for the strand event queue.
///
/// Defines the contract for pushing, popping, snapshotting, and managing
/// strand events. The primary production implementation is disk-backed
/// (`DiskBackedEventQueue`); an in-memory implementation exists for
/// testing and as a Phase-1 compat shim.
///
/// Every operation is safe to call from any thread (`Send + Sync`).
///
/// `notified()` returns a `Pin<Box<dyn Future>>` to keep the trait
/// dyn-compatible (async trait methods are not dyn-compatible in stable
/// Rust without the `async-trait` crate).
pub trait StrandEventQueue: Send + Sync {
    /// Push an event. Returns the assigned ID.
    fn push(&self, event: crate::domain::pending_event::PendingEvent) -> crate::domain::pending_event::PendingEventId;

    /// Push an event, or replace an existing one with the same dedup key.
    /// Returns the ID of the (new or replaced) entry.
    fn push_or_replace(&self, event: crate::domain::pending_event::PendingEvent) -> crate::domain::pending_event::PendingEventId;

    /// Pop the next event from the queue (FIFO).
    ///
    /// Returns `Some(PendingEventOrShutdown::Event)` for real events,
    /// `Some(PendingEventOrShutdown::Shutdown)` for the shutdown sentinel,
    /// or `None` if the queue is empty (no sentinel, no events).
    ///
    /// Delete-on-read: the event is removed from the queue. The service
    /// loop no longer uses this (it peeks via [`front`](Self::front) and
    /// removes explicitly after processing); `pop` remains a valid
    /// primitive for tests and one-shot consumers.
    fn pop(&self) -> Option<crate::domain::pending_event::PendingEventOrShutdown>;

    /// Read the head event (FIFO order) **without** removing it.
    ///
    /// Returns the on-disk content of the front event, or `None` when the
    /// queue is empty. The shutdown sentinel is never returned — callers
    /// check [`shutdown_signaled`](Self::shutdown_signaled) separately.
    ///
    /// This is the peek primitive for the late-removal (at-least-once)
    /// loop: the event file survives while processing is in flight, so a
    /// crash mid-processing re-queues the event instead of losing it.
    fn front(&self) -> Option<crate::domain::pending_event::PendingEvent>;

    /// Whether a shutdown has been signalled via `push_shutdown`.
    ///
    /// Replaces the `pop()`-sentinel shutdown detection for loops that
    /// use `front()` (which never returns the sentinel).
    fn shutdown_signaled(&self) -> bool;

    /// Take a snapshot of all pending events (excludes shutdown sentinel).
    fn snapshot(&self) -> Vec<crate::domain::pending_event::PendingEvent>;

    /// Delete a specific pending event by ID.
    /// Returns `true` if the event was found and removed, `false` otherwise.
    fn delete(&self, id: &crate::domain::pending_event::PendingEventId) -> bool;

    /// Get a single pending event by ID.
    fn pending_event(&self, id: &crate::domain::pending_event::PendingEventId) -> Option<crate::domain::pending_event::PendingEvent>;

    /// Return the number of pending events (excludes shutdown sentinel).
    fn len(&self) -> usize;

    /// Check if the queue is empty (no events, no sentinel).
    fn is_empty(&self) -> bool;

    /// Push a shutdown sentinel (called by debounce engine on channel close).
    fn push_shutdown(&self);

    /// Await a signal that an item was pushed.
    ///
    /// Returns a boxed future to keep the trait dyn-compatible.
    ///
    /// **Armed-at-call contract:** the returned future registers its
    /// `Notify` permit at creation. A signal sent after the call is
    /// guaranteed to wake an await of the returned future, even if the
    /// await has not started. Callers should create the future **before**
    /// re-checking [`front()`](Self::front) and may drop it if the check
    /// already returned an event — dropping an unconsumed armed permit is
    /// harmless and never desynchronises the queue.
    fn notified(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>;
}

/// Port for dispatching agent events to consumer knots.
///
/// An event is dispatched by creating a file in the consumer's loom
/// tie-off directory under an `{event-id}/` subdirectory.
pub trait EventDispatcherPort: Send + Sync {
    /// Dispatch a single agent event to a consumer knot.
    ///
    /// Creates an event file at:
    /// `rig/tie-offs/{consumer-loom-id}/{event-id}/event-{timestamp}.md`
    /// Content: YAML frontmatter with event payload + markdown body with context.
    /// If multiple consumers listen for the same event, each loom gets its own copy.
    ///
    /// `seq` is the dispatch's position within its target directory batch
    /// (the `{consumer-loom-id}/{event-id}/` directory). `0` = plain name
    /// `event-{ts}.md` (single-dispatch group); `i ≥ 1` = suffixed name
    /// `event-{ts}-{i:03}.md` (the i-th dispatch of a same-second fan-out
    /// into the same directory). The adapter owns name materialisation and
    /// guarantees the created file is unique (atomic creation, taken-name
    /// fallback).
    ///
    /// `producer_knot` identifies the knot that emitted the event — derived
    /// from the tie-off processing context, not from the event itself.
    ///
    /// Returns the path of the created event file.
    fn dispatch(
        &self,
        event: &crate::domain::events::AgentEvent,
        consumer_knot: &Knot,
        producer_knot: &str,
        consumer_loom_id: &LoomId,
        rig_dir: &Path,
        seq: u32,
    ) -> Result<std::path::PathBuf, PortError>;
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::outbound::event_source::WatchType;
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
        events: std::sync::Mutex<Vec<LoomEvent>>,
    }

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(&self, _loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
            Ok(self.events.lock().unwrap().clone())
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
                metadata: None,
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

    }

    /// In-memory mock of `ModelRegistryPort`.
    ///
    /// Returns the configured registry on every `load()` call.
    struct MockModelRegistry {
        registry: ModelRegistry,
    }

    impl ModelRegistryPort for MockModelRegistry {
        fn load(&self) -> Result<ModelRegistry, PortError> {
            Ok(self.registry.clone())
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

        fn ensure_rig_repo(&self, _rig_dir: &std::path::Path) -> Result<(), PortError> {
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
            agent_config: AgentConfig {
                goal: "review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
                ctx_wrap_up_limit: None,
            },
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
            agent_events: Vec::new(),
            event_metadata: crate::domain::entities::EventMetadata::default(),
            session_id: None,
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

        // Verify unwatch_with_type compiles and is callable (default impl)
        assert!(
            source.unwatch_with_type(
                path,
                WatchType::Strand(
                    crate::domain::entities::LoomId("test".into()),
                    crate::domain::entities::KnotId("test".into()),
                ),
            )
            .is_ok()
        );
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

    }

    #[test]
    fn model_registry_port_contract() {
        let registry = ModelRegistry::from_yaml(
            "models:\n  fast:\n    provider: openai\n    model: gpt-4o\n",
        )
        .unwrap();
        let port = MockModelRegistry { registry: registry.clone() };

        // Verify trait is object-safe
        let _obj: &dyn ModelRegistryPort = &port;

        // Verify load() returns the configured registry
        let loaded = port.load().unwrap();
        assert_eq!(loaded, registry);
        assert_eq!(loaded.resolve("fast").unwrap().model, "gpt-4o");
    }

    #[test]
    fn port_error_model_ref_not_found_display() {
        let err = PortError::ModelRefNotFound("fast".to_string());
        assert_eq!(
            err.to_string(),
            "model-ref 'fast' not found in rig/models.yml"
        );
        let _: &dyn std::error::Error = &err;
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
            strand_queue: vec![],
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
            agent_config: AgentConfig {
                goal: "process".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: vec![],
                extra_args: vec![],
                thinking_level: None,
                ctx_wrap_up_limit: None,
            },
            prompt: "Process this file".to_string(),
            profile_prompt: "You are an agent.".to_string(),
            strand_path: StrandPath(PathBuf::from("src/main.rs")),
            event_type: "Created".to_string(),
            knot_name: Some("review".to_string()),
            timeout: None,
        };

        assert_eq!(ctx.agent_config.model, "gpt-4o");
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
            metadata: None,
        };

        assert_eq!(output.exit_code, 0);
        assert!(!output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
        assert!(output.metadata.is_none());
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

        let err = PortError::AgentExecutionFailed {
            message: "crash".to_string(),
            session_id: None,
        };
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

    /// Plan 077: `AgentNoResponse` is distinct from `Timeout` — it carries a
    /// session ID (a live session can be re-entered by plan 078), it is
    /// resumable, and it displays without the "timeout:" prefix.
    #[test]
    fn agent_no_response_is_resumable() {
        let err = PortError::AgentNoResponse {
            message: "agent returned empty response".to_string(),
            session_id: Some("sess-abc".to_string()),
        };

        assert!(
            err.is_resumable(),
            "AgentNoResponse should be resumable (session can be re-entered)"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-abc"),
            "session_id() should return the captured session ID"
        );
        assert!(
            err.to_string().starts_with("no final response:"),
            "Display should start with 'no final response:', got: {}",
            err
        );
        assert!(!err.to_string().starts_with("timeout:"));
    }

    /// Plan 077: without a session ID, the error is not resumable via
    /// session-resume (`is_session_resumable` gates on both conditions).
    #[test]
    fn agent_no_response_without_session() {
        let err = PortError::AgentNoResponse {
            message: "agent returned empty response".to_string(),
            session_id: None,
        };

        assert!(err.session_id().is_none());
        assert!(
            !is_session_resumable(&None, &err),
            "is_session_resumable must be false without a session ID"
        );
    }

    /// Plan 079: a terminal context overflow is NOT resumable —
    /// session-resume re-entry cannot fit a context that already does
    /// not fit. It carries a session ID and displays with the
    /// "context limit reached:" prefix.
    #[test]
    fn context_limit_reached_not_resumable() {
        let err = PortError::ContextLimitReached {
            message: "session context cannot fit the model window even after compaction".to_string(),
            session_id: Some("sess-ctx".to_string()),
        };

        assert!(
            !err.is_resumable(),
            "ContextLimitReached must not be resumable — re-entry cannot help"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-ctx"),
            "session_id() should return the captured session ID"
        );
        assert!(
            err.to_string().contains("context limit reached"),
            "Display should contain 'context limit reached', got: {}",
            err
        );
        assert!(
            !is_session_resumable(&Some("sess-ctx".to_string()), &err),
            "is_session_resumable must be false for ContextLimitReached"
        );
    }

    /// Plan 089: an interrupted auto-compact is resumable **when a session id
    /// was captured** (the manual-compact recovery needs a session to open
    /// via `--session-id`). It carries the session ID and displays with the
    /// `compaction interrupted:` prefix.
    #[test]
    fn compaction_interrupted_is_resumable_with_session() {
        let err = PortError::CompactionInterrupted {
            message: "pi's in-process overflow compaction was interrupted".to_string(),
            reason: "overflow".to_string(),
            session_id: Some("sess-int".to_string()),
        };

        assert!(
            err.is_resumable(),
            "CompactionInterrupted with a session id is resumable"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-int"),
            "session_id() should return the captured session ID"
        );
        assert!(
            is_session_resumable(&Some("sess-int".to_string()), &err),
            "is_session_resumable must be true for CompactionInterrupted with a session"
        );
        assert!(
            err.to_string().contains("compaction interrupted"),
            "Display should contain 'compaction interrupted', got: {}",
            err
        );
    }

    /// Plan 089: a `CompactionInterrupted` **without** a session id is not
    /// resumable — the manual-compact recovery has no session to open, so
    /// the strand fails immediately.
    #[test]
    fn compaction_interrupted_without_session_is_not_resumable() {
        let err = PortError::CompactionInterrupted {
            message: "pi's in-process overflow compaction was interrupted".to_string(),
            reason: "overflow".to_string(),
            session_id: None,
        };

        assert!(
            !err.is_resumable(),
            "CompactionInterrupted without a session id is not resumable"
        );
        assert!(err.session_id().is_none());
        assert!(
            !is_session_resumable(&None, &err),
            "is_session_resumable must be false without a session ID"
        );
    }

    /// Plan 089: a failed manual compact is terminal (NOT resumable) — the
    /// context is over-full even after an explicit compact, so a re-entry
    /// would overflow again. It carries the session ID and displays with the
    /// `manual compaction failed:` prefix.
    #[test]
    fn manual_compaction_failed_is_terminal() {
        let err = PortError::ManualCompactionFailed {
            message: "manual compact could not reduce the context".to_string(),
            session_id: Some("sess-manual".to_string()),
        };

        assert!(
            !err.is_resumable(),
            "ManualCompactionFailed must not be resumable — re-entry would overflow again"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-manual"),
            "session_id() should return the captured session ID"
        );
        assert!(
            !is_session_resumable(&Some("sess-manual".to_string()), &err),
            "is_session_resumable must be false for ManualCompactionFailed"
        );
        assert!(
            err.to_string().contains("manual compaction failed"),
            "Display should contain 'manual compaction failed', got: {}",
            err
        );
    }

    /// Plan 081: `AgentInactivity` is resumable, carries its session
    /// ID, and displays with the greppable `inactivity:` prefix
    /// (mirrors the `timeout:` / `no final response:` prefixes).
    #[test]
    fn agent_inactivity_is_resumable_with_session() {
        let err = PortError::AgentInactivity {
            message: "no output for 300s (inactivity window 300s) (mock)"
                .to_string(),
            silent_secs: 300,
            window_secs: 300,
            blocked_call: Some("bash(\"npm run build\")".to_string()),
            session_id: Some("sess-inact".to_string()),
        };

        assert!(
            err.is_resumable(),
            "AgentInactivity should be resumable (the session can be restarted)"
        );
        assert_eq!(
            err.session_id().map(String::as_str),
            Some("sess-inact"),
            "session_id() should return the captured session ID"
        );
        assert!(
            err.to_string().starts_with("inactivity:"),
            "Display should start with 'inactivity:', got: {}",
            err
        );
        assert!(
            err.to_string().contains("no output for 300s"),
            "Display should carry the message, got: {err}"
        );
        assert!(!err.to_string().starts_with("timeout:"));
    }

    /// Plan 081: without a session ID the error is still resumable —
    /// the one deliberate exception: a fresh restart with the
    /// blocking-call note is meaningful (knots are idempotent).
    #[test]
    fn agent_inactivity_is_resumable_without_session() {
        let err = PortError::AgentInactivity {
            message: "no output for 42s (inactivity window 300s) (mock)"
                .to_string(),
            silent_secs: 42,
            window_secs: 300,
            blocked_call: None,
            session_id: None,
        };

        assert!(
            err.is_resumable(),
            "AgentInactivity is resumable even without a session ID"
        );
        assert!(err.session_id().is_none());
        // Note: is_session_resumable still gates on the session ID —
        // the gate exception lives in the resume loop (plan 081,
        // phase 4), not in this helper.
        assert!(!is_session_resumable(&None, &err));
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

    // ── AgentInvocationMetadata / TokenUsage Tests ──────────────────

    #[test]
    fn test_agent_output_with_metadata() {
        let metadata = AgentInvocationMetadata {
            session_id: Some("sess-abc123".to_string()),
            token_usage: Some(TokenUsage {
                input: 100,
                output: 50,
                cache_read: 10,
                cache_write: 5,
                total: 165,
            }),
            compactions: vec![],
            compaction_starts: vec![],
            wrap_up: None,
        };
        let output = AgentOutput {
            stdout: "response".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: Some(metadata),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("session_id"));
        assert!(json.contains("sess-abc123"));
        assert!(json.contains("input"));

        let restored: AgentOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.stdout, output.stdout);
        assert_eq!(restored.metadata, output.metadata);
    }

    #[test]
    fn agent_invocation_metadata_wrap_up_roundtrip() {
        // Present: serialized and restored.
        let metadata = AgentInvocationMetadata {
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            compactions: vec![],
            compaction_starts: vec![],
            wrap_up: Some(WrapUpRecord {
                context_tokens: 150_000,
                limit: 140_000,
                mechanism: "steer".to_string(),
            }),
        };
        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains("wrap_up"));
        assert!(json.contains("150000"));
        let restored: AgentInvocationMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, metadata);

        // Absent: omitted from the serialized form; legacy metadata without
        // the key still deserializes to `None` (additive field).
        let metadata = AgentInvocationMetadata {
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            compactions: vec![],
            compaction_starts: vec![],
            wrap_up: None,
        };
        let json = serde_json::to_string(&metadata).unwrap();
        assert!(!json.contains("wrap_up"));
        let legacy: AgentInvocationMetadata = serde_json::from_str(
            r#"{"session_id":"sess-1","token_usage":null,"compactions":[]}"#,
        )
        .unwrap();
        assert!(legacy.wrap_up.is_none());
    }

    #[test]
    fn test_agent_output_without_metadata() {
        let output = AgentOutput {
            stdout: "response".to_string(),
            stderr: String::new(),
            exit_code: 0,
            metadata: None,
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(!json.contains("metadata"));

        // Round-trip: deserialise JSON without metadata field
        let restored: AgentOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.stdout, "response");
        assert!(restored.metadata.is_none());

        // Also verify deserialising JSON that explicitly omits metadata
        let json_no_meta = r#"{"stdout":"response","stderr":"","exit_code":0}"#;
        let restored2: AgentOutput = serde_json::from_str(json_no_meta).unwrap();
        assert!(restored2.metadata.is_none());
    }

    #[test]
    fn test_token_usage_fields() {
        let json = r#"{"input":200,"output":150,"cache_read":30,"cache_write":10,"total":390}"#;
        let usage: TokenUsage = serde_json::from_str(json).unwrap();

        assert_eq!(usage.input, 200);
        assert_eq!(usage.output, 150);
        assert_eq!(usage.cache_read, 30);
        assert_eq!(usage.cache_write, 10);
        assert_eq!(usage.total, 390);
    }

    // ── PortError session_id Tests ──────────────────────────────────

    #[test]
    fn test_port_error_session_id_timeout() {
        let err = PortError::Timeout {
            message: "timed out".to_string(),
            session_id: Some("sess-xyz".to_string()),
        };
        assert_eq!(
            err.session_id().map(|s| s.as_str()),
            Some("sess-xyz")
        );
    }

    #[test]
    fn test_port_error_session_id_execution_failed() {
        let err = PortError::AgentExecutionFailed {
            message: "crash".to_string(),
            session_id: Some("sess-abc".to_string()),
        };
        assert_eq!(
            err.session_id().map(|s| s.as_str()),
            Some("sess-abc")
        );
    }

    #[test]
    fn test_port_error_session_id_command_not_found() {
        let err = PortError::CommandNotFound("pi not found".to_string());
        assert!(err.session_id().is_none());
    }

    // ── PortError is_resumable Tests ────────────────────────────────

    #[test]
    fn test_port_error_is_resumable_timeout() {
        let err = PortError::Timeout {
            message: "timed out".to_string(),
            session_id: None,
        };
        assert!(err.is_resumable());
    }

    #[test]
    fn test_port_error_is_resumable_execution_failed() {
        let err = PortError::AgentExecutionFailed {
            message: "crash".to_string(),
            session_id: None,
        };
        assert!(err.is_resumable());
    }

    #[test]
    fn test_port_error_is_resumable_command_not_found() {
        let err = PortError::CommandNotFound("not found".to_string());
        assert!(!err.is_resumable());
    }

    #[test]
    fn test_port_error_display_timeout_with_session() {
        let err = PortError::Timeout {
            message: "exceeded 60s".to_string(),
            session_id: Some("sess-123".to_string()),
        };
        // Display shows the message, ignores session_id
        assert_eq!(err.to_string(), "timeout: exceeded 60s");
    }

    // ── is_session_resumable Tests ──────────────────────────────────

    #[test]
    fn is_session_resumable_with_session_and_timeout() {
        let session_id = Some("sess-abc".to_string());
        let err = PortError::Timeout {
            message: "timed out".to_string(),
            session_id: Some("sess-abc".to_string()),
        };
        assert!(is_session_resumable(&session_id, &err));
    }

    #[test]
    fn is_session_resumable_with_session_and_execution_failed() {
        let session_id = Some("sess-def".to_string());
        let err = PortError::AgentExecutionFailed {
            message: "crash".to_string(),
            session_id: Some("sess-def".to_string()),
        };
        assert!(is_session_resumable(&session_id, &err));
    }

    #[test]
    fn is_session_resumable_without_session() {
        let session_id = None;
        let err = PortError::Timeout {
            message: "timed out".to_string(),
            session_id: None,
        };
        assert!(!is_session_resumable(&session_id, &err));
    }

    #[test]
    fn is_session_resumable_command_not_found() {
        let session_id = Some("sess-ghi".to_string());
        let err = PortError::CommandNotFound("pi not found".to_string());
        assert!(!is_session_resumable(&session_id, &err));
    }

    // ── EventStoreFailed Tests ──────────────────────────────────

    #[test]
    fn port_error_event_store_failed_display() {
        let err = PortError::EventStoreFailed("disk full".to_string());
        assert_eq!(err.to_string(), "event store failed: disk full");
    }

    #[test]
    fn port_error_event_store_failed_is_std_error() {
        let err = PortError::EventStoreFailed("io error".to_string());
        let _: &dyn std::error::Error = &err;
    }
}
