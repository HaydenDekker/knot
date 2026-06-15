//! System HTTP handlers (health, agents, rig config).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use std::path::PathBuf;

use crate::adapters::inbound::types::{AppContext, LoomSummary, RigConfigResponse};
use crate::application::usecases::ReloadConfig;

/// HTTP handler — health check.
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Health check ok"),
    ),
)]
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// HTTP handler — list agents in a directory.
#[utoipa::path(
    get,
    path = "/agents/{dir}",
    params(
        ("dir" = String, Path, description = "Directory to list agents in"),
    ),
    responses(
        (status = 200, body = Vec<String>, description = "List of agent names"),
        (status = 404, description = "Directory not found"),
    ),
)]
pub async fn list_agents(Path(dir): Path<String>) -> Response {
    let path = PathBuf::from(dir);
    match std::fs::read_dir(&path) {
        Ok(entries) => {
            let names: Vec<String> = entries
                .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect();
            (StatusCode::OK, Json(names)).into_response()
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            format!("Directory not found: {e}"),
        )
            .into_response(),
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
pub async fn get_rig_config(State(ctx): State<AppContext>) -> Response {
    let response = RigConfigResponse {
        rig_path: ctx.rig_dir.clone(),
        cli_path: ctx.rig_config.cli_path.clone(),
        cli_args: ctx.rig_config.cli_args.clone(),
    };
    (StatusCode::OK, Json(response)).into_response()
}

/// Re-scan the rig and register any looms not already in the store.
///
/// Provides manual recovery when the file watcher misses an event.
/// Returns JSON array of `LoomSummary` for newly discovered looms.
#[utoipa::path(
    post,
    path = "/config/reload",
    responses(
        (status = 200, body = Vec<LoomSummary>, description = "Newly discovered looms"),
    ),
)]
pub async fn reload_config(State(ctx): State<AppContext>) -> Response {
    let use_case = ReloadConfig::new(
        ctx.loom_repo.clone(),
        ctx.loom_log_port.clone(),
        ctx.store.clone(),
        ctx.event_source.clone(),
        ctx.rig_dir.clone(),
    );

    match use_case.execute() {
        Ok(new_loom_ids) => {
            let summaries: Vec<LoomSummary> = new_loom_ids
                .into_iter()
                .filter_map(|id| {
                    ctx.store.get(&id).map(|loom| LoomSummary {
                        id: loom.id.clone(),
                        knot_count: loom.knots.len(),
                    })
                })
                .collect();
            (StatusCode::OK, Json(summaries)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
