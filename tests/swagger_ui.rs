//! Integration tests for Swagger UI and OpenAPI spec endpoints.
//!
//! Verifies that the auto-generated OpenAPI spec is served at the expected
//! URL and that the Swagger UI HTML is accessible.

use axum::{body::Body, http::Request};
use knot::adapters::inbound::AppContext;
use knot::application::ports::{
    AgentProfileRepository, AgentRunner, EventSource, LoomLogPort,
    LoomRepository, PortError, TieOffSink,
};
use knot::application::store::LoomStore;
use knot::domain::entities::{Loom, LoomId};
use knot::domain::events::StrandEvent;
use knot::domain::value_objects::RigAgentConfig;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tower::util::ServiceExt;

// ── Mock Ports ─────────────────────────────────────────────────────────────

struct MockLoomRepository;

impl LoomRepository for MockLoomRepository {
    fn scan(&self, _rig: &Path) -> Result<(Vec<Loom>, Vec<String>), PortError> {
        Ok((vec![], vec![]))
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

struct MockLoomLogPort;

impl LoomLogPort for MockLoomLogPort {
    fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
        Ok(())
    }
    fn append(&self, _event: knot::domain::events::LoomEvent) -> Result<(), PortError> {
        Ok(())
    }
    fn read_all(
        &self,
        _loom_id: &LoomId,
    ) -> Result<Vec<knot::domain::events::LoomEvent>, PortError> {
        Ok(vec![])
    }
}

struct MockTieOffSink;

impl TieOffSink for MockTieOffSink {
    fn write(
        &self,
        _tie_off: knot::domain::entities::TieOff,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn append(
        &self,
        _tie_off: knot::domain::entities::TieOff,
    ) -> Result<(), PortError> {
        Ok(())
    }

    fn read_content(
        &self,
        _path: &knot::domain::entities::TieOffPath,
    ) -> Result<String, PortError> {
        Ok(String::new())
    }
}

struct MockAgentRunner;

impl AgentRunner for MockAgentRunner {
    fn execute(
        &self,
        _ctx: knot::application::ports::ExecutionContext,
    ) -> Result<knot::application::ports::AgentOutput, PortError> {
        Ok(knot::application::ports::AgentOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        })
    }
}

struct MockProfileRepository;

impl AgentProfileRepository for MockProfileRepository {
    fn get(
        &self,
        _name: &str,
    ) -> Result<Option<knot::domain::value_objects::AgentProfile>, PortError> {
        Ok(None)
    }
    fn list(&self) -> Result<Vec<knot::domain::value_objects::AgentProfile>, PortError> {
        Ok(vec![])
    }
    fn save(
        &self,
        _profile: knot::domain::value_objects::AgentProfile,
    ) -> Result<(), PortError> {
        Ok(())
    }
    fn delete(&self, _name: &str) -> Result<(), PortError> {
        Ok(())
    }
}

struct MockEventSource;

impl EventSource for MockEventSource {
    fn watch(&self, _path: &Path) -> Result<(), PortError> {
        Ok(())
    }
    fn unwatch(&self, _path: &Path) -> Result<(), PortError> {
        Ok(())
    }
}

fn build_test_context() -> AppContext {
    let (event_sender, _event_rx) = mpsc::channel::<StrandEvent>(100);
    let _ = _event_rx;

    AppContext {
        store: LoomStore::new(),
        loom_repo: Arc::new(MockLoomRepository),
        loom_log_port: Arc::new(MockLoomLogPort),
        tie_off_sink: Arc::new(MockTieOffSink),
        event_source: Arc::new(MockEventSource),
        event_sender,
        agent_runner: Arc::new(MockAgentRunner),
        profile_repo: Arc::new(MockProfileRepository),
        rig_config: RigAgentConfig::default_config(),
        loom_ids: Vec::new(),
        base_dir: PathBuf::from("./rig"),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// `GET /swagger-ui/` returns 200 with HTML content.
#[tokio::test]
async fn swagger_ui_returns_200() {
    let ctx = build_test_context();
    let app = knot::adapters::inbound::build_app(ctx);

    let req = Request::builder()
        .uri("/swagger-ui/")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    // The Swagger UI should return HTML
    assert!(
        text.contains("<!DOCTYPE") || text.contains("<html") || text.contains("</html>"),
        "Expected HTML content, got: {}",
        &text[..text.len().min(200)]
    );
}

/// `GET /swagger-ui/openapi.json` returns 200 with valid OpenAPI JSON.
#[tokio::test]
async fn openapi_json_returns_valid_spec() {
    let ctx = build_test_context();
    let app = knot::adapters::inbound::build_app(ctx);

    let req = Request::builder()
        .uri("/swagger-ui/openapi.json")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify it is a valid OpenAPI 3.x document
    assert_eq!(spec.get("openapi").and_then(|v| v.as_str()), Some("3.1.0"));
    assert!(spec.get("info").unwrap().is_object());
    assert!(spec.get("paths").unwrap().is_object());
    assert!(spec.get("components").unwrap().is_object());

    // Verify key paths are present
    let paths = spec.get("paths").unwrap().as_object().unwrap();
    assert!(paths.contains_key("/health"));
    assert!(paths.contains_key("/agents/{dir}"));
    assert!(paths.contains_key("/config/rig"));
    assert!(paths.contains_key("/looms"));
    assert!(paths.contains_key("/looms/{id}"));
    assert!(paths.contains_key("/looms/{id}/activity"));
    assert!(paths.contains_key("/looms/{id}/knots"));
    assert!(paths.contains_key("/looms/{id}/knots/{name}"));

    // Verify profile paths are present (observability GET endpoints)
    assert!(paths.contains_key("/profiles"), "GET /profiles should exist");
    assert!(paths.contains_key("/profiles/{name}"), "GET /profiles/{{name}} should exist");

    // Verify control endpoints are NOT present (removed — file-first approach)
    for (path_key, path_obj) in paths.iter() {
        let path_str: &str = path_key;
        let path_map = path_obj.as_object().expect("path should be object");
        // No POST/PATCH/DELETE methods should exist on any path
        assert!(
            !path_map.contains_key("post"),
            "POST method should not exist on {path_str} — control endpoints removed"
        );
        assert!(
            !path_map.contains_key("patch"),
            "PATCH method should not exist on {path_str} — control endpoints removed"
        );
        assert!(
            !path_map.contains_key("delete"),
            "DELETE method should not exist on {path_str} — control endpoints removed"
        );
    }

    // Verify key schemas are present
    let schemas = spec
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object())
        .unwrap();
    assert!(schemas.contains_key("RigAgentConfig"));
    assert!(schemas.contains_key("Loom"));
    assert!(schemas.contains_key("LoomId"));
    assert!(schemas.contains_key("KnotId"));
    assert!(schemas.contains_key("LoomSummary"));
    assert!(schemas.contains_key("KnotStatus"));
    assert!(schemas.contains_key("LoomEvent"));
    assert!(schemas.contains_key("KnotState"));
    assert!(schemas.contains_key("ProcessingStatus"));
    assert!(schemas.contains_key("AgentConfig"));
    assert!(schemas.contains_key("PromptTemplate"));
    assert!(schemas.contains_key("ProfileResponse"));

    // Verify removed request types are NOT in schemas
    assert!(
        !schemas.contains_key("RegisterLoomRequest"),
        "RegisterLoomRequest should be removed from schemas"
    );
    assert!(
        !schemas.contains_key("KnotRequest"),
        "KnotRequest should be removed from schemas"
    );
    assert!(
        !schemas.contains_key("ProfileRequest"),
        "ProfileRequest should be removed from schemas"
    );

    // Verify ProfileResponse schema includes body field
    let profile_response = &schemas["ProfileResponse"];
    let props = profile_response
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("ProfileResponse should have properties");
    assert!(
        props.contains_key("body"),
        "ProfileResponse should have 'body' field for markdown content"
    );
}
