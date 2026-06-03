use axum::{body::Body, http::Request, routing::get, Router};
use knot::{health, list_agents};
use tower::util::ServiceExt;

#[tokio::test]
async fn health_returns_ok() {
    let app = Router::new().route("/health", get(health));

    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body.as_ref(), b"ok");
}

#[tokio::test]
async fn list_agents_returns_404_for_missing_dir() {
    let app = Router::new().route("/agents/{dir}", get(list_agents));

    let req = Request::builder()
        .uri("/agents/nonexistent_xyz")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn list_agents_returns_directory_contents() {
    let tmp = tempfile::tempdir().unwrap();
    let dir_path = tmp.path().to_string_lossy().to_string();

    std::fs::write(tmp.path().join("alpha"), "{}").unwrap();
    std::fs::write(tmp.path().join("beta"), "{}").unwrap();

    let app = Router::new().route("/agents/{dir}", get(list_agents));

    let encoded = dir_path.replace('/', "%2F");
    let req = Request::builder()
        .uri(&format!("/agents/{encoded}"))
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let names: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert_eq!(names.len(), 2);
}
