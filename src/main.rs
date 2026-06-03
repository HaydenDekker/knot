#[tokio::main]
async fn main() -> std::io::Result<()> {
    let app = knot::build_app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await
}
