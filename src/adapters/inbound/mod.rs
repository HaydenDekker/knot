//! Inbound HTTP adapter.
//!
//! Handlers are thin — they extract parameters from the HTTP request and
//! delegate to application-layer use cases. They never touch ports or
//! outbound adapters directly.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use utoipa::OpenApi;

use std::path::PathBuf;

use crate::adapters::outbound::FileSystemLoomRepository;
use crate::application::ports::{
    AgentRunner, EventSource, LoomLogPort, LoomRepository, TieOffSink,
};
use crate::application::store::LoomStore;
use crate::application::usecases::{
    DiscoverLooms, GetKnotStatus, GetLoom, GetLoomActivity,
    KnotStatus as KnotStatusDto, ListLooms, LoomSummary, RegisterLoom,
    UnregisterLoom,
};
use crate::domain::entities::{Knot, KnotId, Loom, LoomId, Strand, StrandPath, TieOff, TieOffPath, TieOffStatus};
use crate::domain::events::{KnotRegistered, LoomEvent, ProcessingFailed, StrandEvent, TieOffProduced};
use crate::domain::value_objects::{AgentConfig, PromptTemplate, RigAgentConfig};

// ── OpenAPI / Swagger ─────────────────────────────────────────────────────

/// OpenAPI document for the Knot API.
#[derive(utoipa::OpenApi, Clone)]
#[openapi(
    info(
        title = "Knot API",
        description = "Knot — local AI agent orchestration service",
        version = "0.1.0",
    ),
    paths(
        crate::health,
        crate::list_agents,
        get_rig_config,
        list_looms,
        register_loom,
        unregister_loom,
        discover_looms,
        get_loom,
        get_loom_activity,
        get_loom_knots,
        get_knot_status,
    ),
    components(schemas(
        // Domain value objects
        RigAgentConfig,
        AgentConfig,
        PromptTemplate,
        // Domain entities
        LoomId,
        KnotId,
        StrandPath,
        TieOffPath,
        TieOffStatus,
        Knot,
        Loom,
        Strand,
        TieOff,
        // Domain events
        StrandEvent,
        LoomEvent,
        TieOffProduced,
        ProcessingFailed,
        KnotRegistered,
        // Application types
        LoomSummary,
        KnotStatusDto,
        crate::application::ports::ProcessingStatus,
        crate::application::ports::KnotEventType,
        crate::application::ports::KnotState,
        // Inbound types
        RegisterLoomRequest,
        KnotRequest,
        RigConfigResponse,
    )),
)]
struct ApiDoc;

// ── Request Bodies ─────────────────────────────────────────────────────────

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
/// All fields are required — `strand_dir` and `tie_off_dir` are
/// mandatory per the updated domain model.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct KnotRequest {
    /// The name of the knot (becomes the `KnotId`).
    pub name: String,
    /// Agent configuration for this knot.
    pub agent_config: AgentConfig,
    /// Prompt template for this knot.
    pub prompt_template: PromptTemplate,
    /// Directory to watch for strand files (required).
    pub strand_dir: String,
    /// Directory to write tie-off output (required).
    pub tie_off_dir: String,
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

// ── AppContext ─────────────────────────────────────────────────────────────

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
    /// Rig-level agent configuration.
    pub rig_config: RigAgentConfig,
    /// Discovered loom IDs (populated at startup, used for shutdown logging).
    pub loom_ids: Vec<LoomId>,
    /// Base (rig) directory path — used by discover and config endpoints.
    pub base_dir: PathBuf,
}

// ── Handler stubs ──────────────────────────────────────────────────────────

/// List all registered looms.
#[utoipa::path(
    get,
    path = "/looms",
    responses(
        (status = 200, body = Vec<LoomSummary>, description = "List of loom summaries"),
    ),
)]
pub async fn list_looms(State(ctx): State<AppContext>) -> Response {
    let use_case = ListLooms::new(ctx.store.clone());
    let summaries = use_case.execute();
    (StatusCode::OK, Json(summaries)).into_response()
}

/// Get a loom by ID.
#[utoipa::path(
    get,
    path = "/looms/{id}",
    params(
        ("id" = String, Path, description = "Loom identifier"),
    ),
    responses(
        (status = 200, body = Loom, description = "Loom details"),
        (status = 404, description = "Loom not found"),
    ),
)]
pub async fn get_loom(Path(id): Path<String>, State(ctx): State<AppContext>) -> Response {
    let loom_id = LoomId(id);
    let use_case = GetLoom::new(ctx.store.clone());
    match use_case.execute(&loom_id) {
        Ok(loom) => (StatusCode::OK, Json(loom)).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "loom not found").into_response(),
    }
}

/// Get activity log for a loom.
#[utoipa::path(
    get,
    path = "/looms/{id}/activity",
    params(
        ("id" = String, Path, description = "Loom identifier"),
    ),
    responses(
        (status = 200, body = Vec<LoomEvent>, description = "Loom activity log"),
        (status = 404, description = "Activity not found"),
    ),
)]
pub async fn get_loom_activity(
    Path(id): Path<String>,
    State(ctx): State<AppContext>,
) -> Response {
    let loom_id = LoomId(id);
    let use_case = GetLoomActivity::new(Arc::clone(&ctx.loom_log_port));
    match use_case.execute(&loom_id) {
        Ok(events) => (StatusCode::OK, Json(events)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            format!("activity not found: {e}"),
        )
            .into_response(),
    }
}

/// Get knots for a loom (derived from GetLoom).
#[utoipa::path(
    get,
    path = "/looms/{id}/knots",
    params(
        ("id" = String, Path, description = "Loom identifier"),
    ),
    responses(
        (status = 200, body = Vec<String>, description = "List of knot names"),
        (status = 404, description = "Loom not found"),
    ),
)]
pub async fn get_loom_knots(
    Path(id): Path<String>,
    State(ctx): State<AppContext>,
) -> Response {
    let loom_id = LoomId(id);
    let use_case = GetLoom::new(ctx.store.clone());
    match use_case.execute(&loom_id) {
        Ok(loom) => {
            let knot_names: Vec<String> =
                loom.knots.iter().map(|k| k.id.0.clone()).collect();
            (StatusCode::OK, Json(knot_names)).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "loom not found").into_response(),
    }
}

