use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

use knot::adapters::outbound::{
    FileSystemKnotStateStore, FileSystemLoomLog, FileSystemLoomRepository,
    FileSystemTieOffSink,
};
use knot::{build_app, AppContext};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let base_dir = PathBuf::from(".");
    let (event_sender, _event_rx) = mpsc::channel(100);
    let _ = _event_rx;

    let ctx = AppContext {
        store: knot::application::store::LoomStore::new(),
        loom_repo: Arc::new(FileSystemLoomRepository::new()),
        knot_state_port: Arc::new(FileSystemKnotStateStore::new(base_dir.clone())),
        loom_log_port: Arc::new(FileSystemLoomLog::new(base_dir.clone())),
        tie_off_sink: Arc::new(FileSystemTieOffSink::new(base_dir)),
        event_sender,
    };

    let app = build_app(ctx);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await
}
