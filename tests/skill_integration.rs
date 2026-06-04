//! Integration tests for Knot skills and their API endpoints.
//!
//! Tests that:
//! 1. Skill files exist and contain valid SKILL.md frontmatter
//! 2. Each skill references the correct OpenAPI spec URL
//! 3. The API endpoints documented in each skill are accessible and return
//!    expected shapes against a live Knot server

use std::path::Path;

use axum::{body::Body, http::Request};
use knot::adapters::inbound::AppContext;
use knot::application::ports::{
    AgentRunner, LoomLogPort,
    LoomRepository, ProcessingStatus, PortError, TieOffSink,
};
use knot::application::store::LoomStore;
use knot::domain::entities::{
    Knot, KnotId, Loom, LoomId, StrandPath, TieOff, TieOffPath,
};
use knot::domain::events::{LoomEvent, StrandEvent};
use knot::domain::value_objects::{AgentConfig, PromptTemplate, RigAgentConfig};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower::util::ServiceExt;

// ── Mock Ports ─────────────────────────────────────────────────────────────

struct MockLoomRepository;

impl LoomRepository for MockLoomRepository {
    fn scan(
        &self,
        _rig: &Path,
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

struct MockLoomLogPort;

impl LoomLogPort for MockLoomLogPort {
    fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
        Ok(())
    }
    fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
        Ok(())
    }
    fn read_all(
        &self,
        _loom_id: &LoomId,
    ) -> Result<Vec<LoomEvent>, PortError> {
        Ok(vec![])
    }
}

struct MockTieOffSink;