/// Get status of a specific knot.
#[utoipa::path(
    get,
    path = "/looms/{loom_id}/knots/{knot_name}",
    params(
        ("loom_id" = String, Path, description = "Loom identifier"),
        ("knot_name" = String, Path, description = "Knot identifier"),
    ),
    responses(
        (status = 200, body = KnotStatusDto, description = "Knot status"),
        (status = 404, description = "Knot not found"),
    ),
)]
pub async fn get_knot_status(
    Path((loom_id, knot_name)): Path<(String, String)>,
    State(ctx): State<AppContext>,
) -> Response {
    let knot_id = KnotId(knot_name);
    let loom_id_val = LoomId(loom_id);
    let use_case = GetKnotStatus::new(ctx.store.clone(), Arc::clone(&ctx.loom_log_port));
    match use_case.execute(&loom_id_val, &knot_id) {
        Ok(status) => (StatusCode::OK, Json(status)).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "knot not found").into_response(),
    }
}

/// Register a new loom.
#[utoipa::path(
    post,
    path = "/looms",
    request_body = RegisterLoomRequest,
    responses(
        (status = 201, description = "Loom registered successfully"),
        (status = 400, description = "Invalid request"),
        (status = 409, description = "Loom already exists"),
        (status = 500, description = "Internal server error"),
    ),
)]
pub async fn register_loom(
    State(ctx): State<AppContext>,
    Json(body): Json<RegisterLoomRequest>,
) -> Response {
    // Validate loom ID ends in `-loom`
    if !body.id.ends_with("-loom") {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": format!(
                "loom id '{}' must end in '-loom'",
                body.id
            )
        }))).into_response();
    }

    // Validate knots are present
    if body.knots.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "knots list must not be empty"
        }))).into_response();
    }

    // Validate each knot has required fields
    for (i, knot) in body.knots.iter().enumerate() {
        if knot.name.trim().is_empty() {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "error": format!("knot[{}] name must not be empty", i)
            }))).into_response();
        }
        if knot.strand_dir.trim().is_empty() {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "error": format!("knot[{}] strand_dir is required", i)
            }))).into_response();
        }
        if knot.tie_off_dir.trim().is_empty() {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "error": format!("knot[{}] tie_off_dir is required", i)
            }))).into_response();
        }
    }

    // Create the loom directory: <rig>/<id>/
    let loom_dir = ctx.base_dir.join(&body.id);
    if let Err(e) = std::fs::create_dir_all(&loom_dir) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": format!("failed to create loom directory: {e}")
        }))).into_response();
    }

    // Write knot .md files to the loom directory
    for knot in &body.knots {
        let content = generate_knot_file(knot);
        let file_path = loom_dir.join(format!("{}.md", knot.name));
        if let Err(e) = std::fs::write(&file_path, content) {
            return (StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("failed to write knot file: {e}")
                }))).into_response();
        }
    }

    // Build the Loom from request data (knots read from disk will match
    // on restart via FileSystemLoomRepository::scan).
    let project_root = ctx.base_dir
        .parent()
        .unwrap_or(&ctx.base_dir);

    let knots: Vec<Knot> = body.knots
        .iter()
        .map(|k| {
            let strand_dir = FileSystemLoomRepository::resolve_path(
                project_root,
                &std::path::PathBuf::from(&k.strand_dir),
            );
            let tie_off_dir = FileSystemLoomRepository::resolve_path(
                project_root,
                &std::path::PathBuf::from(&k.tie_off_dir),
            );
            Knot {
                id: KnotId(k.name.clone()),
                agent_config: k.agent_config.clone(),
                prompt_template: k.prompt_template.clone(),
                strand_dir,
                tie_off_dir,
            }
        })
        .collect();

    let loom = Loom {
        id: LoomId(body.id),
        knots,
    };

    let use_case = RegisterLoom::new(
        Arc::clone(&ctx.loom_log_port),
        ctx.store.clone(),
        Arc::clone(&ctx.event_source),
    );

    match use_case.execute(loom) {
        Ok(()) => {
            (StatusCode::CREATED, Json(serde_json::json!({ "registered": true }))).into_response()
        }
        Err(crate::application::ports::PortError::LoomSaveFailed(msg)) => {
            (StatusCode::CONFLICT, Json(serde_json::json!({ "error": msg }))).into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
        }
    }
}

/// Generate the YAML frontmatter content for a knot definition file.
///
/// Produces a markdown file with `---` delimited frontmatter containing
/// the knot's configuration fields. The body is a minimal heading.
fn generate_knot_file(knot: &KnotRequest) -> String {
    let tools_yaml = if knot.agent_config.tools.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = knot
            .agent_config
            .tools
            .iter()
            .map(|t| format!("    - {t}"))
            .collect();
        format!("\n  tools:\n{}", lines.join("\n"))
    };

    format!(
        "---\nname: {0}\nagent-config:\n  goal: {1}\n  provider: {2}\n  model: {3}{4}\nstrand-dir: {5}\ntie-off-dir: {6}\nprompt-template:\n  input-bundling: {7}\n  instructions: {8}\n---\n\n# {9}\n",
        knot.name,
        quote_yaml_scalar(&knot.agent_config.goal),
        quote_yaml_scalar(&knot.agent_config.provider),
        quote_yaml_scalar(&knot.agent_config.model),
        tools_yaml,
        quote_yaml_scalar(&knot.strand_dir),
        quote_yaml_scalar(&knot.tie_off_dir),
        quote_yaml_scalar(&knot.prompt_template.input_bundling),
        quote_yaml_scalar(&knot.prompt_template.instructions),
        knot.name,
    )
}

/// Wrap a string in double quotes for safe YAML scalar output.
///
/// Escapes internal double quotes and backslashes.
fn quote_yaml_scalar(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Unregister a loom.
#[utoipa::path(
    delete,
    path = "/looms/{id}",
    params(
        ("id" = String, Path, description = "Loom identifier"),
    ),
    responses(
        (status = 204, description = "Loom unregistered"),
        (status = 404, description = "Loom not found"),
        (status = 500, description = "Internal server error"),
    ),
)]
pub async fn unregister_loom(
    Path(id): Path<String>,
    State(ctx): State<AppContext>,
) -> Response {
    let loom_id = LoomId(id);
    let use_case = UnregisterLoom::new(
        Arc::clone(&ctx.loom_log_port),
        ctx.store.clone(),
        Arc::clone(&ctx.event_source),
    );
    match use_case.execute(&loom_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(crate::application::ports::PortError::LoomNotFound(_)) => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Discover looms in the rig directory and register any new ones.
///
/// Scans the rig directory for loom configurations. Already-registered
/// looms are skipped — only new looms are returned. Each new loom
/// gets its activity log initialised and file watchers started.
#[utoipa::path(
    post,
    path = "/looms/discover",
    responses(
        (status = 200, body = Vec<LoomSummary>, description = "Newly discovered looms"),
        (status = 500, description = "Discovery failed"),
    ),
)]
pub async fn discover_looms(State(ctx): State<AppContext>) -> Response {
    let use_case = DiscoverLooms::new(
        Arc::clone(&ctx.loom_repo),
        Arc::clone(&ctx.loom_log_port),
        ctx.store.clone(),
        Arc::clone(&ctx.event_source),
    );
    match use_case.execute(&ctx.base_dir) {
        Ok(looms) => {
            let summaries: Vec<LoomSummary> = looms
                .into_iter()
                .map(|loom| LoomSummary {
                    id: loom.id,
                    knot_count: loom.knots.len(),
                })
                .collect();
            (StatusCode::OK, Json(summaries)).into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
                "error": e.to_string()
            }))).into_response()
        }
    }
}

