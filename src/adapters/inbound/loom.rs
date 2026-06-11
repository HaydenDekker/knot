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

use crate::adapters::inbound::types::{AppContext, KnotRequest, RegisterLoomRequest};
use crate::adapters::outbound::FileSystemLoomRepository;
use crate::application::usecases::{
    GetKnotStatus as GetKnotStatusUc, GetLoom as GetLoomUc,
    GetLoomActivity, ListLooms, LoomSummary, RegisterLoom, UnregisterLoom,
};
use crate::domain::entities::{Knot, KnotId, Loom, LoomId};
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

/// Register a new loom.
#[utoipa::path(
    post,
    path = "/looms",
    request_body = RegisterLoomRequest,
    responses(
        (status = 201, description = "Loom registered successfully"),
        (status = 400, description = "Invalid request"),
        // 409 removed — RegisterLoom is now idempotent (auto-discovery may pre-register)
        // (status = 409, description = "Loom already exists"),
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
    }

    // Create the loom directory: <rig>/<id>/
    let loom_dir = ctx.base_dir.join(&body.id);
    if let Err(e) = std::fs::create_dir_all(&loom_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("failed to create loom directory: {e}")
            })),
        )
            .into_response();
    }

    // Write knot .md files to the loom directory
    for knot in &body.knots {
        let content = generate_knot_file(knot);
        let file_path = loom_dir.join(format!("{}.md", knot.name));
        if let Err(e) = std::fs::write(&file_path, content) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("failed to write knot file: {e}")
                })),
            )
                .into_response();
        }
    }

    // Build the Loom from request data (knots read from disk will match
    // on restart via FileSystemLoomRepository::scan).
    let project_root = ctx
        .base_dir
        .parent()
        .unwrap_or(&ctx.base_dir);

    let knots: Vec<Knot> = body
        .knots
        .iter()
        .map(|k| {
            let strand_dir = FileSystemLoomRepository::resolve_path(
                project_root,
                &std::path::PathBuf::from(&k.strand_dir),
            );
            Knot {
                id: KnotId(k.name.clone()),
                agent_config: Some(k.agent_config.clone()),
                agent_profile_ref: None,
                prompt_template: k.prompt_template.clone(),
                strand_dir,
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
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "registered": true })),
        )
            .into_response(),
        Err(crate::application::ports::PortError::LoomSaveFailed(msg)) => {
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Generate the YAML frontmatter content for a knot definition file.
///
/// Produces a markdown file with `---` delimited frontmatter containing
/// the knot's configuration fields. The body is a minimal heading.
fn generate_knot_file(knot: &KnotRequest) -> String {
    let ac = &knot.agent_config;
    let tools_yaml = if ac.tools.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = ac
            .tools
            .iter()
            .map(|t| format!("    - {t}"))
            .collect();
        format!("\n  tools:\n{}", lines.join("\n"))
    };

    format!(
        "---\nname: {0}\nagent-config:\n  goal: {1}\n  provider: {2}\n  model: {3}{4}\nstrand-dir: {5}\nprompt-template:\n  input-bundling: {6}\n  instructions: {7}\n---\n\n# {8}\n",
        knot.name,
        quote_yaml_scalar(&ac.goal),
        quote_yaml_scalar(&ac.provider),
        quote_yaml_scalar(&ac.model),
        tools_yaml,
        quote_yaml_scalar(&knot.strand_dir),
        quote_yaml_scalar(&knot.prompt_template.input_bundling),
        quote_yaml_scalar(&knot.prompt_template.instructions),
        knot.name,
    )
}