impl TieOffSink for MockTieOffSink {
    fn write(&self, _tie_off: TieOff) -> Result<(), PortError> {
        Ok(())
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

/// Build an AppContext with mock ports and a pre-registered loom.
fn build_context_with_loom() -> AppContext {
    let (event_sender, _event_rx) = mpsc::channel::<StrandEvent>(100);
    let _ = _event_rx;

    let ctx = AppContext {
        store: LoomStore::new(),
        loom_repo: Arc::new(MockLoomRepository),
        loom_log_port: Arc::new(MockLoomLogPort),
        tie_off_sink: Arc::new(MockTieOffSink),
        event_sender,
        agent_runner: Arc::new(MockAgentRunner),
        rig_config: RigAgentConfig::default_config(),
        loom_ids: Vec::new(),
    };

    // Register a test loom with a knot
    let test_loom = Loom {
        id: LoomId("test-loom".to_string()),
        source_dir: PathBuf::from("src/docs"),
        tie_off_dir: PathBuf::from("output/docs"),
        knots: vec![Knot {
            id: KnotId("review".to_string()),
            agent_config: AgentConfig {
                goal: "Review documents".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            },
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "Review this document.".to_string(),
            },
            source_dir: None,
            tie_off_dir: None,
        }],
    };
    ctx.store.register(test_loom);

    ctx
}

// ── Skill File Tests ───────────────────────────────────────────────────────

/// Verify all three skill directories exist with SKILL.md files.
#[test]
fn skill_files_exist() {
    let skills_dir = Path::new("skills");

    for skill_name in &["knot-init", "knots-and-looms", "knot-inspect"] {
        let skill_md = skills_dir.join(skill_name).join("SKILL.md");
        assert!(
            skill_md.exists(),
            "Skill file missing: {}",
            skill_md.display()
        );
    }
}

/// Each SKILL.md must have valid YAML frontmatter with required fields.
#[test]
fn skill_files_have_valid_frontmatter() {
    let skills_dir = Path::new("skills");

    for skill_name in &["knot-init", "knots-and-looms", "knot-inspect"] {
        let skill_md = skills_dir.join(skill_name).join("SKILL.md");
        let content = std::fs::read_to_string(&skill_md)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", skill_md.display(), e));

        // Must start with ---
        assert!(
            content.starts_with("---"),
            "{}: SKILL.md must start with --- (YAML frontmatter)",
            skill_name
        );

        // Must have a closing ---
        let first_line = content.lines().next().unwrap();
        assert_eq!(first_line, "---", "{}: First line must be ---", skill_name);

        // Extract frontmatter between first and second ---
        let lines: Vec<&str> = content.lines().collect();
        let mut closing_idx = None;
        for (i, line) in lines.iter().enumerate().skip(1) {
            if *line == "---" {
                closing_idx = Some(i);
                break;
            }
        }
        assert!(
            closing_idx.is_some(),
            "{}: SKILL.md must have closing --- for frontmatter",
            skill_name
        );

        let closing = closing_idx.unwrap();
        let frontmatter: String = lines[1..closing].join("\n");

        // Must contain required YAML fields
        assert!(
            frontmatter.contains("name:"),
            "{}: frontmatter must contain 'name:'",
            skill_name
        );
        assert!(
            frontmatter.contains("description:"),
            "{}: frontmatter must contain 'description:'",
            skill_name
        );
        assert!(
            frontmatter.contains("metadata:"),
            "{}: frontmatter must contain 'metadata:'",
            skill_name
        );
        assert!(
            frontmatter.contains("api_spec:"),
            "{}: frontmatter metadata must contain 'api_spec:'",
            skill_name
        );
    }
}

/// Each skill must reference the OpenAPI spec URL.
#[test]
fn skills_reference_openapi_spec_url() {
    let expected_url = "http://localhost:3000/swagger-ui/openapi.json";
    let skills_dir = Path::new("skills");

    for skill_name in &["knot-init", "knots-and-looms", "knot-inspect"] {
        let skill_md = skills_dir.join(skill_name).join("SKILL.md");
        let content = std::fs::read_to_string(&skill_md)
            .unwrap_or_else(|e| panic!("Cannot read {}: {}", skill_md.display(), e));

        assert!(
            content.contains(expected_url),
            "{}: skill must reference OpenAPI spec URL '{}'",
            skill_name,
            expected_url
        );
    }
}

/// knot-init skill must reference the endpoints it uses.
#[test]
fn knot_init_skill_references_endpoints() {
    let skill_md = Path::new("skills/knot-init/SKILL.md");
    let content = std::fs::read_to_string(skill_md).unwrap();

    // Must reference the endpoints used by the skill
    assert!(content.contains("/health"), "must reference /health");
    assert!(content.contains("/config/rig"), "must reference /config/rig");
    assert!(content.contains("/looms"), "must reference /looms");
}

/// knots-and-looms skill must reference CRUD endpoints.
#[test]
fn knots_and_looms_skill_references_endpoints() {
    let skill_md = Path::new("skills/knots-and-looms/SKILL.md");
    let content = std::fs::read_to_string(skill_md).unwrap();

    assert!(content.contains("POST /looms"), "must reference POST /looms");
    assert!(content.contains("DELETE /looms"), "must reference DELETE /looms");
    assert!(content.contains("/looms/{id}"), "must reference /looms/ID");
    assert!(
        content.contains("/looms/{id}/knots"),
        "must reference /looms/ID/knots"
    );
}

/// knot-inspect skill must reference all read endpoints.
#[test]
fn knot_inspect_skill_references_endpoints() {
    let skill_md = Path::new("skills/knot-inspect/SKILL.md");
    let content = std::fs::read_to_string(skill_md).unwrap();

    assert!(content.contains("/health"), "must reference /health");
    assert!(content.contains("/config/rig"), "must reference /config/rig");
    assert!(content.contains("/looms"), "must reference /looms");
    assert!(
        content.contains("/looms/{id}"),
        "must reference /looms/ID"
    );
    assert!(
        content.contains("/looms/{id}/activity"),
        "must reference /looms/ID/activity"
    );
    assert!(
        content.contains("/looms/{id}/knots/{knot_name}"),
        "must reference /looms/ID/knots/KNOT_NAME"
    );
}

// ── API Endpoint Tests (knot-init skill endpoints) ─────────────────────────

/// `GET /health` returns 200 — the first check knot-init performs.
#[tokio::test]
async fn knot_init_health_check() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

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
}

/// `GET /config/rig` returns 200 with valid RigAgentConfig — knot-init
/// verifies rig configuration exists.
#[tokio::test]
async fn knot_init_config_rig_check() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/config/rig")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let config: RigAgentConfig = serde_json::from_slice(&body).unwrap();
    assert_eq!(config.cli_path, "pi");
    assert!(config.cli_args.is_empty());
}

// ── API Endpoint Tests (knots-and-looms skill endpoints) ───────────────────

