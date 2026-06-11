//! Shared types for the inbound HTTP adapter.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::application::ports::{
    AgentProfileRepository, AgentRunner, EventSource, LoomLogPort,
    LoomRepository, TieOffSink,
};
use crate::application::store::LoomStore;
use crate::domain::entities::LoomId;
use crate::domain::events::StrandEvent;
use crate::domain::value_objects::{AgentConfig, PromptTemplate, RigAgentConfig};

// ── Request Bodies ────────────────────────────────────────────────────────

/// JSON body for `POST /looms` to register a new loom.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct RegisterLoomRequest {
    /// Unique loom identifier (must end in `-loom`).
    pub id: String,
    /// Knot definitions to write and register.
    pub knots: Vec<KnotRequest>,
}

/// A single knot definition within a `RegisterLoomRequest`.
///
/// All fields are required — `strand_dir` is mandatory.
/// Tie-off paths are now statically derived from loom ID and knot name.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct KnotRequest {
    /// The name of the knot (becomes the `KnotId`).
    pub name: String,
    /// Agent configuration for this knot.
    pub agent_config: AgentConfig,
    /// Reference to a named agent profile (mutually exclusive with
    /// `agent_config` — if both are present, `agent_config` takes
    /// precedence for model/tools overrides).
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub agent_profile_ref: Option<String>,
    /// Prompt template for this knot.
    pub prompt_template: PromptTemplate,
    /// Directory to watch for strand files (required).
    pub strand_dir: String,
}

/// Request body for `POST /profiles/{name}`.
///
/// The `name` is derived from the URL path, not the request body.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProfileRequest {
    /// The LLM provider identifier (e.g. "openai", "anthropic").
    pub provider: String,
    /// The model name to use (e.g. "gpt-4o").
    pub model: String,
    /// Optional list of tool identifiers to enable.
    #[serde(default)]
    #[schema(value_type = Vec<String>)]
    pub tools: Vec<String>,
    /// The system prompt given to the agent (required, non-empty).
    pub system_prompt: String,
}

/// Response body for profile GET/POST endpoints.
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
    /// Rig-level agent configuration.
    pub rig_config: RigAgentConfig,
    /// Discovered loom IDs (populated at startup, used for shutdown logging).
    pub loom_ids: Vec<LoomId>,
    /// Base (rig) directory path — used by discover and config endpoints.
    pub base_dir: PathBuf,
}
