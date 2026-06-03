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
use tokio::sync::mpsc;

use crate::application::ports::{KnotStatePort, LoomLogPort, LoomRepository, TieOffSink};
use crate::application::store::LoomStore;
use crate::application::usecases::{
    DiscoverLooms, GetKnotStatus, GetLoom, GetLoomActivity,
    ListLooms, RegisterLoom, UnregisterLoom,
};
use crate::domain::entities::LoomId;
use crate::domain::events::StrandEvent;

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
    /// Knot state port.
    pub knot_state_port: Arc<dyn KnotStatePort>,
    /// Loom log port.
    pub loom_log_port: Arc<dyn LoomLogPort>,
    /// Tie-off sink port.
    pub tie_off_sink: Arc<dyn TieOffSink>,
    /// Debounce engine sender — feed raw strand events.
    pub event_sender: mpsc::Sender<StrandEvent>,
}

// ── Handler stubs ──────────────────────────────────────────────────────────

/// List all registered looms.
pub async fn list_looms(State(ctx): State<AppContext>) -> Response {
    let use_case = ListLooms::new(ctx.store.clone());
    let summaries = use_case.execute();
    (StatusCode::OK, Json(summaries)).into_response()
}

/// Get a loom by ID.
pub async fn get_loom(Path(id): Path<String>, State(ctx): State<AppContext>) -> Response {
    let loom_id = LoomId(id);
    let use_case = GetLoom::new(ctx.store.clone());
    match use_case.execute(&loom_id) {
        Ok(loom) => (StatusCode::OK, Json(loom)).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "loom not found").into_response(),
    }
}

/// Get activity log for a loom.
pub async fn get_loom_activity(
    Path(id): Path<String>,
    State(ctx): State<AppContext>,
) -> Response {
    let loom_id = LoomId(id);
    let use_case = GetLoomActivity::new(Arc::clone(&ctx.loom_log_port));
    let _ = use_case.execute(&loom_id);
    (StatusCode::NOT_IMPLEMENTED, "todo").into_response()
}

/// Get knots for a loom (derived from GetLoom).
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
pub async fn get_knot_status(
    Path((loom_id, knot_name)): Path<(String, String)>,
    State(ctx): State<AppContext>,
) -> Response {
    let _loom_id = loom_id;
    let _knot_name = knot_name;
    let _use_case = GetKnotStatus::new(Arc::clone(&ctx.knot_state_port));
    (StatusCode::NOT_IMPLEMENTED, "todo").into_response()
}

/// Register a new loom.
pub async fn register_loom(State(ctx): State<AppContext>) -> Response {
    let use_case = RegisterLoom::new(
        Arc::clone(&ctx.loom_log_port),
        Arc::clone(&ctx.knot_state_port),
        ctx.store.clone(),
    );
    let _ = use_case;
    (StatusCode::NOT_IMPLEMENTED, "todo").into_response()
}

/// Unregister a loom.
pub async fn unregister_loom(
    Path(id): Path<String>,
    State(ctx): State<AppContext>,
) -> Response {
    let loom_id = LoomId(id);
    let use_case = UnregisterLoom::new(
        Arc::clone(&ctx.loom_log_port),
        ctx.store.clone(),
    );
    let _ = use_case.execute(&loom_id);
    (StatusCode::NOT_IMPLEMENTED, "todo").into_response()
}

/// Discover looms in a workspace.
pub async fn discover_looms(State(ctx): State<AppContext>) -> Response {
    let use_case = DiscoverLooms::new(
        Arc::clone(&ctx.loom_repo),
        Arc::clone(&ctx.knot_state_port),
        Arc::clone(&ctx.loom_log_port),
        ctx.store.clone(),
    );
    let _ = use_case;
    (StatusCode::NOT_IMPLEMENTED, "todo").into_response()
}

// ── Router builder ─────────────────────────────────────────────────────────

/// Build the application router with loom routes and existing endpoints.
///
/// Accepts `AppContext` as shared state for all loom handlers.
pub fn build_app(ctx: AppContext) -> Router {
    Router::new()
        // Existing endpoints
        .route("/health", get(crate::health))
        .route("/agents/{dir}", get(crate::list_agents))
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
    use crate::application::ports::{KnotState, PortError};
    use crate::application::usecases::LoomSummary;
    use crate::domain::entities::{Knot, KnotId, Loom, LoomId, TieOff};
    use crate::domain::events::LoomEvent;
    use crate::domain::value_objects::{AgentConfig, PromptTemplate};
    use axum::{body::Body, http::Request};
    use std::path::PathBuf;
    use tower::util::ServiceExt;

    // ── Mock Port Implementations ──────────────────────────────────────

    struct MockLoomRepository;

    impl LoomRepository for MockLoomRepository {
        fn scan(
            &self,
            _workspace: &std::path::Path,
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

    struct MockKnotStatePort;

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

    struct MockLoomLogPort;

    impl LoomLogPort for MockLoomLogPort {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }

        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }

        fn read_all(&self, _loom_id: &LoomId) -> Result<Vec<LoomEvent>, PortError> {
            Ok(vec![])
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
        let (event_sender, _event_rx) = mpsc::channel::<StrandEvent>(100);
        let _ = _event_rx;

        AppContext {
            store: LoomStore::new(),
            loom_repo: Arc::new(MockLoomRepository),
            knot_state_port: Arc::new(MockKnotStatePort),
            loom_log_port: Arc::new(MockLoomLogPort),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_sender,
        }
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
                    },
                    prompt_template: PromptTemplate {
                        input_bundling: "full-file".to_string(),
                        instructions: "check it".to_string(),
                    },
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

    // Suppress unused mock warnings
    #[allow(dead_code)]
    fn _verify_agent_runner_mock() {
        struct MockAgentRunner;
        use crate::application::ports::{AgentOutput, AgentRunner, ExecutionContext};
        impl AgentRunner for MockAgentRunner {
            fn execute(
                &self,
                _ctx: ExecutionContext,
            ) -> Result<AgentOutput, PortError> {
                Ok(AgentOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 0,
                })
            }
        }
    }
}