/// `POST /looms` creates a loom, then `GET /looms` lists it — the
/// knots-and-looms skill register + verify flow.
#[tokio::test]
async fn knots_and_looms_register_and_list() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    // POST /looms — register a new loom
    let body = serde_json::json!({
        "id": "new-loom",
        "source_dir": "src/new",
        "tie_off_dir": "output/new"
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

    // GET /looms — verify it appears
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
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let summaries: Vec<knot::application::usecases::LoomSummary> =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(summaries.len(), 2); // test-loom + new-loom

    let new = summaries.iter().find(|s| s.id.0 == "new-loom").unwrap();
    assert_eq!(new.source_dir, PathBuf::from("src/new"));
    assert_eq!(new.tie_off_dir, PathBuf::from("output/new"));
}

/// `GET /looms/{id}` returns loom details with knots — the knots-and-looms
/// skill verify step.
#[tokio::test]
async fn knots_and_looms_get_loom_details() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/looms/test-loom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let loom: Loom = serde_json::from_slice(&body).unwrap();
    assert_eq!(loom.id, LoomId("test-loom".to_string()));
    assert_eq!(loom.knots.len(), 1);
    assert_eq!(loom.knots[0].id, KnotId("review".to_string()));
}

/// `GET /looms/{id}/knots` returns knot names — knots-and-looms skill lists
/// knots in a loom.
#[tokio::test]
async fn knots_and_looms_list_knots() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/looms/test-loom/knots")
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
    assert_eq!(names, vec!["review"]);
}

/// `DELETE /looms/{id}` returns 204 — the knots-and-looms skill unregister
/// flow.
#[tokio::test]
async fn knots_and_looms_delete_loom() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/looms/test-loom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 204);
}

// ── API Endpoint Tests (knot-inspect skill endpoints) ──────────────────────

/// knot-inspect: full rig inspection flow.
/// GET /health → GET /config/rig → GET /looms.
#[tokio::test]
async fn knot_inspect_full_rig() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    // 1. GET /health
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

    // 2. GET /config/rig
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/config/rig")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let config: RigAgentConfig = serde_json::from_slice(&body).unwrap();
    assert_eq!(config.cli_path, "pi");

    // 3. GET /looms
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
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let summaries: Vec<knot::application::usecases::LoomSummary> =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, LoomId("test-loom".to_string()));
    assert_eq!(summaries[0].knot_count, 1);
}

/// knot-inspect: loom activity endpoint returns 200 with events.
#[tokio::test]
async fn knot_inspect_loom_activity() {
    // Build context with a loom log that has events
    let events = vec![
        LoomEvent::LoomStarted {
            loom_id: LoomId("test-loom".to_string()),
        },
        LoomEvent::KnotRegistered {
            loom_id: LoomId("test-loom".to_string()),
            knot_id: KnotId("review".to_string()),
        },
    ];

    let (event_sender, _event_rx) = mpsc::channel::<StrandEvent>(100);
    let _ = _event_rx;

    struct MockLoomLogPortWithEvents {
        events: Vec<LoomEvent>,
    }

    impl LoomLogPort for MockLoomLogPortWithEvents {
        fn open(&self, _loom_id: &LoomId) -> Result<(), PortError> {
            Ok(())
        }
        fn append(&self, _event: LoomEvent) -> Result<(), PortError> {
            Ok(())
        }
        fn read_all(
            &self,
            _loom_id: &LoomId,
        ) -> Result<Vec<LoomEvent>, PortError> {
            Ok(self.events.clone())
        }
    }

    let ctx = AppContext {
        store: LoomStore::new(),
        loom_repo: Arc::new(MockLoomRepository),
        loom_log_port: Arc::new(MockLoomLogPortWithEvents { events }),
        tie_off_sink: Arc::new(MockTieOffSink),
        event_sender,
        agent_runner: Arc::new(MockAgentRunner),
        rig_config: RigAgentConfig::default_config(),
        loom_ids: Vec::new(),
    };
    let app = knot::adapters::inbound::build_app(ctx);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/looms/test-loom/activity")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Returns 200 with events
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let log_events: Vec<LoomEvent> = serde_json::from_slice(&body).unwrap();
    assert_eq!(log_events.len(), 2);
}

/// knot-inspect: knot status endpoint. With no state, returns 404.
/// With state, returns 200 with KnotStatus.
#[tokio::test]
async fn knot_inspect_knot_status_not_found() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/looms/test-loom/knots/review")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // No knot state created, so returns 404
    assert_eq!(resp.status(), 404);
}

