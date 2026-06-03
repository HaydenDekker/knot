pub mod adapters;
pub mod application;
pub mod domain;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use tokio::sync::mpsc;

// Re-export inbound adapter types
pub use adapters::inbound::{build_app, AppContext};
pub use domain::value_objects::WorkspaceAgentConfig;

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

// ── Composition Root ───────────────────────────────────────────────────────

/// Configuration for starting the Knot server.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Base directory for filesystem adapters.
    pub base_dir: PathBuf,
    /// Address to bind the HTTP server on.
    pub bind_addr: SocketAddr,
    /// Workspace-level agent configuration.
    pub workspace_config: WorkspaceAgentConfig,
    /// Timeout for subprocess agent runner.
    pub agent_timeout: Duration,
}

impl AppConfig {
    /// Create default configuration: bind `127.0.0.1:3000`, workspace dir `.`.
    pub fn default_config() -> Self {
        Self {
            base_dir: PathBuf::from("."),
            bind_addr: "127.0.0.1:3000".parse().unwrap(),
            workspace_config: WorkspaceAgentConfig::default_config(),
            agent_timeout: Duration::from_secs(120),
        }
    }
}

/// Build the `AppContext` by wiring together all hex layers.
///
/// Creates:
/// - Outbound adapter instances (filesystem adapters, notify watcher, subprocess)
/// - `LoomStore` (in-memory loom registry)
/// - `AppContext` holding store, ports, and workspace config
/// - Debounce engine sender for the event pipeline
///
/// This is the composition root — the only place where all layers meet.
pub fn build_app_context(config: &AppConfig) -> AppContext {
    let store = application::store::LoomStore::new();

    // Outbound adapters (ports implemented with filesystem / subprocess IO)
    let loom_repo: Arc<dyn application::ports::LoomRepository> =
        Arc::new(adapters::outbound::FileSystemLoomRepository::new());
    let knot_state_port: Arc<dyn application::ports::KnotStatePort> =
        Arc::new(adapters::outbound::FileSystemKnotStateStore::new(
            config.base_dir.clone(),
        ));
    let loom_log_port: Arc<dyn application::ports::LoomLogPort> =
        Arc::new(adapters::outbound::FileSystemLoomLog::new(
            config.base_dir.clone(),
        ));
    let tie_off_sink: Arc<dyn application::ports::TieOffSink> =
        Arc::new(adapters::outbound::FileSystemTieOffSink::new(
            config.base_dir.clone(),
        ));

    // Event channel: adapters push raw StrandEvent into this sender.
    // The debounce engine consumes from this channel (Phase 2 wiring).
    let (event_tx, _event_rx) = mpsc::channel(100);
    let _ = _event_rx; // Receiver drained in Phase 2

    AppContext {
        store,
        loom_repo,
        knot_state_port,
        loom_log_port,
        tie_off_sink,
        event_sender: event_tx,
        workspace_config: config.workspace_config.clone(),
    }
}

/// Start the Knot HTTP server with the given configuration.
///
/// Builds the `AppContext`, wires the axum router, and binds to
/// `config.bind_addr`. Blocks until the shutdown signal is received.
///
/// Returns the `SocketAddr` the server is actually listening on.
/// This is useful when `bind_addr.port()` is `0` (random port).
pub async fn start_server(config: AppConfig) -> std::io::Result<()> {
    start_server_with_shutdown(config, ShutdownSignal::CtrlC).await
}

/// Shutdown signal type for the server.
pub enum ShutdownSignal {
    /// Wait for Ctrl+C (default for production).
    CtrlC,
    /// Wait for the provided oneshot channel (useful for tests).
    Channel(tokio::sync::oneshot::Receiver<()>),
}

/// Start the Knot HTTP server with a custom shutdown signal.
///
/// This variant allows callers (especially tests) to control when
/// the server shuts down via an oneshot channel.
pub async fn start_server_with_shutdown(
    config: AppConfig,
    shutdown_signal: ShutdownSignal,
) -> std::io::Result<()> {
    let ctx = build_app_context(&config);
    let app = build_app(ctx);

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;

    let shutdown = async {
        match shutdown_signal {
            ShutdownSignal::CtrlC => {
                let _ = tokio::signal::ctrl_c().await;
            }
            ShutdownSignal::Channel(rx) => {
                let _ = rx.await;
            }
        }
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}
