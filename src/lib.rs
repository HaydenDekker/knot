use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use std::path::PathBuf;

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

/// Build the application router.
pub fn build_app() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/agents/{dir}", get(list_agents))
}