/// knot-inspect: knot status with state returns 200.
#[tokio::test]
async fn knot_inspect_knot_status_with_state() {
    let (event_sender, _event_rx) = mpsc::channel::<StrandEvent>(100);
    let _ = _event_rx;

    let ctx = AppContext {
        store: LoomStore::new(),
        loom_repo: Arc::new(MockLoomRepository),
        loom_log_port: Arc::new(MockLoomLogPort),
        tie_off_sink: Arc::new(MockTieOffSink),
        event_sender,
        agent_runner: Arc::new(MockAgentRunner),
        rig_config: RigAgentConfig::default_config(),
        loom_ids: Vec::new(),
    };
    ctx.store.register(Loom {
        id: LoomId("test-loom".to_string()),
        source_dir: PathBuf::from("src/docs"),
        tie_off_dir: PathBuf::from("output/docs"),
        knots: vec![Knot {
            id: KnotId("review".to_string()),
            agent_config: AgentConfig {
                goal: "Review".to_string(),
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
                tools: Vec::new(),
            },
            prompt_template: PromptTemplate {
                input_bundling: "full-file".to_string(),
                instructions: "Check it.".to_string(),
            },
            source_dir: None,
            tie_off_dir: None,
        }],
    });
    let app = knot::adapters::inbound::build_app(ctx);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/looms/test-loom/knots/review")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let status: knot::application::usecases::KnotStatus =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(status.knot_id, KnotId("review".to_string()));
    // KnotStatus now derived from loom-log, no .state field
    assert_eq!(status.status, ProcessingStatus::Completed);
    assert_eq!(status.last_error, None);
}

// ── Skill + API Contract Tests ─────────────────────────────────────────────

/// End-to-end: simulate the full knot-init workflow against the mock server.
///
/// 1. Health check passes
/// 2. Config rig returns valid config
/// 3. Looms list returns data
///
/// This validates the exact API contract that knot-init skill expects.
#[tokio::test]
async fn skill_contract_knot_init_workflow() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    // Step 1: Health check (knot-init step 1)
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

    // Step 2: Config rig (knot-init step 3)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/config/rig")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let config: RigAgentConfig = serde_json::from_slice(&body).unwrap();
    assert!(!config.cli_path.is_empty());

    // Step 3: List looms (knot-init step 4)
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
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let summaries: Vec<knot::application::usecases::LoomSummary> =
        serde_json::from_slice(&body).unwrap();
    // Should have at least the pre-registered loom
    assert!(!summaries.is_empty());
}

/// End-to-end: simulate the knots-and-looms register flow.
///
/// 1. POST /looms creates a loom
/// 2. GET /looms/{id} verifies it
/// 3. GET /looms/{id}/knots lists knots
/// 4. DELETE /looms/{id} removes it
///
/// This validates the exact API contract that knots-and-looms skill expects.
#[tokio::test]
async fn skill_contract_knots_and_looms_workflow() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    // Step 1: Register a loom
    let body = serde_json::json!({
        "id": "workflow-loom",
        "source_dir": "src/workflow",
        "tie_off_dir": "output/workflow"
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

    // Step 2: Verify with GET /looms/{id}
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/looms/workflow-loom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let loom: Loom = serde_json::from_slice(&body).unwrap();
    assert_eq!(loom.id, LoomId("workflow-loom".to_string()));

    // Step 3: List knots
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/looms/workflow-loom/knots")
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
    assert!(names.is_empty()); // No knots registered yet

    // Step 4: Delete loom
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/looms/workflow-loom")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
}

/// End-to-end: simulate the knot-inspect full inspection workflow.
///
/// 1. GET /health → 200
/// 2. GET /config/rig → 200 with config
/// 3. GET /looms → 200 with summaries
/// 4. GET /looms/{id} → 200 with loom details
/// 5. GET /looms/{id}/knots → 200 with knot names
///
/// This validates the exact API contract that knot-inspect skill expects.
#[tokio::test]
async fn skill_contract_knot_inspect_workflow() {
    let ctx = build_context_with_loom();
    let app = knot::adapters::inbound::build_app(ctx);

    // Step 1: Health check
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

    // Step 2: Config rig
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/config/rig")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let config: RigAgentConfig = serde_json::from_slice(&body).unwrap();
    assert_eq!(config.cli_path, "pi");

    // Step 3: List looms
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
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let summaries: Vec<knot::application::usecases::LoomSummary> =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(summaries.len(), 1);
    let loom_id = &summaries[0].id.0;

    // Step 4: Get loom details
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/looms/{}", loom_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let loom: Loom = serde_json::from_slice(&body).unwrap();
    assert_eq!(loom.id.0, *loom_id);

    // Step 5: List knots
    let resp = app
        .oneshot(
            Request::builder()
                .uri(&format!("/looms/{}/knots", loom_id))
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
    assert_eq!(names.len(), 1);
    assert_eq!(names[0], "review");
}
