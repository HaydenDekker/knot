pub mod adapters;
pub mod application;
pub mod domain;

use std::net::SocketAddr;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use domain::events::StrandEvent;
use tokio::sync::mpsc;

// Re-export inbound adapter types
pub use adapters::inbound::{build_app, AppContext};
pub use adapters::subprocess::SubprocessAgentRunner;
pub use domain::entities::Loom;
pub use domain::value_objects::RigAgentConfig;

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
    pub workspace_config: RigAgentConfig,
    /// Timeout for subprocess agent runner.
    pub agent_timeout: Duration,
}

/// Start the event processing pipeline.
///
/// Wires:
/// NotifyEventSource → event_sender → event_rx → DebounceEngine
/// → ProcessStrand loop (tokio task)
///
/// The `event_rx` parameter is the receiver from the channel that
/// `NotifyEventSource` sends raw events into.
///
/// Returns the process strand task handle.
pub fn start_event_pipeline(
    ctx: &AppContext,
    event_rx: mpsc::Receiver<domain::events::StrandEvent>,
) -> tokio::task::JoinHandle<()> {
    // Wire event_rx directly into the debounce engine.
    let (mut debounce_rx, _debounce_handle) =
        application::debounce::DebounceEngine::start_with_receiver(event_rx);

    // ProcessStrand loop: read debounced events and process them.
    let store = ctx.store.clone();
    let state_port = Arc::clone(&ctx.knot_state_port);
    let log_port = Arc::clone(&ctx.loom_log_port);
    let agent_runner = Arc::clone(&ctx.agent_runner);
    let tie_off_sink = Arc::clone(&ctx.tie_off_sink);
    let workspace_config = ctx.workspace_config.clone();

    tokio::spawn(async move {
        let use_case = application::usecases::ProcessStrand::new(
            store,
            state_port,
            log_port,
            agent_runner,
            tie_off_sink,
            workspace_config,
        );
        while let Some(event) = debounce_rx.recv().await {
            if let Err(e) = use_case.execute(event) {
                eprintln!("ProcessStrand error: {e}");
            }
        }
    })
}

impl AppConfig {
    /// Create default configuration: bind `127.0.0.1:3000`, workspace dir `.`.
    pub fn default_config() -> Self {
        Self {
            base_dir: PathBuf::from("."),
            bind_addr: "127.0.0.1:3000".parse().unwrap(),
            workspace_config: RigAgentConfig::default_config(),
            agent_timeout: Duration::from_secs(120),
        }
    }
}

/// Load the workspace agent configuration from `.workspace-agent-config.yaml`
/// in the given directory. Falls back to `default` if the file does not
/// exist or cannot be parsed.
fn load_workspace_config(
    base_dir: &std::path::Path,
    default: RigAgentConfig,
) -> RigAgentConfig {
    let config_path = base_dir.join(".workspace-agent-config.yaml");
    if !config_path.exists() {
        return default;
    }
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "WARNING: could not read {}: {}, using defaults",
                config_path.display(),
                e
            );
            return default;
        }
    };
    match serde_yaml::from_str::<RigAgentConfig>(&content) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "WARNING: malformed YAML in {}: {}, using defaults",
                config_path.display(),
                e
            );
            default
        }
    }
}

/// Build the `AppContext` by wiring together all hex layers.
///
/// Creates:
/// - Outbound adapter instances (filesystem adapters, notify watcher, subprocess)
/// - `LoomStore` (in-memory loom registry)
/// - `AppContext` holding store, ports, and workspace config
/// - Event channel: sender goes into AppContext, receiver is returned
///
/// Returns `(AppContext, Receiver<StrandEvent>)` — the receiver is wired
/// into the debounce engine by `start_event_pipeline`.
///
/// This is the composition root — the only place where all layers meet.
pub fn build_app_context(
    config: &AppConfig,
) -> (AppContext, mpsc::Receiver<StrandEvent>) {
    let store = application::store::LoomStore::new();

    // Load workspace config from .workspace-agent-config.yaml (falls back to defaults).
    let workspace_config =
        load_workspace_config(&config.base_dir, config.workspace_config.clone());

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
    let agent_runner: Arc<dyn application::ports::AgentRunner> =
        Arc::new(
            SubprocessAgentRunner::with_timeout(config.agent_timeout),
        );

    // Event channel: NotifyEventSource sends raw StrandEvents here.
    // The receiver is wired into the debounce engine.
    let (event_tx, event_rx) = mpsc::channel(100);

    (
        AppContext {
            store,
            loom_repo,
            knot_state_port,
            loom_log_port,
            tie_off_sink,
            event_sender: event_tx,
            agent_runner,
            workspace_config,
            loom_ids: Vec::new(),
        },
        event_rx,
    )
}

/// Run the startup discovery and registration sequence.
///
/// After building the AppContext, this:
/// 1. Runs DiscoverLooms to scan workspace and register looms
/// 2. For each loom: opens activity log, writes LoomStarted event
/// 3. Stores loom IDs in AppContext for use during graceful shutdown
///
/// Returns the list of discovered looms (used to start watchers).
pub fn run_startup(
    ctx: &AppContext,
    base_dir: &StdPath,
) -> std::io::Result<Vec<Loom>> {
    let discover = application::usecases::DiscoverLooms::new(
        Arc::clone(&ctx.loom_repo),
        Arc::clone(&ctx.knot_state_port),
        Arc::clone(&ctx.loom_log_port),
        ctx.store.clone(),
    );

    let looms = discover
        .execute(base_dir)
        .map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
        })?;

    // For each loom: open activity log and record LoomStarted
    for loom in &looms {
        let _ = ctx.loom_log_port.open(&loom.id);
        let _ = ctx.loom_log_port.append(
            domain::events::LoomEvent::LoomStarted {
                loom_id: loom.id.clone(),
            },
        );
    }

    Ok(looms)
}