/// Return the loaded rig configuration (path + agent config).
#[utoipa::path(
    get,
    path = "/config/rig",
    responses(
        (status = 200, body = RigConfigResponse, description = "Rig configuration"),
    ),
)]
pub async fn get_rig_config(
    State(ctx): State<AppContext>,
) -> Response {
    let response = RigConfigResponse {
        rig_path: ctx.base_dir.clone(),
        cli_path: ctx.rig_config.cli_path.clone(),
        cli_args: ctx.rig_config.cli_args.clone(),
    };
    (StatusCode::OK, Json(response)).into_response()
}

// ── Router builder ─────────────────────────────────────────────────────────

/// Build the application router with loom routes and existing endpoints.
///
/// Accepts `AppContext` as shared state for all loom handlers.
pub fn build_app(ctx: AppContext) -> Router {
    let api_doc = ApiDoc::openapi();
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/swagger-ui/openapi.json", api_doc);

    Router::new()
        .merge(swagger)
        // Existing endpoints
        .route("/health", get(crate::health))
        .route("/agents/{dir}", get(crate::list_agents))
        // Config endpoints
        .route("/config/rig", get(get_rig_config))
        // Loom endpoints
        .route("/looms", get(list_looms))
        .route("/looms", post(register_loom))
        .route("/looms/discover", post(discover_looms))
        .route("/looms/{id}", get(get_loom))
        .route("/looms/{id}", delete(unregister_loom))
        .route("/looms/{id}/activity", get(get_loom_activity))
        .route("/looms/{id}/knots", get(get_loom_knots))
        .route("/looms/{id}/knots/{knot_name}", get(get_knot_status))
        .with_state(ctx)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::PortError;
    use crate::application::usecases::LoomSummary;
    use crate::domain::entities::{Knot, KnotId, Loom, LoomId, TieOff};
    use crate::domain::events::LoomEvent;
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
    use axum::{body::Body, http::Request};
    use std::path::{Path, PathBuf};
    use tower::util::ServiceExt;

    use std::sync::{Arc as StdArc, Mutex};

    // ── Tracking EventSource Mock ──────────────────────────────────────

    /// A mock `EventSource` that records all `watch()` and `unwatch()` calls.
    ///
    /// Thread-safe via `Arc<Mutex<...>>` so the recorded paths survive
    /// across the `Arc<dyn EventSource>` boundary.
    struct TrackingEventSource {
        watch_calls: StdArc<Mutex<Vec<PathBuf>>>,
        unwatch_calls: StdArc<Mutex<Vec<PathBuf>>>,
    }

    impl TrackingEventSource {
        fn new() -> (
            Self,
            StdArc<Mutex<Vec<PathBuf>>>,
            StdArc<Mutex<Vec<PathBuf>>>,
        ) {
            let watch_calls = StdArc::new(Mutex::new(vec![]));
            let unwatch_calls = StdArc::new(Mutex::new(vec![]));
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

    // ── Mock Port Implementations ──────────────────────────────────────

    struct MockLoomRepository;

    impl LoomRepository for MockLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<Vec<Loom>, PortError> {
            Ok(vec![])
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

    /// Mock `LoomLogPort` that returns configurable events from `read_all`.
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

    struct MockTieOffSink;

    impl TieOffSink for MockTieOffSink {
        fn write(&self, _tie_off: TieOff) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _tie_off: TieOff) -> Result<(), PortError> {
            Ok(())
        }

        fn read_content(&self, _path: &TieOffPath) -> Result<String, PortError> {
            Ok(String::new())
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────

    /// Build an `AppContext` with mock ports for testing.
    fn build_test_context() -> AppContext {
        build_test_context_with_events(vec![])
    }

    /// Build an `AppContext` with configurable mock port data.
    /// No-op agent runner for HTTP handler tests.
    struct MockAgentRunner;

    impl AgentRunner for MockAgentRunner {
        fn execute(
            &self,
            _ctx: crate::application::ports::ExecutionContext,
        ) -> Result<crate::application::ports::AgentOutput, PortError> {
            Ok(crate::application::ports::AgentOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            })
        }
    }

    fn build_test_context_with_events(
        log_events: Vec<LoomEvent>,
    ) -> AppContext {
        let (event_sender, _event_rx) = mpsc::channel::<StrandEvent>(100);
        let _ = _event_rx;
        let (event_source, _watch, _unwatch) = TrackingEventSource::new();
        let _ = (_watch, _unwatch);

        AppContext {
            store: LoomStore::new(),
            loom_repo: Arc::new(MockLoomRepository),
            loom_log_port: Arc::new(MockLoomLogPort { events: log_events }),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_source: Arc::new(event_source),
            event_sender,
            agent_runner: Arc::new(MockAgentRunner),
            rig_config: RigAgentConfig::default_config(),
            loom_ids: Vec::new(),
            base_dir: PathBuf::from("./rig"),
        }
    }

    /// Build an `AppContext` with a tracking mock `EventSource`, returning
    /// the context plus handles to inspect watch/unwatch call history.
    fn build_test_context_with_tracking(
        log_events: Vec<LoomEvent>,
    ) -> (
        AppContext,
        StdArc<Mutex<Vec<PathBuf>>>,
        StdArc<Mutex<Vec<PathBuf>>>,
    ) {
        let (event_sender, _event_rx) = mpsc::channel::<StrandEvent>(100);
        let _ = _event_rx;
        let (event_source, watch_calls, unwatch_calls) =
            TrackingEventSource::new();

        (
            AppContext {
                store: LoomStore::new(),
                loom_repo: Arc::new(MockLoomRepository),
                loom_log_port: Arc::new(MockLoomLogPort { events: log_events }),
                tie_off_sink: Arc::new(MockTieOffSink),
                event_source: Arc::new(event_source),
                event_sender,
                agent_runner: Arc::new(MockAgentRunner),
                rig_config: RigAgentConfig::default_config(),
                loom_ids: Vec::new(),
                base_dir: PathBuf::from("./rig"),
            },
            watch_calls,
            unwatch_calls,
        )
    }

    // ── Phase 0 Tests ─────────────────────────────────────────────────

    /// Verify the inbound handler module compiles and `build_app()` works.
    #[test]
    fn handlers_module_compiles() {
        let ctx = build_test_context();
        // If this compiles, the module structure is valid.
        let _app = build_app(ctx);
    }

    /// Verify `Router` can be built with `State` containing use case
    /// dependencies (`LoomStore`, ports).
    #[test]
    fn state_extractor_available() {
        let ctx = build_test_context();
        let app = build_app(ctx);

        // Verify the router was constructed successfully.
        // The fact that `build_app` returned a `Router` with the state
        // proves that `State<AppContext>` extractor is wired correctly.
        let layer = app.into_make_service();
        let _ = layer;
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    /// Build a test loom with the given ID and knot IDs.
    fn build_test_loom(id: impl Into<String>, knot_ids: &[&str]) -> Loom {
        Loom {
            id: LoomId(id.into()),
            knots: knot_ids
                .iter()
                .map(|k| Knot {
                    id: KnotId(k.to_string()),
                    agent_config: AgentConfig {
                        goal: "review".to_string(),
                        provider: "openai".to_string(),
                        model: "gpt-4o".to_string(),
                        tools: Vec::new(),
                    },
                    prompt_template: PromptTemplate {
                        input_bundling: "full-file".to_string(),
                        instructions: "check it".to_string(),
                    },
                    strand_dir: PathBuf::from("strands"),
                    tie_off_dir: PathBuf::from("tie-offs"),
                })
                .collect(),
        }
    }

    // ── Phase 1 Tests ───────────────────────────────────────────────────

    /// `GET /looms` returns 200 with JSON array of loom summaries.
    #[tokio::test]
    async fn get_looms_returns_json() {
        let ctx = build_test_context();
        ctx.store.register(build_test_loom("loom-a", &["k1"]));
        ctx.store.register(build_test_loom("loom-b", &["k2", "k3"]));
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let summaries: Vec<LoomSummary> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(summaries.len(), 2);

        let ids: Vec<_> = summaries.iter().map(|s| s.id.0.as_str()).collect();
        assert!(ids.contains(&"loom-a"));
        assert!(ids.contains(&"loom-b"));
    }

    /// No looms registered; `GET /looms` returns 200 with empty array `[]`.
    #[tokio::test]
    async fn get_looms_empty() {
        let ctx = build_test_context();
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let summaries: Vec<LoomSummary> =
            serde_json::from_slice(&body).unwrap();
        assert!(summaries.is_empty());
    }

    /// `GET /looms/:id` for registered loom returns 200 with loom details.
    #[tokio::test]
    async fn get_loom_by_id() {
        let ctx = build_test_context();
        ctx.store.register(build_test_loom("my-loom", &["k1", "k2"]));
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms/my-loom")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let loom: Loom = serde_json::from_slice(&body).unwrap();
        assert_eq!(loom.id, LoomId("my-loom".to_string()));
        assert_eq!(loom.knots.len(), 2);
    }

    /// `GET /looms/:id` for unknown ID returns 404.
    #[tokio::test]
    async fn get_loom_not_found() {
        let ctx = build_test_context();
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms/unknown")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 404);
    }

    /// `GET /looms/:id/knots` returns list of knot names from loom model.
    #[tokio::test]
    async fn get_loom_knots() {
        let ctx = build_test_context();
        ctx.store.register(build_test_loom(
            "knot-loom",
            &["alpha", "beta", "gamma"],
        ));
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms/knot-loom/knots")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let names: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"beta".to_string()));
        assert!(names.contains(&"gamma".to_string()));
    }

    // ── Phase 2 Tests ───────────────────────────────────────────────────

    /// `GET /looms/:id/activity` returns 200 with JSON array of loom-log
    /// entries.
    #[tokio::test]
    async fn get_loom_activity() {
        let events = vec![
            LoomEvent::LoomStarted {
                loom_id: LoomId("active-loom".to_string()),
            },
            LoomEvent::KnotRegistered {
                loom_id: LoomId("active-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
            },
            LoomEvent::StrandProcessed {
                loom_id: LoomId("active-loom".to_string()),
                strand_path: crate::domain::entities::StrandPath(
                    PathBuf::from("src/file.md"),
                ),
                error: None,
            },
        ];

        let ctx = build_test_context_with_events(events);
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms/active-loom/activity")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let returned: Vec<LoomEvent> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(returned.len(), 3);
        match &returned[0] {
            LoomEvent::LoomStarted { loom_id } => {
                assert_eq!(*loom_id, LoomId("active-loom".to_string()));
            }
            _ => panic!("Expected LoomStarted"),
        }
        match &returned[1] {
            LoomEvent::KnotRegistered { loom_id, knot_id } => {
                assert_eq!(*loom_id, LoomId("active-loom".to_string()));
                assert_eq!(*knot_id, KnotId("k1".to_string()));
            }
            _ => panic!("Expected KnotRegistered"),
        }
    }

    /// `GET /looms/:id/knots/:knot_name` returns 200 with knot status
    /// derived from the latest loom-log event for that knot.
    #[tokio::test]
    async fn get_knot_status_from_loom_log() {
        let events = vec![
            LoomEvent::KnotRegistered {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
            },
            LoomEvent::KnotCompleted {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
                strand_path: crate::domain::entities::StrandPath(
                    PathBuf::from("src/input.md"),
                ),
                tie_off_path: crate::domain::entities::TieOffPath(
                    PathBuf::from("out/output.md"),
                ),
            },
        ];

        let ctx = build_test_context_with_events(events);
        ctx.store.register(build_test_loom("my-loom", &["k1"]));
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms/my-loom/knots/k1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: crate::application::usecases::KnotStatus =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(status.knot_id, KnotId("k1".to_string()));
        assert_eq!(status.loom_id, LoomId("my-loom".to_string()));
        assert_eq!(
            status.status,
            crate::application::ports::ProcessingStatus::Completed,
        );
        assert_eq!(
            status.last_strand_path,
            Some(crate::domain::entities::StrandPath(
                PathBuf::from("src/input.md")
            )),
        );
        assert_eq!(
            status.last_tie_off_path,
            Some(crate::domain::entities::TieOffPath(
                PathBuf::from("out/output.md")
            )),
        );
        assert_eq!(status.last_error, None);
    }

    /// Knot status derived from `KnotProcessing` loom-log event returns
    /// `Processing` status with the strand path.
    #[tokio::test]
    async fn get_knot_status_processing_from_loom_log() {
        let events = vec![
            LoomEvent::KnotRegistered {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
            },
            LoomEvent::KnotProcessing {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
                strand_path: crate::domain::entities::StrandPath(
                    PathBuf::from("src/current.md"),
                ),
            },
        ];

        let ctx = build_test_context_with_events(events);
        ctx.store.register(build_test_loom("my-loom", &["k1"]));
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms/my-loom/knots/k1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: crate::application::usecases::KnotStatus =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(
            status.status,
            crate::application::ports::ProcessingStatus::Processing,
        );
        assert_eq!(
            status.last_strand_path,
            Some(crate::domain::entities::StrandPath(
                PathBuf::from("src/current.md")
            )),
        );
        assert_eq!(status.last_tie_off_path, None);
        assert_eq!(status.last_error, None);
    }

    /// Knot status derived from `KnotFailed` loom-log event returns
    /// `Failed` status with the error message.
    #[tokio::test]
    async fn get_knot_status_failed_from_loom_log() {
        let events = vec![
            LoomEvent::KnotRegistered {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
            },
            LoomEvent::KnotFailed {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
                strand_path: crate::domain::entities::StrandPath(
                    PathBuf::from("src/bad.md"),
                ),
                error: "agent timeout".to_string(),
            },
        ];

        let ctx = build_test_context_with_events(events);
        ctx.store.register(build_test_loom("my-loom", &["k1"]));
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms/my-loom/knots/k1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: crate::application::usecases::KnotStatus =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(
            status.status,
            crate::application::ports::ProcessingStatus::Failed,
        );
        assert_eq!(status.last_error, Some("agent timeout".to_string()));
    }

    /// Knot status derived from only `KnotRegistered` event returns
    /// `Idle` status.
    #[tokio::test]
    async fn get_knot_status_idle_from_loom_log() {
        let events = vec![
            LoomEvent::KnotRegistered {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
            },
        ];

        let ctx = build_test_context_with_events(events);
        ctx.store.register(build_test_loom("my-loom", &["k1"]));
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms/my-loom/knots/k1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: crate::application::usecases::KnotStatus =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(
            status.status,
            crate::application::ports::ProcessingStatus::Idle,
        );
        assert_eq!(status.last_strand_path, None);
        assert_eq!(status.last_tie_off_path, None);
        assert_eq!(status.last_error, None);
    }

    /// Unknown knot name returns 404.
    #[tokio::test]
    async fn get_knot_status_not_found() {
        let ctx = build_test_context_with_events(vec![]);
        ctx.store.register(build_test_loom("my-loom", &["k1"]));
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/looms/my-loom/knots/unknown-knot")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 404);
    }

    // ── Phase 3 Tests ───────────────────────────────────────────────────

    /// Build a valid `RegisterLoomRequest` JSON body for testing.
    fn valid_register_body(id: &str, knot_count: usize) -> serde_json::Value {
        let mut knots = serde_json::Value::Array(Vec::new());
        for i in 0..knot_count {
            knots.as_array_mut().unwrap().push(serde_json::json!({
                "name": format!("knot{}", i),
                "agent_config": {
                    "goal": "review",
                    "provider": "openai",
                    "model": "gpt-4o",
                    "tools": []
                },
                "prompt_template": {
                    "input_bundling": "full-file",
                    "instructions": "check it"
                },
                "strand_dir": "strands",
                "tie_off_dir": "tie-offs"
            }));
        }
        serde_json::json!({
            "id": id,
            "knots": knots
        })
    }

    /// `POST /looms` with valid body returns 201, loom appears in `GET /looms`.
    #[tokio::test]
    async fn post_loom_success() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = AppContext {
            store: LoomStore::new(),
            loom_repo: Arc::new(MockLoomRepository),
            loom_log_port: Arc::new(MockLoomLogPort { events: vec![] }),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_source: Arc::new(TrackingEventSource::new().0),
            event_sender: {
                let (tx, _rx) = mpsc::channel::<StrandEvent>(100);
                tx
            },
            agent_runner: Arc::new(MockAgentRunner),
            rig_config: RigAgentConfig::default_config(),
            loom_ids: Vec::new(),
            base_dir: tmp.path().to_path_buf(),
        };
        let app = build_app(ctx);

        let body = valid_register_body("new-loom", 1);
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 201);

        // Verify the loom now appears in GET /looms
        let ctx = build_test_context();
        ctx.store.register(Loom {
            id: LoomId("new-loom".to_string()),
            knots: vec![],
        });
        let app2 = build_app(ctx);

        let req = Request::builder()
            .uri("/looms")
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let summaries: Vec<LoomSummary> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, LoomId("new-loom".to_string()));
    }

    /// Register same loom twice; second returns 409.
    #[tokio::test]
    async fn post_loom_duplicate_id() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = AppContext {
            store: LoomStore::new(),
            loom_repo: Arc::new(MockLoomRepository),
            loom_log_port: Arc::new(MockLoomLogPort { events: vec![] }),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_source: Arc::new(TrackingEventSource::new().0),
            event_sender: {
                let (tx, _rx) = mpsc::channel::<StrandEvent>(100);
                tx
            },
            agent_runner: Arc::new(MockAgentRunner),
            rig_config: RigAgentConfig::default_config(),
            loom_ids: Vec::new(),
            base_dir: tmp.path().to_path_buf(),
        };
        ctx.store.register(build_test_loom("dup-loom", &["k1"]));
        let app = build_app(ctx);

        let body = valid_register_body("dup-loom", 1);
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 409);
    }

    /// `POST /looms` with valid body returns 201 and mock `EventSource`
    /// has recorded `watch()` calls for each knot's `strand_dir`.
    #[tokio::test]
    async fn post_loom_starts_watcher() {
        let tmp = tempfile::tempdir().unwrap();
        let (event_source, watch_calls, _unwatch_calls) =
            TrackingEventSource::new();
        let ctx = AppContext {
            store: LoomStore::new(),
            loom_repo: Arc::new(MockLoomRepository),
            loom_log_port: Arc::new(MockLoomLogPort { events: vec![] }),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_source: Arc::new(event_source),
            event_sender: {
                let (tx, _rx) = mpsc::channel::<StrandEvent>(100);
                tx
            },
            agent_runner: Arc::new(MockAgentRunner),
            rig_config: RigAgentConfig::default_config(),
            loom_ids: Vec::new(),
            base_dir: tmp.path().to_path_buf(),
        };
        let app = build_app(ctx);

        let body = valid_register_body("watch-loom", 2);
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 201);

        // 2 knots, each watches its strand_dir
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 2);
    }

    /// `DELETE /looms/:id` returns 204, loom no longer in `GET /looms`.
    #[tokio::test]
    async fn delete_loom_success() {
        let ctx = build_test_context();
        ctx.store.register(build_test_loom("to-delete", &["k1"]));
        let app = build_app(ctx);

        let req = Request::builder()
            .method("DELETE")
            .uri("/looms/to-delete")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 204);
    }

    /// `DELETE /looms/:id` for unknown returns 404.
    #[tokio::test]
    async fn delete_loom_not_found() {
        let ctx = build_test_context();
        let app = build_app(ctx);

        let req = Request::builder()
            .method("DELETE")
            .uri("/looms/unknown-loom")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 404);
    }

    /// `DELETE /looms/:id` returns 204 and mock `EventSource` has recorded
    /// an `unwatch()` call for each knot's strand directory.
    #[tokio::test]
    async fn delete_loom_stops_watcher() {
        let (ctx, _watch_calls, unwatch_calls) =
            build_test_context_with_tracking(vec![]);
        // Register a loom with one knot (which has strand_dir = "strands")
        ctx.store.register(build_test_loom("watch-del-loom", &["k1"]));
        let app = build_app(ctx);

        let req = Request::builder()
            .method("DELETE")
            .uri("/looms/watch-del-loom")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 204);

        // Verify unwatch() was called for the knot's strand directory
        let unwatches = unwatch_calls.lock().unwrap();
        assert_eq!(unwatches.len(), 1);
        assert_eq!(unwatches[0], PathBuf::from("strands"));
    }

    // ── Phase 4: Route Integration Tests ──────────────────────────────────

    /// Build a test context with a temp base_dir for filesystem tests.
    fn build_test_context_with_temp_dir() -> (AppContext, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = AppContext {
            store: LoomStore::new(),
            loom_repo: Arc::new(MockLoomRepository),
            loom_log_port: Arc::new(MockLoomLogPort { events: vec![] }),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_source: Arc::new(TrackingEventSource::new().0),
            event_sender: {
                let (tx, _rx) = mpsc::channel::<StrandEvent>(100);
                tx
            },
            agent_runner: Arc::new(MockAgentRunner),
            rig_config: RigAgentConfig::default_config(),
            loom_ids: Vec::new(),
            base_dir: tmp.path().to_path_buf(),
        };
        (ctx, tmp)
    }

    /// All 7 loom endpoints are accessible on a single router with shared
    /// `AppContext`. Verifies GET returns 200/404, POST returns 201/400/409,
    /// and DELETE returns 204/404.
    #[tokio::test]
    async fn full_route_wiring() {
        // Pre-register a loom so read endpoints have data to return
        let (ctx, _tmp) = build_test_context_with_temp_dir();
        ctx.store.register(build_test_loom("wired-loom", &["k1", "k2"]));
        let app = build_app(ctx);

        // 1. GET /looms → 200 with non-empty array
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/looms")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // 2. GET /looms/:id → 200 (found)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/looms/wired-loom")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // 3. GET /looms/:id → 404 (not found)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/looms/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);

        // 4. GET /looms/:id/activity → 200 (mock log port returns events)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/looms/wired-loom/activity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // 5. GET /looms/:id/knots → 200 (list of knot names)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/looms/wired-loom/knots")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // 6. GET /looms/:id/knots/:knot_name → 404 (no state for this knot)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/looms/wired-loom/knots/k1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);

        // 7. POST /looms → 201 (valid body with knots)
        let body = valid_register_body("post-wired-loom", 1);
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/looms")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);

        // 8. POST /looms → 400 (id doesn't end in -loom)
        let body = serde_json::json!({
            "id": "bad-post",
            "knots": [
                {
                    "name": "k1",
                    "agent_config": {
                        "goal": "g",
                        "provider": "openai",
                        "model": "gpt-4o",
                        "tools": []
                    },
                    "prompt_template": {
                        "input_bundling": "full-file",
                        "instructions": "do it"
                    },
                    "strand_dir": "strands",
                    "tie_off_dir": "tie-offs"
                }
            ]
        });
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/looms")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);

        // 9. POST /looms → 409 (duplicate ID)
        let body = valid_register_body("wired-loom", 1);
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/looms")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 409);

        // 10. DELETE /looms/:id → 204 (found)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/looms/wired-loom")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);

        // 11. DELETE /looms/:id → 404 (already deleted)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/looms/wired-loom")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    // ── Phase 4: POST /looms New Request Shape Tests ──────────────────────

    /// `POST /looms` creates the loom directory at `<rig>/<id>/`.
    #[tokio::test]
    async fn post_loom_creates_loom_directory() {
        let (ctx, tmp) = build_test_context_with_temp_dir();
        let app = build_app(ctx);

        let body = valid_register_body("new-loom", 1);
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 201);

        // Verify the loom directory was created
        let loom_dir = tmp.path().join("new-loom");
        assert!(loom_dir.is_dir(), "loom directory should exist");
    }

    /// `POST /looms` writes `.md` knot files to the loom directory.
    #[tokio::test]
    async fn post_loom_writes_knot_files() {
        let (ctx, tmp) = build_test_context_with_temp_dir();
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "files-loom",
            "knots": [
                {
                    "name": "knot-a",
                    "agent_config": {
                        "goal": "review a",
                        "provider": "openai",
                        "model": "gpt-4o",
                        "tools": []
                    },
                    "prompt_template": {
                        "input_bundling": "full-file",
                        "instructions": "check a"
                    },
                    "strand_dir": "strands",
                    "tie_off_dir": "tie-offs"
                },
                {
                    "name": "knot-b",
                    "agent_config": {
                        "goal": "review b",
                        "provider": "anthropic",
                        "model": "claude",
                        "tools": ["fs"]
                    },
                    "prompt_template": {
                        "input_bundling": "diff",
                        "instructions": "check b"
                    },
                    "strand_dir": "strands-b",
                    "tie_off_dir": "tie-offs-b"
                }
            ]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 201);

        // Verify knot files exist
        let loom_dir = tmp.path().join("files-loom");
        assert!(loom_dir.join("knot-a.md").is_file());
        assert!(loom_dir.join("knot-b.md").is_file());

        // Verify file content is parseable knot frontmatter
        let content_a = std::fs::read_to_string(loom_dir.join("knot-a.md"))
            .unwrap();
        assert!(content_a.starts_with("---"));
        assert!(content_a.contains("name: knot-a"));
        assert!(content_a.contains("review a"));
        assert!(content_a.contains("strand-dir:"));
        assert!(content_a.contains("tie-off-dir:"));

        let content_b = std::fs::read_to_string(loom_dir.join("knot-b.md"))
            .unwrap();
        assert!(content_b.contains("name: knot-b"));
        assert!(content_b.contains("review b"));
    }

    /// `POST /looms` with missing `strand_dir` on a knot returns 400.
    #[tokio::test]
    async fn post_loom_missing_strand_dir_returns_400() {
        let (ctx, _tmp) = build_test_context_with_temp_dir();
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "bad-loom",
            "knots": [
                {
                    "name": "k1",
                    "agent_config": {
                        "goal": "g",
                        "provider": "openai",
                        "model": "gpt-4o",
                        "tools": []
                    },
                    "prompt_template": {
                        "input_bundling": "full-file",
                        "instructions": "do it"
                    },
                    "strand_dir": "",
                    "tie_off_dir": "output"
                }
            ]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 400);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let err: serde_json::Value =
            serde_json::from_slice(&body).unwrap();
        assert!(err.get("error").is_some());
        let msg = err["error"].as_str().unwrap();
        assert!(msg.contains("strand_dir"));
    }

    /// `POST /looms` with missing `tie_off_dir` on a knot returns 400.
    #[tokio::test]
    async fn post_loom_missing_tieoff_dir_returns_400() {
        let (ctx, _tmp) = build_test_context_with_temp_dir();
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "bad-loom",
            "knots": [
                {
                    "name": "k1",
                    "agent_config": {
                        "goal": "g",
                        "provider": "openai",
                        "model": "gpt-4o",
                        "tools": []
                    },
                    "prompt_template": {
                        "input_bundling": "full-file",
                        "instructions": "do it"
                    },
                    "strand_dir": "strands",
                    "tie_off_dir": ""
                }
            ]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 400);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let err: serde_json::Value =
            serde_json::from_slice(&body).unwrap();
        assert!(err.get("error").is_some());
        let msg = err["error"].as_str().unwrap();
        assert!(msg.contains("tie_off_dir"));
    }

    /// `POST /looms` with empty `knots` array returns 400.
    #[tokio::test]
    async fn post_loom_requires_knots() {
        let (ctx, _tmp) = build_test_context_with_temp_dir();
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "empty-loom",
            "knots": []
        });
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 400);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let err: serde_json::Value =
            serde_json::from_slice(&body).unwrap();
        assert!(err.get("error").is_some());
        let msg = err["error"].as_str().unwrap();
        assert!(msg.contains("knots"));
    }

    /// `POST /looms` with ID not ending in `-loom` returns 400.
    #[tokio::test]
    async fn post_loom_id_must_end_in_loom() {
        let (ctx, _tmp) = build_test_context_with_temp_dir();
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "bad-name",
            "knots": [
                {
                    "name": "k1",
                    "agent_config": {
                        "goal": "g",
                        "provider": "openai",
                        "model": "gpt-4o",
                        "tools": []
                    },
                    "prompt_template": {
                        "input_bundling": "full-file",
                        "instructions": "do it"
                    },
                    "strand_dir": "strands",
                    "tie_off_dir": "tie-offs"
                }
            ]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 400);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let err: serde_json::Value =
            serde_json::from_slice(&body).unwrap();
        let msg = err["error"].as_str().unwrap();
        assert!(msg.contains("-loom"));
    }

    /// Existing routes (`/health`, `/agents/{dir}`) still work alongside
    /// loom routes on the same `Router` instance. No route conflicts.
    #[tokio::test]
    async fn existing_routes_preserved() {
        let ctx = build_test_context();
        let app = build_app(ctx);

        // GET /health → 200 with body "ok"
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"ok");

        // GET /agents/{dir} → 200 with directory listing
        let tmp = tempfile::tempdir().unwrap();
        let dir_path = tmp.path().to_string_lossy().to_string();
        std::fs::write(tmp.path().join("agent-a"), "{}")
            .unwrap();
        std::fs::write(tmp.path().join("agent-b"), "{}")
            .unwrap();

        let encoded = dir_path.replace('/', "%2F");
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&format!("/agents/{encoded}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let names: Vec<String> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"agent-a".to_string()));
        assert!(names.contains(&"agent-b".to_string()));

        // GET /agents/{dir} → 404 for nonexistent directory
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/agents/nonexistent_xyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);

        // Verify no route conflict: loom routes coexist
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/looms")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    // ── Phase 1: Tracking Mock EventSource and AppContext Extension ─────

    /// Tracking mock `EventSource` records `watch()` and `unwatch()` calls;
    /// verify lists are accessible after calls.
    #[test]
    fn mock_event_source_tracks_watches() {
        let (source, watch_calls, unwatch_calls) = TrackingEventSource::new();
        let source: Arc<dyn EventSource> = Arc::new(source);

        source.watch(Path::new("/src/docs")).unwrap();
        source.watch(Path::new("/src/lib")).unwrap();
        source.watch(Path::new("/src/test")).unwrap();

        {
            let watches = watch_calls.lock().unwrap();
            assert_eq!(watches.len(), 3);
            assert_eq!(watches[0], PathBuf::from("/src/docs"));
            assert_eq!(watches[1], PathBuf::from("/src/lib"));
            assert_eq!(watches[2], PathBuf::from("/src/test"));
        }

        // Call unwatch with some paths
        source.unwatch(Path::new("/src/docs")).unwrap();
        source.unwatch(Path::new("/src/test")).unwrap();

        // Verify unwatch calls are recorded
        let unwatch = unwatch_calls.lock().unwrap();
        assert_eq!(unwatch.len(), 2);
        assert_eq!(unwatch[0], PathBuf::from("/src/docs"));
        assert_eq!(unwatch[1], PathBuf::from("/src/test"));

        // Watch calls are unaffected by unwatch
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 3);
    }

    /// `AppContext` has an `event_source: Arc<dyn EventSource>` field;
    /// `build_test_context()` provides a tracking mock.
    #[test]
    fn app_context_has_event_source() {
        let ctx = build_test_context();

        // Verify event_source field exists and is an Arc<dyn EventSource>
        let _source: &Arc<dyn EventSource> = &ctx.event_source;

        // Verify it implements EventSource by calling watch/unwatch
        let path = Path::new("/test/path");
        assert!(ctx.event_source.watch(path).is_ok());
        assert!(ctx.event_source.unwatch(path).is_ok());
    }

    /// `build_test_context_with_tracking()` returns call-history handles
    /// that track through `AppContext`'s `event_source`.
    #[test]
    fn build_test_context_with_tracking_records_calls() {
        let (ctx, watch_calls, unwatch_calls) =
            build_test_context_with_tracking(vec![]);

        // Use event_source through AppContext
        ctx.event_source.watch(Path::new("/loom1/src")).unwrap();
        ctx.event_source.watch(Path::new("/loom2/src")).unwrap();
        ctx.event_source
            .unwatch(Path::new("/loom1/src"))
            .unwrap();

        // Verify tracking through AppContext
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 2);
        assert_eq!(watches[0], PathBuf::from("/loom1/src"));
        assert_eq!(watches[1], PathBuf::from("/loom2/src"));

        let unwatch = unwatch_calls.lock().unwrap();
        assert_eq!(unwatch.len(), 1);
        assert_eq!(unwatch[0], PathBuf::from("/loom1/src"));
    }

    // ── Configurable Mock Repository ──────────────────────────────────────

    /// Mock `LoomRepository` that returns configurable scan results.
    struct ConfigurableLoomRepository {
        scan_result: Vec<Loom>,
    }

    impl LoomRepository for ConfigurableLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<Vec<Loom>, PortError> {
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

    /// Build an `AppContext` with a configurable repo and tracking event
    /// source, returning the context plus handles to inspect watch calls.
    fn build_test_context_with_repo_and_tracking(
        looms: Vec<Loom>,
        log_events: Vec<LoomEvent>,
    ) -> (
        AppContext,
        StdArc<Mutex<Vec<PathBuf>>>,
        StdArc<Mutex<Vec<PathBuf>>>,
    ) {
        let (event_sender, _event_rx) = mpsc::channel::<StrandEvent>(100);
        let _ = _event_rx;
        let (event_source, watch_calls, unwatch_calls) =
            TrackingEventSource::new();

        (
            AppContext {
                store: LoomStore::new(),
                loom_repo: Arc::new(ConfigurableLoomRepository {
                    scan_result: looms,
                }),
                loom_log_port: Arc::new(MockLoomLogPort { events: log_events }),
                tie_off_sink: Arc::new(MockTieOffSink),
                event_source: Arc::new(event_source),
                event_sender,
                agent_runner: Arc::new(MockAgentRunner),
                rig_config: RigAgentConfig::default_config(),
                loom_ids: Vec::new(),
                base_dir: PathBuf::from("./rig"),
            },
            watch_calls,
            unwatch_calls,
        )
    }

    // ── Phase 4 Tests ───────────────────────────────────────────────────

    /// `POST /looms/discover` with a rig containing new loom directories
    /// → 200 with list of discovered IDs → mock `EventSource` has `watch()`
    /// calls → looms appear in `GET /looms`.
    #[tokio::test]
    async fn discover_looms_scans_and_registers() {
        let loom_a = build_test_loom("loom-a", &["k1"]);
        let loom_b = build_test_loom("loom-b", &["k2", "k3"]);

        let (ctx, watch_calls, _unwatch_calls) =
            build_test_context_with_repo_and_tracking(
                vec![loom_a.clone(), loom_b.clone()],
                vec![],
            );
        let app = build_app(ctx);

        // POST /looms/discover
        let req = Request::builder()
            .method("POST")
            .uri("/looms/discover")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let summaries: Vec<LoomSummary> =
            serde_json::from_slice(&body).unwrap();

        // Both looms discovered
        assert_eq!(summaries.len(), 2);
        let ids: Vec<_> =
            summaries.iter().map(|s| s.id.0.as_str()).collect();
        assert!(ids.contains(&"loom-a"));
        assert!(ids.contains(&"loom-b"));

        // Watchers started for each loom (each has knots, so source dirs)
        let watches = watch_calls.lock().unwrap();
        // loom_a has 1 knot → 1 watch
        // loom_b has 2 knots → 2 watches
        assert_eq!(watches.len(), 3);

        // Verify looms are in the store via GET /looms
        let ctx = build_test_context();
        ctx.store.register(loom_a);
        ctx.store.register(loom_b);
        let app2 = build_app(ctx);
        let req = Request::builder()
            .uri("/looms")
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let all: Vec<LoomSummary> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(all.len(), 2);
    }

    /// `POST /looms/discover` when loom already registered → 200 with
    /// empty or partial list (no duplicates) → no duplicate `watch()`
    /// calls.
    #[tokio::test]
    async fn discover_looms_skips_existing() {
        let existing = build_test_loom("existing", &["k1"]);
        let new_loom = build_test_loom("new-discovered", &["k2"]);

        // Pre-register existing loom in the store
        let (ctx, watch_calls, _unwatch_calls) =
            build_test_context_with_repo_and_tracking(
                vec![existing.clone(), new_loom.clone()],
                vec![],
            );
        ctx.store.register(existing.clone());
        let app = build_app(ctx);

        // POST /looms/discover
        let req = Request::builder()
            .method("POST")
            .uri("/looms/discover")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let summaries: Vec<LoomSummary> =
            serde_json::from_slice(&body).unwrap();

        // Only the new loom returned (existing skipped)
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, LoomId("new-discovered".to_string()));

        // Only 1 watch call (for the new loom's knots), not 2
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);

        // Verify no duplicate in store
        let ctx = build_test_context();
        ctx.store.register(existing);
        ctx.store.register(new_loom);
        let app2 = build_app(ctx);
        let req = Request::builder()
            .uri("/looms")
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let all: Vec<LoomSummary> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(all.len(), 2);
    }
}
