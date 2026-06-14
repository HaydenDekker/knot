//! System HTTP handlers (health, agents, rig config).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use std::path::PathBuf;

use crate::adapters::inbound::types::{AppContext, RigConfigResponse};

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
