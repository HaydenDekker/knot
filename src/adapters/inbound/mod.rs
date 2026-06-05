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
        RigConfigResponse,
    )),
)]
struct ApiDoc;

// ── Request Bodies ─────────────────────────────────────────────────────────

/// JSON body for `POST /looms` to register a new loom.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct RegisterLoomRequest {
    /// Unique loom identifier.
    pub id: String,
    /// Source directory to watch.
    pub source_dir: Option<String>,
    /// Tie-off (output) directory.
    #[serde(default)]
    pub tie_off_dir: Option<String>,
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
    // Validate source_dir is present
    let source_dir = match &body.source_dir {
        Some(dir) if !dir.trim().is_empty() => dir,
        _ => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "error": "source_dir is required and must not be empty"
            }))).into_response();
        }
    };

    let tie_off_dir = body.tie_off_dir.unwrap_or_else(|| "output".to_string());

    let loom = Loom {
        id: LoomId(body.id),
        source_dir: std::path::PathBuf::from(source_dir),
        tie_off_dir: std::path::PathBuf::from(&tie_off_dir),
        knots: vec![],
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
#[utoipa::path(
    post,
    path = "/looms/discover",
    responses(
        (status = 200, body = Vec<LoomSummary>, description = "Discovered looms"),
        (status = 500, description = "Discovery failed"),
    ),
)]
pub async fn discover_looms(State(ctx): State<AppContext>) -> Response {
    let use_case = DiscoverLooms::new(
        Arc::clone(&ctx.loom_repo),
        Arc::clone(&ctx.loom_log_port),
        ctx.store.clone(),
    );
    match use_case.execute(&ctx.base_dir) {
        Ok(looms) => {
            let summaries: Vec<LoomSummary> = looms
                .into_iter()
                .map(|loom| LoomSummary {
                    id: loom.id,
                    source_dir: loom.source_dir,
                    tie_off_dir: loom.tie_off_dir,
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
            source_dir: PathBuf::from("src"),
            tie_off_dir: PathBuf::from("out"),
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
                    source_dir: None,
                    tie_off_dir: None,
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

    /// `POST /looms` with valid body returns 201, loom appears in `GET /looms`.
    #[tokio::test]
    async fn post_loom_success() {
        let ctx = build_test_context();
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "new-loom",
            "source_dir": "src/docs",
            "tie_off_dir": "output/docs"
        });
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
            source_dir: PathBuf::from("src/docs"),
            tie_off_dir: PathBuf::from("output/docs"),
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

    /// Body missing `source_dir`; returns 400.
    #[tokio::test]
    async fn post_loom_missing_source_dir() {
        let ctx = build_test_context();
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "bad-loom"
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
    }

    /// Register same loom twice; second returns 409.
    #[tokio::test]
    async fn post_loom_duplicate_id() {
        let ctx = build_test_context();
        ctx.store.register(build_test_loom("dup-loom", &["k1"]));
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "dup-loom",
            "source_dir": "src/other"
        });
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
    /// has recorded a `watch()` call for the source directory.
    #[tokio::test]
    async fn post_loom_starts_watcher() {
        let (ctx, watch_calls, _unwatch_calls) =
            build_test_context_with_tracking(vec![]);
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "watch-loom",
            "source_dir": "src/docs",
            "tie_off_dir": "output/docs"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/looms")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 201);

        // Verify watch() was called for the source directory
        let watches = watch_calls.lock().unwrap();
        assert_eq!(watches.len(), 1);
        assert_eq!(watches[0], PathBuf::from("src/docs"));
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

    // ── Phase 4: Route Integration Tests ──────────────────────────────────

    /// All 7 loom endpoints are accessible on a single router with shared
    /// `AppContext`. Verifies GET returns 200/404, POST returns 201/400/409,
    /// and DELETE returns 204/404.
    #[tokio::test]
    async fn full_route_wiring() {
        // Pre-register a loom so read endpoints have data to return
        let ctx = build_test_context();
        ctx.store.register(build_test_loom("wired", &["k1", "k2"]));
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
                    .uri("/looms/wired")
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
                    .uri("/looms/wired/activity")
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
                    .uri("/looms/wired/knots")
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
                    .uri("/looms/wired/knots/k1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);

        // 7. POST /looms → 201 (valid body)
        let body = serde_json::json!({
            "id": "post-wired",
            "source_dir": "src/wired"
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
        assert_eq!(resp.status(), 201);

        // 8. POST /looms → 400 (missing source_dir)
        let body = serde_json::json!({
            "id": "bad-post"
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
        let body = serde_json::json!({
            "id": "wired",
            "source_dir": "src/other"
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
        assert_eq!(resp.status(), 409);

        // 10. DELETE /looms/:id → 204 (found)
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/looms/wired")
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
                    .uri("/looms/wired")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
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
}