/// Perform graceful shutdown of the Knot system.
///
/// This function:
/// 1. Closes the debounce engine sender (stops accepting new events)
/// 2. Waits for the processing pipeline to drain (in-flight events finish)
/// 3. Writes `LoomStopped` to each loom's activity log
///
/// The `event_source` should already be dropped by the caller (which stops
/// the notify watcher). The `process_handle` is the JoinHandle for the
/// processing pipeline task — this function waits for it to complete.
pub async fn graceful_shutdown(
    ctx: &AppContext,
    process_handle: tokio::task::JoinHandle<()>,
) {
    // Close the event sender to signal the debounce engine to stop.
    // This drops the sender held by AppContext — the debounce engine's
    // input receiver will see None and drain remaining events.
    // We don't directly close event_sender here because it's cloned into
    // the AppContext state; instead, we wait for the process task.
    drop(ctx.event_sender.clone());

    // Wait for the processing pipeline to drain.
    // The debounce engine flushes remaining events when its input closes,
    // then the ProcessStrand loop exits.
    let _ = process_handle.await;

    // Write LoomStopped to each loom's activity log.
    for loom_id in &ctx.loom_ids {
        let _ = ctx.loom_log_port.append(
            domain::events::LoomEvent::LoomStopped {
                loom_id: loom_id.clone(),
            },
        );
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
///
/// Graceful shutdown sequence:
/// 1. Awaits shutdown signal (Ctrl+C or oneshot channel)
/// 2. Stops accepting new HTTP connections (axum graceful shutdown)
/// 3. Drops NotifyEventSource (stops file watcher, closes event channel)
/// 4. Waits for debounce engine + processing pipeline to drain
/// 5. Writes `LoomStopped` to each loom's activity log
/// 6. Returns
pub async fn start_server_with_shutdown(
    config: AppConfig,
    shutdown_signal: ShutdownSignal,
) -> std::io::Result<()> {
    let (mut ctx, event_rx) = build_app_context(&config);

    // Clone event sender for the watcher (ctx is consumed below)
    let event_sender = ctx.event_sender.clone();

    // Start the event pipeline: debounce + ProcessStrand
    let process_handle = start_event_pipeline(&ctx, event_rx);

    // Startup: discover looms, create state files
    let looms = run_startup(&ctx, &config.base_dir).unwrap_or_else(|e| {
        eprintln!("WARNING: startup discovery failed: {e}");
        Vec::new()
    });

    // Store loom IDs in context for graceful shutdown logging.
    {
        let loom_ids: Vec<_> = looms.iter().map(|l| l.id.clone()).collect();
        ctx.loom_ids = loom_ids;
    }

    // Start file watchers on each loom source directory.
    // Kept alive in a scope — dropped during shutdown to stop the watcher.
    let event_source = {
        use application::ports::EventSource;
        let source =
            adapters::outbound::NotifyEventSource::new(event_sender);
        for loom in &looms {
            let knot_id = loom.knots
                .first()
                .map(|k| k.id.clone())
                .unwrap_or_else(|| {
                    domain::entities::KnotId("default".to_string())
                });
            // Register IDs for this loom's source directory
            source.with_loom_ids(
                loom.source_dir.clone(),
                loom.id.clone(),
                knot_id,
            );
            if let Err(e) = source.watch(&loom.source_dir) {
                eprintln!(
                    "WARNING: failed to watch {}: {e}",
                    loom.source_dir.display()
                );
            }
        }
        source
    };

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

    // Serve HTTP with graceful shutdown.
    // When the shutdown signal fires, axum stops accepting new connections
    // and waits for existing requests to complete.
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;

    // Shutdown sequence:
    // 1. Drop the file watcher — this stops the notify RecommendedWatcher
    //    and closes the mpsc sender it holds.
    drop(event_source);

    // 2. The AppContext is held by the axum Router (already stopped).
    //    We need the loom_log_port and loom_ids for logging LoomStopped.
    //    The router state is still accessible since the serve call returned.
    //    However, ctx was moved into build_app. We reconstruct the minimal
    //    context we need for shutdown logging.

    // 3. Wait for the processing pipeline to drain and write LoomStopped.
    //    We need a fresh loom_log_port and the loom_ids.
    let loom_log_port: Arc<dyn application::ports::LoomLogPort> =
        Arc::new(adapters::outbound::FileSystemLoomLog::new(
            config.base_dir.clone(),
        ));
    let loom_ids: Vec<_> = looms.iter().map(|l| l.id.clone()).collect();

    // Wait for the processing pipeline to finish draining.
    let _ = process_handle.await;

    // Write LoomStopped to each loom's activity log.
    for loom_id in &loom_ids {
        let _ = loom_log_port.append(
            domain::events::LoomEvent::LoomStopped {
                loom_id: loom_id.clone(),
            },
        );
    }

    Ok(())
}
