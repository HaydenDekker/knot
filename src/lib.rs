pub mod adapters;
pub mod application;
pub mod domain;

use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use std::path::PathBuf;

// Re-export inbound adapter types
pub use adapters::inbound::{build_app, AppContext};

/// HTTP handler — health check.
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// HTTP handler — list agents in a directory.
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