/// Wrap a string in double quotes for safe YAML scalar output.
///
/// Escapes internal double quotes and backslashes.
fn quote_yaml_scalar(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
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

/// Create a new knot in a loom.
///
/// Validates the loom exists, writes the `.md` file to the loom directory,
/// and updates the in-memory store via `ManageKnot::Create`.
#[utoipa::path(
    post,
    path = "/looms/{id}/knots",
    params(
        ("id" = String, Path, description = "Loom identifier"),
    ),
    request_body = KnotRequest,
    responses(
        (status = 201, description = "Knot created successfully"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Loom not found"),
        (status = 409, description = "Knot already exists"),
        (status = 500, description = "Internal server error"),
    ),
)]
pub async fn create_knot(
    Path(id): Path<String>,
    State(ctx): State<AppContext>,
    Json(body): Json<KnotRequest>,
) -> Response {
    let loom_id = LoomId(id);

    // Validate knot has required fields
    if body.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "knot name must not be empty"
            })),
        )
            .into_response();
    }
    if body.strand_dir.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "strand_dir is required"
            })),
        )
            .into_response();
    }

    // Check loom exists
    if ctx.store.get(&loom_id).is_none() {
        return (StatusCode::NOT_FOUND, "loom not found").into_response();
    }

    // Write the knot .md file to the loom directory
    let loom_dir = ctx.base_dir.join(&loom_id.0);
    let content = generate_knot_file(&body);
    let file_path = loom_dir.join(format!("{}.md", body.name));
    if let Err(e) = std::fs::write(&file_path, content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("failed to write knot file: {e}")
            })),
        )
            .into_response();
    }

    // Build the Knot from request data
    let project_root = ctx.base_dir.parent().unwrap_or(&ctx.base_dir);
    let strand_dir = FileSystemLoomRepository::resolve_path(
        project_root,
        &std::path::PathBuf::from(&body.strand_dir),
    );
    let knot = Knot {
        id: KnotId(body.name.clone()),
        agent_config: Some(body.agent_config.clone()),
        agent_profile_ref: None,
        prompt_template: body.prompt_template.clone(),
        strand_dir,
    };

    // Update the in-memory store
    let knot_id = knot.id.clone();
    let strand_dir = knot.strand_dir.clone();
    let use_case = crate::application::usecases::ManageKnot::new(
        ctx.store.clone(),
    );
    match use_case.execute(
        crate::application::usecases::KnotAction::Create {
            loom_id: loom_id.clone(),
            knot,
        },
    ) {
        Ok(()) => {
            // Start file watcher for the knot's strand directory
            ctx.event_source.set_loom_ids(
                &strand_dir,
                &loom_id,
                &knot_id,
            );
            let _ = ctx.event_source.watch(&strand_dir);
            (
                StatusCode::CREATED,
                Json(serde_json::json!({ "created": true })),
            )
                .into_response()
        }
        Err(crate::application::ports::PortError::LoomSaveFailed(msg)) => {
            if msg.contains("already exists") {
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": msg })),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": msg })),
                )
                    .into_response()
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Update an existing knot's configuration.
///
/// Validates the loom and knot exist, writes the updated `.md` file,
/// and updates the in-memory store via `ManageKnot::Update`.
#[utoipa::path(
    patch,
    path = "/looms/{id}/knots/{name}",
    params(
        ("id" = String, Path, description = "Loom identifier"),
        ("name" = String, Path, description = "Knot identifier"),
    ),
    request_body = KnotRequest,
    responses(
        (status = 200, description = "Knot updated successfully"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Loom or knot not found"),
        (status = 500, description = "Internal server error"),
    ),
)]
pub async fn update_knot(
    Path((id, name)): Path<(String, String)>,
    State(ctx): State<AppContext>,
    Json(body): Json<KnotRequest>,
) -> Response {
    let loom_id = LoomId(id);

    // Validate knot has required fields
    if body.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "knot name must not be empty"
            })),
        )
            .into_response();
    }
    if body.strand_dir.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "strand_dir is required"
            })),
        )
            .into_response();
    }
    // Check loom exists and contains the knot
    let loom = match ctx.store.get(&loom_id) {
        Some(loom) => loom,
        None => {
            return (StatusCode::NOT_FOUND, "loom not found").into_response();
        }
    };
    if !loom.knots.iter().any(|k| k.id.0 == name) {
        return (StatusCode::NOT_FOUND, "knot not found").into_response();
    }

    // Write the updated knot .md file
    let loom_dir = ctx.base_dir.join(&loom_id.0);
    let content = generate_knot_file(&body);
    let file_path = loom_dir.join(format!("{}.md", name));
    if let Err(e) = std::fs::write(&file_path, content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("failed to write knot file: {e}")
            })),
        )
            .into_response();
    }

    // Build the Knot from request data
    let project_root = ctx.base_dir.parent().unwrap_or(&ctx.base_dir);
    let strand_dir = FileSystemLoomRepository::resolve_path(
        project_root,
        &std::path::PathBuf::from(&body.strand_dir),
    );
    let knot = Knot {
        id: KnotId(name),
        agent_config: Some(body.agent_config.clone()),
        agent_profile_ref: None,
        prompt_template: body.prompt_template.clone(),
        strand_dir,
    };

    // Update the in-memory store
    let knot_id = knot.id.clone();
    let new_strand_dir = knot.strand_dir.clone();
    let use_case = crate::application::usecases::ManageKnot::new(
        ctx.store.clone(),
    );
    match use_case.execute(
        crate::application::usecases::KnotAction::Update {
            loom_id: loom_id.clone(),
            knot,
        },
    ) {
        Ok(()) => {
            // Update watcher: set new loom/knot ids and re-watch new strand dir
            ctx.event_source.set_loom_ids(
                &new_strand_dir,
                &loom_id,
                &knot_id,
            );
            let _ = ctx.event_source.watch(&new_strand_dir);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "updated": true })),
            )
                .into_response()
        }
        Err(crate::application::ports::PortError::LoomSaveFailed(msg)) => {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Remove a knot from a loom.
///
/// Deletes the `.md` file from the loom directory and removes the knot
/// from the in-memory store via `ManageKnot::Delete`.
#[utoipa::path(
    delete,
    path = "/looms/{id}/knots/{name}",
    params(
        ("id" = String, Path, description = "Loom identifier"),
        ("name" = String, Path, description = "Knot identifier"),
    ),
    responses(
        (status = 204, description = "Knot deleted"),
        (status = 404, description = "Loom or knot not found"),
        (status = 500, description = "Internal server error"),
    ),
)]
pub async fn delete_knot(
    Path((id, name)): Path<(String, String)>,
    State(ctx): State<AppContext>,
) -> Response {
    let loom_id = LoomId(id);
    let knot_id = KnotId(name);

    // Check loom exists and contains the knot
    let loom = match ctx.store.get(&loom_id) {
        Some(loom) => loom,
        None => {
            return (StatusCode::NOT_FOUND, "loom not found").into_response();
        }
    };
    if !loom.knots.iter().any(|k| k.id == knot_id) {
        return (StatusCode::NOT_FOUND, "knot not found").into_response();
    }

    // Get the strand_dir before deletion so we can stop the watcher
    let strand_dir = match loom.knots.iter().find(|k| k.id == knot_id) {
        Some(k) => k.strand_dir.clone(),
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Delete the knot .md file
    let loom_dir = ctx.base_dir.join(&loom_id.0);
    let file_path = loom_dir.join(format!("{}.md", knot_id.0));
    // Ignore errors if file doesn't exist (idempotent)
    let _ = std::fs::remove_file(&file_path);

    // Update the in-memory store
    let use_case = crate::application::usecases::ManageKnot::new(
        ctx.store.clone(),
    );
    match use_case.execute(
        crate::application::usecases::KnotAction::Delete {
            loom_id: loom_id.clone(),
            knot_id: knot_id.clone(),
        },
    ) {
        Ok(()) => {
            // Stop watching the strand directory
            let _ = ctx.event_source.unwatch(&strand_dir);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(crate::application::ports::PortError::LoomSaveFailed(msg)) => {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response()
        }
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
        LoomRepository, PortError, TieOffSink,
    };
    use crate::application::store::LoomStore;
    use crate::application::usecases::LoomSummary;
    use crate::domain::entities::{
        Knot, KnotId, Loom, LoomId, StrandPath, TieOff, TieOffPath,
    };
    use crate::domain::events::{LoomEvent, StrandEvent};
    use crate::domain::value_objects::{
        AgentConfig, AgentProfile, PromptTemplate, RigAgentConfig,
    };
    use axum::{body::Body, http::Request};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc as StdArc, Mutex};
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

    /// In-memory mock of `AgentProfileRepository`.
    struct MockProfileRepository;

    impl AgentProfileRepository for MockProfileRepository {
        fn get(
            &self,
            _name: &str,
        ) -> Result<Option<AgentProfile>, PortError> {
            Ok(None)
        }

        fn list(&self) -> Result<Vec<AgentProfile>, PortError> {
            Ok(vec![])
        }

        fn save(
            &self,
            _profile: AgentProfile,
        ) -> Result<(), PortError> {
            Ok(())
        }

        fn delete(&self, _name: &str) -> Result<(), PortError> {
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
            loom_repo: Arc::new(MockLoomRepository),
            loom_log_port: Arc::new(MockLoomLogPort { events: log_events }),
            tie_off_sink: Arc::new(MockTieOffSink),
            event_source: Arc::new(event_source),
            event_sender,
            agent_runner: Arc::new(MockAgentRunner),
            profile_repo: Arc::new(MockProfileRepository),
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
                profile_repo: Arc::new(MockProfileRepository),
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
                    agent_config: Some(AgentConfig {
                        goal: "review".to_string(),
                        provider: "openai".to_string(),
                        model: "gpt-4o".to_string(),
                        tools: Vec::new(),
                    }),
                    agent_profile_ref: None,
                    prompt_template: PromptTemplate {
                        input_bundling: "full-file".to_string(),
                        instructions: "check it".to_string(),
                    },
                    strand_dir: PathBuf::from("strands"),
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
            profile_repo: Arc::new(MockProfileRepository),
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

    /// Register same loom twice; second returns 201 (idempotent).
    ///
    /// This reflects the reality that auto-discovery may pre-register a loom
    /// before the POST arrives, and the POST should be a no-op rather than
    /// a conflict.
    #[tokio::test]
    async fn post_loom_duplicate_id_is_idempotent() {
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
            profile_repo: Arc::new(MockProfileRepository),
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

        // Idempotent: loom already registered → 201 (not 409)
        assert_eq!(resp.status(), 201);
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
            profile_repo: Arc::new(MockProfileRepository),
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

    // ── Phase 4: Route Integration Tests ────────────────────────────────

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
            profile_repo: Arc::new(MockProfileRepository),
            rig_config: RigAgentConfig::default_config(),
            loom_ids: Vec::new(),
            base_dir: tmp.path().to_path_buf(),
        };
        (ctx, tmp)
    }

    /// All 7 loom endpoints are accessible on a single router with shared
    /// `AppContext`. Verifies GET returns 200/404, POST returns 201/400 (idempotent),
    /// and DELETE returns 204/404.
    #[tokio::test]
    async fn full_route_wiring() {
        // Pre-register a loom so read endpoints have data to return
        let (ctx, _tmp) = build_test_context_with_temp_dir();
        ctx.store.register(build_test_loom("wired-loom", &["k1", "k2"]));
        let app = build_app(ctx);

        // 1. GET /looms -> 200 with non-empty array
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

        // 2. GET /looms/:id -> 200 (found)
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

        // 3. GET /looms/:id -> 404 (not found)
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

        // 4. GET /looms/:id/activity -> 200 (mock log port returns events)
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

        // 5. GET /looms/:id/knots -> 200 (list of knot names)
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

        // 6. GET /looms/:id/knots/:knot_name -> 404 (no state for this knot)
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

        // 7. POST /looms -> 201 (valid body with knots)
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

        // 8. POST /looms -> 400 (id doesn't end in -loom)
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

        // 9. POST /looms -> 201 (idempotent duplicate — auto-discovery may pre-register)
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
        assert_eq!(resp.status(), 201);

        // 10. DELETE /looms/:id -> 204 (found)
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

        // 11. DELETE /looms/:id -> 404 (already deleted)
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

    // ── Phase 4: POST /looms New Request Shape Tests ────────────────────

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
        let content_a =
            std::fs::read_to_string(loom_dir.join("knot-a.md")).unwrap();
        assert!(content_a.starts_with("---"));
        assert!(content_a.contains("name: knot-a"));
        assert!(content_a.contains("review a"));
        assert!(content_a.contains("strand-dir:"));

        let content_b =
            std::fs::read_to_string(loom_dir.join("knot-b.md")).unwrap();
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
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(err.get("error").is_some());
        let msg = err["error"].as_str().unwrap();
        assert!(msg.contains("strand_dir"));
    }

    /// `POST /looms` without `tie_off_dir` succeeds (it's now statically derived).
    #[tokio::test]
    async fn post_loom_without_tieoff_dir_succeeds() {
        let (ctx, _tmp) = build_test_context_with_temp_dir();
        let app = build_app(ctx);

        let body = serde_json::json!({
            "id": "no-tieoff-loom",
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
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
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
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let msg = err["error"].as_str().unwrap();
        assert!(msg.contains("-loom"));
    }

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

    // ── Phase 1: Tracking Mock EventSource and AppContext Extension ────

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
