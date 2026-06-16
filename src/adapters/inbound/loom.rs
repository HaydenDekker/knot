//! Loom HTTP handlers and helpers.
//!
//! Thin handlers that extract parameters from HTTP requests and delegate
//! to application-layer use cases. They never touch ports or outbound
//! adapters directly.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use std::sync::Arc;

use crate::adapters::inbound::types::{
    AppContext, ProfileResponse,
};
use crate::application::usecases::{
    GetKnotStatus as GetKnotStatusUc, GetLoom as GetLoomUc,
    GetLoomActivity, ListLooms, LoomSummary,
};
use crate::domain::entities::{KnotId, Loom, LoomId};
use crate::domain::events::LoomEvent;
use crate::application::usecases::KnotStatus as KnotStatusDto;

// ── Loom Handlers ─────────────────────────────────────────────────────────

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
pub async fn get_loom(
    Path(id): Path<String>,
    State(ctx): State<AppContext>,
) -> Response {
    let loom_id = LoomId(id);
    let use_case = GetLoomUc::new(ctx.store.clone());
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
    let use_case = GetLoomUc::new(ctx.store.clone());
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
    path = "/looms/{id}/knots/{name}",
    params(
        ("id" = String, Path, description = "Loom identifier"),
        ("name" = String, Path, description = "Knot identifier"),
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
    let use_case =
        GetKnotStatusUc::new(ctx.store.clone(), Arc::clone(&ctx.loom_log_port));
    let result = tokio::task::spawn_blocking(move || {
        use_case.execute(&loom_id_val, &knot_id)
    })
    .await;
    match result {
        Ok(Ok(status)) => (StatusCode::OK, Json(status)).into_response(),
        Ok(Err(_)) => {
            (StatusCode::NOT_FOUND, "knot not found").into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "task execution failed",
        )
            .into_response(),
    }
}

// ── Profile Handlers ───────────────────────────────────────────────────

/// List all registered agent profiles.
#[utoipa::path(
    get,
    path = "/profiles",
    responses(
        (status = 200, body = Vec<ProfileResponse>, description = "List of profiles"),
    ),
)]
pub async fn list_profiles(State(ctx): State<AppContext>) -> Response {
    match ctx.profile_repo.list() {
        Ok(profiles) => {
            let responses: Vec<ProfileResponse> = profiles
                .into_iter()
                .map(|p| ProfileResponse {
                    name: p.name,
                    provider: p.provider,
                    model: p.model,
                    tools: p.tools,
                    profile_prompt: p.profile_prompt,
                    body: p.body,
                })
                .collect();
            (StatusCode::OK, Json(responses)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Get a single agent profile by name.
#[utoipa::path(
    get,
    path = "/profiles/{name}",
    params(
        ("name" = String, Path, description = "Profile name"),
    ),
    responses(
        (status = 200, body = ProfileResponse, description = "Profile details"),
        (status = 404, description = "Profile not found"),
    ),
)]
pub async fn get_profile(
    Path(name): Path<String>,
    State(ctx): State<AppContext>,
) -> Response {
    match ctx.profile_repo.get(&name) {
        Ok(Some(profile)) => {
            let response = ProfileResponse {
                name: profile.name,
                provider: profile.provider,
                model: profile.model,
                tools: profile.tools,
                profile_prompt: profile.profile_prompt,
                body: profile.body,
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "profile not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::inbound::router::build_app;
    use crate::adapters::inbound::types::AppContext;
    use crate::application::ports::{
        AgentProfileRepository, AgentRunner, EventSource, LoomLogPort,
        LoomRepository, PortError, RigLogPort, TieOffSink,
    };
    use crate::application::store::LoomStore;
    use crate::application::usecases::LoomSummary;
    use crate::domain::entities::{
        Knot, KnotId, Loom, LoomId, StrandPath, TieOff, TieOffPath,
    };
    use crate::domain::events::{LoomEvent, StrandEvent};
    use crate::domain::value_objects::{
        AgentProfile, PromptTemplate, RigAgentConfig,
    };
    use axum::{body::Body, http::Request};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Arc as StdArc, Mutex};
    use tokio::sync::mpsc;
    use tower::util::ServiceExt;

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
        #[allow(clippy::type_complexity)]
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

    struct MockLoomRepository {
        looms: Arc<Mutex<HashMap<LoomId, Loom>>>,
    }

    impl Default for MockLoomRepository {
        fn default() -> Self {
            Self {
                looms: Arc::new(Mutex::new(HashMap::new())),
            }
        }
    }

    impl MockLoomRepository {
        fn new() -> Self {
            Self::default()
        }
    }

    impl LoomRepository for MockLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<(Vec<Loom>, Vec<String>), PortError> {
            Ok((vec![], vec![]))
        }

        fn scan_knot_files(
            &self,
            _loom_dir: &std::path::Path,
        ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
            Ok((vec![], vec![]))
        }

        fn get(&self, id: &LoomId) -> Result<Option<Loom>, PortError> {
            Ok(self.looms.lock().unwrap().get(id).cloned())
        }

        fn list(&self) -> Result<Vec<Loom>, PortError> {
            Ok(self.looms.lock().unwrap().values().cloned().collect())
        }

        fn save(&self, loom: Loom) -> Result<(), PortError> {
            self.looms
                .lock()
                .unwrap()
                .insert(loom.id.clone(), loom);
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

    /// Mock `RigLogPort` that does nothing.
    struct MockRigLogPort;

    impl RigLogPort for MockRigLogPort {
        fn append(
            &self,
            _event: crate::domain::events::RigLogEvent,
        ) -> Result<(), PortError> {
            Ok(())
        }
        fn read_all(
            &self,
        ) -> Result<Vec<crate::domain::events::RigLogEvent>, PortError> {
            Ok(vec![])
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

    /// In-memory mock of `AgentProfileRepository`.
    struct MockProfileRepository {
        profiles: Arc<Mutex<HashMap<String, AgentProfile>>>,
    }

    impl MockProfileRepository {
        fn new() -> Self {
            Self {
                profiles: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn add(&self, profile: AgentProfile) {
            self.profiles
                .lock()
                .unwrap()
                .insert(profile.name.clone(), profile);
        }
    }

    impl Default for MockProfileRepository {
        fn default() -> Self {
            Self::new()
        }
    }

    impl AgentProfileRepository for MockProfileRepository {
        fn get(
            &self,
            name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .get(name)
                .cloned())
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(self
                .profiles
                .lock()
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
                .lock()
                .unwrap()
                .insert(profile.name.clone(), profile);
            Ok(())
        }

        fn delete(&self, name: &str) -> Result<(), PortError> {
            let mut map = self.profiles.lock().unwrap();
            if map.remove(name).is_none() {
                return Err(PortError::ProfileNotFound(name.to_string()));
            }
            Ok(())
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
            loom_repo: Arc::new(MockLoomRepository::new()),
            loom_log_port: Arc::new(MockLoomLogPort { events: log_events }),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_source: Arc::new(event_source),
            event_sender,
            agent_runner: Arc::new(MockAgentRunner),
            profile_repo: Arc::new(MockProfileRepository::default()),
            rig_log_port: Arc::new(MockRigLogPort),
            rig_config: RigAgentConfig::default_config(),
            loom_ids: Vec::new(),
            rig_dir: PathBuf::from("./rig"),
        }
    }

    /// Build an `AppContext` with a tracking mock `EventSource`, returning
    /// the context plus handles to inspect watch/unwatch call history.
    #[allow(clippy::type_complexity)]
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
                loom_repo: Arc::new(MockLoomRepository::new()),
                loom_log_port: Arc::new(MockLoomLogPort { events: log_events }),
                tie_off_sink: Arc::new(MockTieOffSink),
                event_source: Arc::new(event_source),
                event_sender,
                agent_runner: Arc::new(MockAgentRunner),
                profile_repo: Arc::new(MockProfileRepository::default()),
                rig_log_port: Arc::new(MockRigLogPort),
                rig_config: RigAgentConfig::default_config(),
                loom_ids: Vec::new(),
                rig_dir: PathBuf::from("./rig"),
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
                    agent_profile_ref: "fast".to_string(),
                    prompt_template: PromptTemplate {
                        input_bundling: "full-file".to_string(),
                        instructions: "check it".to_string(),
                    },
                    strand_dir: PathBuf::from("strands"),
                    git_versioned: true,
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
                timestamp: "2026-06-10T12:00:00Z".to_string(),
            },
            LoomEvent::KnotRegistered {
                loom_id: LoomId("active-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
                timestamp: "2026-06-10T12:00:01Z".to_string(),
            },
            LoomEvent::StrandProcessed {
                loom_id: LoomId("active-loom".to_string()),
                strand_path: StrandPath(PathBuf::from("src/file.md")),
                error: None,
                timestamp: "2026-06-10T12:00:02Z".to_string(),
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
        let returned: Vec<LoomEvent> = serde_json::from_slice(&body).unwrap();
        assert_eq!(returned.len(), 3);
        match &returned[0] {
            LoomEvent::LoomStarted { loom_id, .. } => {
                assert_eq!(*loom_id, LoomId("active-loom".to_string()));
            }
            _ => panic!("Expected LoomStarted"),
        }
        match &returned[1] {
            LoomEvent::KnotRegistered { loom_id, knot_id, .. } => {
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
                timestamp: "2026-06-10T12:00:00Z".to_string(),
            },
            LoomEvent::KnotCompleted {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
                strand_path: StrandPath(PathBuf::from("src/input.md")),
                tie_off_path: TieOffPath(PathBuf::from("out/output.md")),
                timestamp: "2026-06-10T12:00:01Z".to_string(),
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
            Some(StrandPath(PathBuf::from("src/input.md"))),
        );
        assert_eq!(
            status.last_tie_off_path,
            Some(TieOffPath(PathBuf::from("out/output.md"))),
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
                timestamp: "2026-06-10T12:00:00Z".to_string(),
            },
            LoomEvent::KnotProcessing {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
                strand_path: StrandPath(PathBuf::from("src/current.md")),
                timestamp: "2026-06-10T12:00:01Z".to_string(),
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
            Some(StrandPath(PathBuf::from("src/current.md"))),
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
                timestamp: "2026-06-10T12:00:00Z".to_string(),
            },
            LoomEvent::KnotFailed {
                loom_id: LoomId("my-loom".to_string()),
                knot_id: KnotId("k1".to_string()),
                strand_path: StrandPath(PathBuf::from("src/bad.md")),
                error: "agent timeout".to_string(),
                timestamp: "2026-06-10T12:00:01Z".to_string(),
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
                timestamp: "2026-06-10T12:00:00Z".to_string(),
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

    // ── Route Integration Tests ─────────────────────────────────────

    /// Existing routes (`/health`, `/agents/{dir}`) still work alongside
    /// loom routes on the same `Router` instance. No route conflicts.
    #[tokio::test]
    async fn existing_routes_preserved() {
        let ctx = build_test_context();
        let app = build_app(ctx);

        // GET /health -> 200 with body "ok"
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

        // GET /agents/{dir} -> 200 with directory listing
        let tmp = tempfile::tempdir().unwrap();
        let dir_path = tmp.path().to_string_lossy().to_string();
        std::fs::write(tmp.path().join("agent-a"), "{}").unwrap();
        std::fs::write(tmp.path().join("agent-b"), "{}").unwrap();

        let encoded = dir_path.replace('/', "%2F");
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/agents/{encoded}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let names: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"agent-a".to_string()));
        assert!(names.contains(&"agent-b".to_string()));

        // GET /agents/{dir} -> 404 for nonexistent directory
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

    // ── Infrastructure Tests ──────────────────────────────────────────

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

    // ── Profile Handler Tests ─────────────────────────────────────────────

    /// Build an `AppContext` with a populated `MockProfileRepository`.
    fn build_test_context_with_profiles(
        profile_names: &[&str],
    ) -> AppContext {
        let repo = MockProfileRepository::new();
        for &name in profile_names {
            let profile = AgentProfile::new(
                name.to_string(),
                "openai".to_string(),
                "gpt-4o".to_string(),
                format!("You are {name}.").to_string(),
            )
            .unwrap();
            repo.add(profile);
        }

        AppContext {
            store: LoomStore::new(),
            loom_repo: Arc::new(MockLoomRepository::new()),
            loom_log_port: Arc::new(MockLoomLogPort { events: vec![] }),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_source: Arc::new(TrackingEventSource::new().0),
            event_sender: {
                let (tx, _rx) = mpsc::channel::<StrandEvent>(100);
                tx
            },
            agent_runner: Arc::new(MockAgentRunner),
            profile_repo: Arc::new(repo),
            rig_log_port: Arc::new(MockRigLogPort),
            rig_config: RigAgentConfig::default_config(),
            loom_ids: Vec::new(),
            rig_dir: PathBuf::from("./rig"),
        }
    }

    /// `GET /profiles` returns 200 with JSON array of profile responses.
    #[tokio::test]
    async fn get_profiles_returns_json() {
        let ctx = build_test_context_with_profiles(&["fast", "detailed"]);
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/profiles")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let profiles: Vec<ProfileResponse> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(profiles.len(), 2);

        let names: Vec<_> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"fast"));
        assert!(names.contains(&"detailed"));
    }

    /// `GET /profiles` with no profiles returns 200 with empty array `[]`.
    #[tokio::test]
    async fn get_profiles_empty() {
        let ctx = build_test_context_with_profiles(&[]);
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/profiles")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let profiles: Vec<ProfileResponse> =
            serde_json::from_slice(&body).unwrap();
        assert!(profiles.is_empty());
    }

    /// `GET /profiles/:name` for a known profile returns 200 with details.
    #[tokio::test]
    async fn get_profile_by_name() {
        let ctx = build_test_context_with_profiles(&["fast"]);
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/profiles/fast")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let profile: ProfileResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(profile.name, "fast");
        assert_eq!(profile.provider, "openai");
        assert_eq!(profile.model, "gpt-4o");
        assert!(profile.profile_prompt.contains("fast"));
    }

    /// `GET /profiles/:name` for unknown profile returns 404.
    #[tokio::test]
    async fn get_profile_not_found() {
        let ctx = build_test_context_with_profiles(&["fast"]);
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/profiles/unknown")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 404);
    }

    /// `GET /profiles/:name` returns the body field when the profile
    /// has markdown body content (documentation after frontmatter).
    #[tokio::test]
    async fn get_profile_with_body() {
        let repo = MockProfileRepository::new();
        let profile = AgentProfile::new(
            "documented".to_string(),
            "anthropic".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            "You are a code reviewer.".to_string(),
        )
        .unwrap()
        .with_body(Some("This profile is used for code review tasks.".to_string()));
        repo.add(profile);

        let ctx = AppContext {
            store: LoomStore::new(),
            loom_repo: Arc::new(MockLoomRepository::new()),
            loom_log_port: Arc::new(MockLoomLogPort { events: vec![] }),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_source: Arc::new(TrackingEventSource::new().0),
            event_sender: {
                let (tx, _rx) = mpsc::channel::<StrandEvent>(100);
                tx
            },
            agent_runner: Arc::new(MockAgentRunner),
            profile_repo: Arc::new(repo),
            rig_log_port: Arc::new(MockRigLogPort),
            rig_config: RigAgentConfig::default_config(),
            loom_ids: Vec::new(),
            rig_dir: PathBuf::from("./rig"),
        };
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/profiles/documented")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let profile: ProfileResponse =
            serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(profile.name, "documented");
        assert_eq!(profile.provider, "anthropic");
        assert_eq!(profile.body, Some("This profile is used for code review tasks.".to_string()));
    }

    /// `GET /profiles/:name` omits body field when profile has no
    /// markdown body (skip_serializing_if = None).
    #[tokio::test]
    async fn get_profile_without_body_omits_field() {
        let ctx = build_test_context_with_profiles(&["fast"]);
        let app = build_app(ctx);

        let req = Request::builder()
            .uri("/profiles/fast")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&body_bytes);
        // body field should be omitted from JSON when None
        assert!(
            !text.contains("\"body\""),
            "body field should be omitted when None, got: {text}"
        );
    }

    // ── Configurable MockLoomRepository for scan tests ────────────────

    /// Mock `LoomRepository` that returns configurable looms from `scan()`.
    struct ScanableMockLoomRepository {
        scan_looms: StdArc<Mutex<Vec<Loom>>>,
    }

    impl ScanableMockLoomRepository {
        fn new() -> Self {
            Self {
                scan_looms: StdArc::new(Mutex::new(vec![])),
            }
        }

        fn set_scan_results(&self, looms: Vec<Loom>) {
            self.scan_looms.lock().unwrap().clear();
            self.scan_looms.lock().unwrap().extend(looms);
        }
    }

    impl LoomRepository for ScanableMockLoomRepository {
        fn scan(
            &self,
            _rig: &std::path::Path,
        ) -> Result<(Vec<Loom>, Vec<String>), PortError> {
            Ok((
                self.scan_looms.lock().unwrap().clone(),
                vec![],
            ))
        }

        fn scan_knot_files(
            &self,
            _loom_dir: &std::path::Path,
        ) -> Result<(Vec<Knot>, Vec<String>), PortError> {
            Ok((vec![], vec![]))
        }

        fn get(&self, id: &LoomId) -> Result<Option<Loom>, PortError> {
            Ok(self
                .scan_looms
                .lock()
                .unwrap()
                .iter()
                .find(|l| l.id == *id)
                .cloned())
        }

        fn list(&self) -> Result<Vec<Loom>, PortError> {
            Ok(self.scan_looms.lock().unwrap().clone())
        }

        fn save(&self, loom: Loom) -> Result<(), PortError> {
            self.scan_looms
                .lock()
                .unwrap()
                .push(loom);
            Ok(())
        }
    }

    /// Build an `AppContext` with a `ScanableMockLoomRepository` so that
    /// `POST /config/reload` can discover looms from `scan()`.
    #[allow(clippy::type_complexity)]
    fn build_test_context_with_scanable_repo() -> (
        AppContext,
        StdArc<Mutex<Vec<Loom>>>,
    ) {
        let (event_sender, _event_rx) = mpsc::channel::<StrandEvent>(100);
        let _ = _event_rx;
        let (event_source, _watch, _unwatch) = TrackingEventSource::new();
        let _ = (_watch, _unwatch);
        let repo = ScanableMockLoomRepository::new();
        let scan_looms = repo.scan_looms.clone();

        (
            AppContext {
                store: LoomStore::new(),
                loom_repo: Arc::new(repo),
                loom_log_port: Arc::new(MockLoomLogPort { events: vec![] }),
                tie_off_sink: Arc::new(MockTieOffSink),
                event_source: Arc::new(event_source),
                event_sender,
                agent_runner: Arc::new(MockAgentRunner),
                profile_repo: Arc::new(MockProfileRepository::default()),
                rig_log_port: Arc::new(MockRigLogPort),
                rig_config: RigAgentConfig::default_config(),
                loom_ids: Vec::new(),
                rig_dir: PathBuf::from("./rig"),
            },
            scan_looms,
        )
    }

    // ── POST /config/reload Tests ──────────────────────────────────────────

    /// `POST /config/reload` discovers new looms and returns their summaries.
    #[tokio::test]
    async fn post_config_reload_success() {
        let (ctx, scan_looms) = build_test_context_with_scanable_repo();

        // Seed the mock repo with a loom that is NOT in the store
        scan_looms.lock().unwrap().clear();
        scan_looms.lock().unwrap().push(build_test_loom(
            "new-discovered-loom",
            &["k1", "k2"],
        ));

        let app = build_app(ctx);

        let req = Request::builder()
            .method("POST")
            .uri("/config/reload")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), 200);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let summaries: Vec<LoomSummary> =
            serde_json::from_slice(&body).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(
            summaries[0].id,
            LoomId("new-discovered-loom".to_string())
        );
        assert_eq!(summaries[0].knot_count, 2);
    }

    /// `POST /config/reload` with no new looms returns 200 with empty array.
    #[tokio::test]
    async fn post_config_reload_no_new_looms() {
        let (ctx, scan_looms) = build_test_context_with_scanable_repo();

        // No looms in the mock repo's scan results
        scan_looms.lock().unwrap().clear();

        let app = build_app(ctx);

        let req = Request::builder()
            .method("POST")
            .uri("/config/reload")
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
}
