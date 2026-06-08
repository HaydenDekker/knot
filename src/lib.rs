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

// ── Composition Root ───────────────────────────────────────────────────────

/// Configuration for starting the Knot server.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Base directory for filesystem adapters.
    pub base_dir: PathBuf,
    /// Address to bind the HTTP server on.
    pub bind_addr: SocketAddr,
    /// Rig-level agent configuration.
    pub rig_config: RigAgentConfig,
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
/// Spawns both the debounce engine and process strand into the provided
/// `JoinSet`. This ensures the pipeline tasks are children of the server
/// task and are aborted when the server stops.
pub fn start_event_pipeline(
    ctx: &AppContext,
    event_rx: mpsc::Receiver<domain::events::StrandEvent>,
    join_set: &mut tokio::task::JoinSet<()>,
) {
    // Wire event_rx into the debounce engine, spawned into the join set.
    //
    // `spawn_with_receiver` creates an output channel, moves `output_tx` into
    // the spawned debounce task, and returns `output_rx` to the caller.
    // When the debounce task exits (input channel closed → flush → return),
    // its `output_tx` is dropped → `output_rx.recv()` yields None →
    // ProcessStrand exits naturally.
    let mut debounce_rx =
        application::debounce::DebounceEngine::spawn_with_receiver(event_rx, join_set);

    // ProcessStrand loop: read debounced events and process them.
    let store = ctx.store.clone();
    let log_port = Arc::clone(&ctx.loom_log_port);
    let agent_runner = Arc::clone(&ctx.agent_runner);
    let tie_off_sink = Arc::clone(&ctx.tie_off_sink);
    let rig_config = ctx.rig_config.clone();

    join_set.spawn(async move {
        let use_case = application::usecases::ProcessStrand::new(
            store,
            log_port,
            agent_runner,
            tie_off_sink,
            rig_config,
        );
        while let Some(event) = debounce_rx.recv().await {
            if let Err(e) = use_case.execute(event) {
                eprintln!("ProcessStrand error: {e}");
            }
        }
    });
}

impl AppConfig {
    /// Create default configuration: bind `127.0.0.1:3000`, rig dir `./rig`.
    pub fn default_config() -> Self {
        let base_dir = std::env::current_dir()
            .map(|cwd| cwd.join("rig"))
            .unwrap_or_else(|_| PathBuf::from("./rig"));
        Self {
            base_dir,
            bind_addr: "127.0.0.1:3000".parse().unwrap(),
            rig_config: RigAgentConfig::default_config(),
            agent_timeout: Duration::from_secs(120),
        }
    }
}

/// Load the rig agent configuration from `.rig-agent-config.yaml`
/// in the given directory. Falls back to `default` if the file does not
/// exist or cannot be parsed.
fn load_rig_config(
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
/// - `AppContext` holding store, ports, and rig config
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

    // Load rig config from .rig-agent-config.yaml (falls back to defaults).
    let rig_config =
        load_rig_config(&config.base_dir, config.rig_config.clone());

    // Outbound adapters (ports implemented with filesystem / subprocess IO)
    let loom_repo: Arc<dyn application::ports::LoomRepository> =
        Arc::new(adapters::outbound::FileSystemLoomRepository::new());
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

    // File-system event source — created once, shared via AppContext.
    // Handlers can pass this to use cases for watch/unwatch.
    let event_source: Arc<dyn application::ports::EventSource> =
        Arc::new(
            adapters::outbound::NotifyEventSource::new(event_tx.clone()),
        );

    (
        AppContext {
            store,
            loom_repo,
            loom_log_port,
            tie_off_sink,
            event_source,
            event_sender: event_tx,
            agent_runner,
            rig_config,
            loom_ids: Vec::new(),
            base_dir: config.base_dir.clone(),
        },
        event_rx,
    )
}

/// Run the startup discovery and registration sequence.
///
/// After building the AppContext, this:
/// 1. Runs DiscoverLooms to scan rig and register looms
/// 2. DiscoverLooms handles log events, storage, and watchers internally
/// 3. Returns list of discovered looms
///
/// Returns the list of discovered looms.
pub fn run_startup(
    ctx: &AppContext,
    base_dir: &StdPath,
) -> std::io::Result<Vec<Loom>> {
    // Auto-create the rig directory if it doesn't exist.
    std::fs::create_dir_all(base_dir).map_err(|e| {
        eprintln!("WARNING: failed to create rig dir {}: {e}", base_dir.display());
        e
    })?;

    let discover = application::usecases::DiscoverLooms::new(
        Arc::clone(&ctx.loom_repo),
        Arc::clone(&ctx.loom_log_port),
        ctx.store.clone(),
        Arc::clone(&ctx.event_source),
    );

    let looms = discover
        .execute(base_dir)
        .map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
        })?;

    Ok(looms)
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

    // JoinSet ties the pipeline task lifetimes to the server task.
    let mut join_set = tokio::task::JoinSet::new();

    // Start the event pipeline: debounce + ProcessStrand (children of this task)
    start_event_pipeline(&ctx, event_rx, &mut join_set);

    // Startup: discover looms, create state files, start watchers
    let looms = run_startup(&ctx, &config.base_dir).unwrap_or_else(|e| {
        eprintln!("WARNING: startup discovery failed: {e}");
        Vec::new()
    });

    // Store loom IDs in context for graceful shutdown logging.
    {
        let loom_ids: Vec<_> = looms.iter().map(|l| l.id.clone()).collect();
        ctx.loom_ids = loom_ids;
    }

    // Preserve references needed after AppContext is consumed by the router.
    let shutdown_log_port: Arc<dyn application::ports::LoomLogPort> =
        Arc::clone(&ctx.loom_log_port);
    let shutdown_loom_ids: Vec<_> = looms.iter().map(|l| l.id.clone()).collect();

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

    // ── Graceful Cascade Shutdown ─────────────────────────────────────
    //
    // The shutdown sequence is a cooperative cascade, not forced abort:
    //
    // 1. axum::serve has exited — HTTP server stopped, AppContext dropped,
    //    NotifyEventSource dropped (file watcher stopped).
    //
    // 2. The event_sender clone held by AppContext is dropped. Any other
    //    clones (e.g., from route handlers) are also gone.
    //
    // 3. DebounceEngine: its input rx.recv() yields None → flushes all
    //    pending entries to output channel → task exits naturally.
    //
    // 4. ProcessStrand: finishes in-flight agent execution → writes
    //    tie-off → its debounce_rx.recv() yields None → exits naturally.
    //
    // 5. JoinSet drained: `while let Some` loop waits for ALL tasks to
    //    complete. No tasks are aborted — they all exit cooperatively.
    //
    // 6. LoomStopped written to each loom-log.
    //
    // Safety net: if a task has a bug and never exits, the JoinSet Drop
    // will abort it. This is a last resort, not the primary mechanism.

    // Drain all pipeline tasks. Use `while let Some` to wait for every
    // task to complete — not just the first one. This ensures ProcessStrand
    // finishes in-flight agent work before the JoinSet is dropped.
    while let Some(res) = join_set.join_next().await {
        if let Err(e) = res {
            eprintln!("Background task failed: {e}");
        }
    }

    // Write LoomStopped to each loom's activity log.
    for loom_id in &shutdown_loom_ids {
        let _ = shutdown_log_port.append(
            domain::events::LoomEvent::LoomStopped {
                loom_id: loom_id.clone(),
            },
        );
    }

    Ok(())
}
