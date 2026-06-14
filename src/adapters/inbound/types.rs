//! Shared types for the inbound HTTP adapter.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::application::ports::{
    AgentProfileRepository, AgentRunner, EventSource, LoomLogPort,
    LoomRepository, RigLogPort, TieOffSink,
};
use crate::application::store::LoomStore;
use crate::domain::entities::LoomId;
use crate::domain::events::StrandEvent;
use crate::domain::value_objects::RigAgentConfig;

// ── Response Bodies ───────────────────────────────────────────────────────

/// Response body for profile GET endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProfileResponse {
    /// Profile name (derived from filename).
    pub name: String,
    /// The LLM provider identifier.
    pub provider: String,
    /// The model name to use.
    pub model: String,
    /// Optional list of tool identifiers.
    #[schema(value_type = Vec<String>)]
    pub tools: Vec<String>,
    /// The system prompt.
    pub system_prompt: String,
    /// Optional markdown body from the profile file (documentation only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// Response for `GET /config/rig` — rig path info plus agent config.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct RigConfigResponse {
    /// Absolute path to the rig directory.
    #[schema(value_type = String)]
    pub rig_path: PathBuf,
    /// Path to the agent CLI binary.
    pub cli_path: String,
    /// Arguments passed to the agent CLI.
    pub cli_args: Vec<String>,
}

// ── AppContext ────────────────────────────────────────────────────────────

/// Application context passed to handlers via Axum `State`.
///
/// Holds port instances and the debounce engine sender.
/// Handlers clone ports from this context and delegate to use cases.
#[derive(Clone)]
pub struct AppContext {
    /// In-memory loom registry.
    pub store: LoomStore,
    /// Loom repository port.
    pub loom_repo: Arc<dyn LoomRepository>,
    /// Loom log port.
    pub loom_log_port: Arc<dyn LoomLogPort>,
    /// Tie-off sink port.
    pub tie_off_sink: Arc<dyn TieOffSink>,
    /// File-system event source — used to watch/unwatch source dirs.
    pub event_source: Arc<dyn EventSource>,
    /// Debounce engine sender — feed raw strand events.
    pub event_sender: mpsc::Sender<StrandEvent>,
    /// Agent runner for subprocess execution.
    pub agent_runner: Arc<dyn AgentRunner>,
    /// Agent profile repository for dynamic profile resolution.
    pub profile_repo: Arc<dyn AgentProfileRepository>,
    /// Rig-log port for recording operational events (timeouts, idle).
    pub rig_log_port: Arc<dyn RigLogPort>,
    /// Rig-level agent configuration.
    pub rig_config: RigAgentConfig,
    /// Discovered loom IDs (populated at startup, used for shutdown logging).
    pub loom_ids: Vec<LoomId>,
    /// Base (rig) directory path — used by discover and config endpoints.
    pub base_dir: PathBuf,
}
