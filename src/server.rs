//! Composition root and server lifecycle.
//!
//! Wires all hexagonal layers together and manages the server lifecycle
//! (startup, event pipeline, graceful shutdown).

use crate::adapters::inbound::AppContext;
use crate::adapters::outbound::FileSystemStateWriter;
use crate::adapters::subprocess::SubprocessAgentRunner;
use crate::application;
use crate::application::ports::{GitVersioningPort, StateWriterPort};
use crate::domain;
use crate::domain::entities::Loom;
use crate::domain::events::{ConfigEvent, StrandEvent};
use crate::adapters::outbound::event_source::WatchType;
use crate::domain::value_objects::RigAgentConfig;

use std::net::SocketAddr;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

// ── Configuration ──────────────────────────────────────────────────────────

/// Configuration for starting the Knot server.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Rig directory for filesystem adapters.
    pub rig_dir: PathBuf,
    /// Address to bind the HTTP server on.
    pub bind_addr: SocketAddr,
    /// Rig-level agent configuration.
    pub rig_config: RigAgentConfig,
    /// Timeout for subprocess agent runner.
    pub agent_timeout: Duration,
}

impl AppConfig {
    /// Create default configuration: bind `127.0.0.1:3000`, rig dir `./rig`.
    pub fn default_config() -> Self {
        let rig_dir = std::env::current_dir()
            .map(|cwd| cwd.join("rig"))
            .unwrap_or_else(|_| PathBuf::from("./rig"));
        Self {
            rig_dir,
            bind_addr: "127.0.0.1:3000".parse().unwrap(),
            rig_config: RigAgentConfig::default_config(),
            agent_timeout: Duration::from_secs(300),
        }
    }

    /// Create configuration with an explicit rig directory.
    ///
    /// All other fields use the same defaults as `default_config()`
    /// (bind `127.0.0.1:3000`, default rig config, 300s agent timeout).
    pub fn with_rig_dir(rig_dir: PathBuf) -> Self {
        Self {
            rig_dir,
            bind_addr: "127.0.0.1:3000".parse().unwrap(),
            rig_config: RigAgentConfig::default_config(),
            agent_timeout: Duration::from_secs(300),
        }
    }
}

/// Load the rig agent configuration from `.workspace-agent-config.yaml`
/// in the given directory. Falls back to `default` if the file does not
/// exist or cannot be parsed.
fn load_rig_config(
    rig_dir: &std::path::Path,
    default: RigAgentConfig,
) -> RigAgentConfig {
    let config_path = rig_dir.join(".workspace-agent-config.yaml");
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

// ── Composition Root ───────────────────────────────────────────────────────

/// Build the `AppContext` by wiring together all hex layers.
///
/// Creates:
/// - Outbound adapter instances (filesystem adapters, notify watcher, subprocess)
/// - `LoomStore` (in-memory loom registry)
/// - `AppContext` holding store, ports, and rig config
/// - Event channels: strand sender and config sender go into AppContext,
///   receivers are returned
///
/// Returns `(AppContext, Receiver<StrandEvent>, Receiver<ConfigEvent>)` —
/// the strand receiver is wired into the debounce engine by
/// `start_event_pipeline`, and the config receiver is wired into
/// `start_config_pipeline`.
///
/// This is the composition root — the only place where all layers meet.
pub fn build_app_context(
    config: &AppConfig,
) -> (
    AppContext,
    mpsc::Receiver<StrandEvent>,
    mpsc::Receiver<ConfigEvent>,
) {
    let store = application::store::LoomStore::new();

    // Load rig config from .rig-agent-config.yaml (falls back to defaults).
    let rig_config =
        load_rig_config(&config.rig_dir, config.rig_config.clone());

    // Outbound adapters (ports implemented with filesystem / subprocess IO)
    let loom_repo: Arc<dyn application::ports::LoomRepository> =
        Arc::new(crate::adapters::outbound::FileSystemLoomRepository::new());
    let loom_log_port: Arc<dyn application::ports::LoomLogPort> =
        Arc::new(crate::adapters::outbound::FileSystemLoomLog::new(
            config.rig_dir.clone(),
        ));
    let tie_off_sink: Arc<dyn application::ports::TieOffSink> =
        Arc::new(crate::adapters::outbound::FileSystemTieOffSink::new(
            config.rig_dir.clone(),
        ));
    let agent_runner: Arc<dyn application::ports::AgentRunner> =
        Arc::new(
            SubprocessAgentRunner::with_timeout(config.agent_timeout),
        );
    let profile_repo: Arc<dyn application::ports::AgentProfileRepository> =
        Arc::new(
            crate::adapters::outbound::FileSystemAgentProfileRepository::new(
                config.rig_dir.join("profiles"),
            ),
        );
    let rig_log_port: Arc<dyn application::ports::RigLogPort> =
        Arc::new(
            crate::adapters::outbound::FileSystemRigLog::new(
                config.rig_dir.clone(),
            ),
        );

    // State writer: writes rig/state.json on a poll cycle.
    let state_writer: Arc<dyn StateWriterPort> = Arc::new(
        FileSystemStateWriter::new(config.rig_dir.clone()),
    );

    // Event channels: NotifyEventSource sends StrandEvents and ConfigEvents.
    // Strand receiver is wired into the debounce engine.
    // Config receiver is wired into the ConfigEventHandler.
    let (strand_tx, strand_rx) = mpsc::channel(100);
    let (config_tx, config_rx) = mpsc::channel(100);

    // File-system event source — created once, shared via AppContext.
    // Handlers can pass this to use cases for watch/unwatch.
    // Project root is the parent of the rig directory, matching the
    // resolution in FileSystemLoomRepository::scan(). This ensures
    // relative strand_dir paths resolve against the project root,
    // not the rig directory.
    let project_root = config.rig_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.rig_dir.clone());
    let event_source: Arc<dyn application::ports::EventSource> =
        Arc::new(
            crate::adapters::outbound::NotifyEventSource::new(
                strand_tx.clone(),
                config_tx,
                project_root,
            ),
        );

    (
        AppContext {
            store,
            loom_repo,
            loom_log_port,
            tie_off_sink,
            event_source,
            event_sender: strand_tx,
            agent_runner,
            profile_repo,
            rig_log_port,
            rig_config,
            loom_ids: Vec::new(),
            rig_dir: config.rig_dir.clone(),
            state_writer,
        },
        strand_rx,
        config_rx,
    )
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
    // `spawn_with_receiver` creates an InspectQueue<Option<StrandEvent>>
    // and returns an Arc to it. The debounce engine pushes `Some(event)`
    // for debounced events and `None` as a shutdown sentinel after
    // flushing pending entries. ProcessStrand reads from the queue
    // directly using pop() + notified().await, breaking on `None`.
    let debounce_rx =
        application::debounce::DebounceEngine::spawn_with_receiver(event_rx, join_set);

    // ProcessStrand loop: read debounced events and process them.
    let store = ctx.store.clone();
    let log_port = Arc::clone(&ctx.loom_log_port);
    let agent_runner = Arc::clone(&ctx.agent_runner);
    let tie_off_sink = Arc::clone(&ctx.tie_off_sink);
    let rig_config = ctx.rig_config.clone();
    let rig_dir = ctx.rig_dir.clone();
    let profile_repo = Arc::clone(&ctx.profile_repo);
    let rig_log_port = Arc::clone(&ctx.rig_log_port);

    // Git versioning: project root is the parent of the rig directory.
    // Falls back to rig_dir itself if parent does not exist.
    let project_root = rig_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| rig_dir.clone());
    let git_versioning_port: Arc<dyn GitVersioningPort> = Arc::new(
        crate::adapters::outbound::FileSystemGitVersioner::new(project_root),
    );

    join_set.spawn(async move {
        let use_case = application::usecases::ProcessStrand::new(
            store,
            log_port,
            agent_runner,
            tie_off_sink,
            rig_config,
            rig_dir,
            profile_repo,
            rig_log_port.clone(),
            git_versioning_port,
        );

        // Process strand events with queue idle detection.
        //
        // After each event, poll for 500ms — if no event arrives,
        // write QueueIdle to the rig-log and go back to blocking.
        //
        // `is_burst_active` controls whether the next recv is blocking
        // (idle, wait for first event) or timed (drain check, detect end
        // of burst). This keeps a single flat loop with no nesting.
        //
        // The queue holds `Option<StrandEvent>`: `Some(event)` for real
        // events, `None` for the shutdown sentinel from the debounce
        // engine. The inner pop+notified loop drains the queue; the
        // outer match handles events vs. shutdown vs. timeout.
        let poll_window = Duration::from_millis(500);
        let mut is_burst_active = false;

        loop {
            // Read next item from the InspectQueue.
            // queue.pop() returns Option<Option<StrandEvent>>:
            //   Some(Some(event)) → real event
            //   Some(None) → shutdown sentinel
            //   None → queue empty, wait for notification
            let next_event: Option<StrandEvent> = if is_burst_active {
                match tokio::time::timeout(poll_window, async {
                    loop {
                        if let Some(item) = debounce_rx.pop() {
                            break item;
                        }
                        debounce_rx.notified().await;
                    }
                }).await {
                    Ok(item) => item,
                    Err(_) => {
                        // Timeout: burst has ended — queue is idle.
                        let ts = application::usecases::format_timestamp();
                        let result = rig_log_port.append(
                            domain::events::RigLogEvent::QueueIdle {
                                timestamp: ts.clone(),
                            },
                        );
                        match result {
                            Ok(()) => {
                                eprintln!("[pipeline] QueueIdle written to rig-log (ts={})", ts);
                            }
                            Err(e) => {
                                eprintln!("[pipeline] QueueIdle WRITE FAILED: {e}");
                            }
                        }
                        is_burst_active = false;
                        continue;
                    }
                }
            } else {
                // Queue is idle; block until a fresh event arrives.
                async {
                    loop {
                        if let Some(item) = debounce_rx.pop() {
                            break item;
                        }
                        debounce_rx.notified().await;
                    }
                }.await
            };

            // Handle the event (or shutdown sentinel).
            match next_event {
                Some(event) => {
                    is_burst_active = true;
                    if let Err(e) = use_case.execute(event) {
                        eprintln!("[pipeline] ProcessStrand error: {e}");
                    }
                    // Loop continues — next poll will use timeout (drain check).
                }
                None => {
                    // Shutdown sentinel — pipeline shutting down.
                    break;
                }
            }
        }
    });
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
    rig_dir: &StdPath,
) -> std::io::Result<Vec<Loom>> {
    // Auto-create the rig directory if it doesn't exist.
    std::fs::create_dir_all(rig_dir).map_err(|e| {
        eprintln!("WARNING: failed to create rig dir {}: {e}", rig_dir.display());
        e
    })?;

    let discover = application::usecases::DiscoverLooms::new(
        Arc::clone(&ctx.loom_repo),
        Arc::clone(&ctx.loom_log_port),
        ctx.store.clone(),
        Arc::clone(&ctx.event_source),
    );

    let looms = discover
        .execute(rig_dir)
        .map_err(|e| {
            std::io::Error::other(e.to_string())
        })?;

    // Register rig directory watch — auto-discover new `*-loom` directories
    // and knot changes within existing looms.
    ctx.event_source
        .register_watch(rig_dir.to_path_buf(), WatchType::Rig);
    if let Err(e) = ctx.event_source.watch(rig_dir) {
        eprintln!("WARNING: failed to watch rig dir: {e}");
    }

    Ok(looms)
}

/// Start the config event processing pipeline.
///
/// Wires:
/// NotifyEventSource → config_sender → config_rx → ConfigEventHandler
///
/// The `config_rx` parameter is the receiver from the channel that
/// `NotifyEventSource` sends config events (new looms, knot changes)
/// into. The handler updates `LoomStore`, manages watchers, and writes
/// loom-log entries.
///
/// Spawns the config handler into the provided `JoinSet`.
pub fn start_config_pipeline(
    ctx: &AppContext,
    mut config_rx: mpsc::Receiver<ConfigEvent>,
    join_set: &mut tokio::task::JoinSet<()>,
) {
    let repository = Arc::clone(&ctx.loom_repo);
    let log_port = Arc::clone(&ctx.loom_log_port);
    let store = ctx.store.clone();
    let event_source = Arc::clone(&ctx.event_source);
    let rig_path = ctx.rig_dir.clone();

    join_set.spawn(async move {
        let use_case = application::usecases::ConfigEventHandler::new(
            repository,
            log_port,
            store,
            event_source,
            rig_path,
        );
        while let Some(event) = config_rx.recv().await {
            if let Err(e) = use_case.execute(event) {
                eprintln!("ConfigEventHandler error: {e}");
            }
        }
    });
}

/// Start the state writer background task.
///
/// Spawns a `tokio::task` that polls every 5 seconds, builds a
/// `RigState` snapshot from the current in-memory state, and writes
/// it atomically to `{rig_dir}/state.json`.
///
/// The task writes immediately on start (so `state.json` exists right
/// away), then enters the 5-second poll cycle.
///
/// Spawns into the provided `JoinSet` so it is a child of the server
/// task and is aborted when the server stops.
pub fn start_state_writer(
    ctx: &AppContext,
    join_set: &mut tokio::task::JoinSet<()>,
) {
    let store = ctx.store.clone();
    let log_port = Arc::clone(&ctx.loom_log_port);
    let profile_repo = Arc::clone(&ctx.profile_repo);
    let state_writer = Arc::clone(&ctx.state_writer);
    let rig_dir = ctx.rig_dir.clone();

    join_set.spawn(async move {
        let use_case = application::usecases::WriteState::new(
            store,
            log_port,
            profile_repo,
            state_writer,
            rig_dir,
        );

        // Write immediately on start so state.json exists right away
        if let Err(e) = use_case.execute() {
            eprintln!("[state-writer] initial write failed: {e}");
        }

        let poll_interval = Duration::from_secs(5);
        let mut interval = tokio::time::interval(poll_interval);

        loop {
            interval.tick().await;
            if let Err(e) = use_case.execute() {
                eprintln!("[state-writer] write failed: {e}");
            }
        }
    });
}

// ── Server Lifecycle ───────────────────────────────────────────────────────

/// Shutdown signal type for the server.
pub enum ShutdownSignal {
    /// Wait for Ctrl+C (default for production).
    CtrlC,
    /// Wait for the provided oneshot channel (useful for tests).
    Channel(tokio::sync::oneshot::Receiver<()>),
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
    let (mut ctx, strand_rx, config_rx) = build_app_context(&config);

    // JoinSet ties the pipeline task lifetimes to the server task.
    let mut join_set = tokio::task::JoinSet::new();

    // Start the config event pipeline: ConfigEventHandler (child of this task)
    start_config_pipeline(&ctx, config_rx, &mut join_set);

    // Start the strand event pipeline: debounce + ProcessStrand (child of this task)
    start_event_pipeline(&ctx, strand_rx, &mut join_set);

    // Start the state writer: writes rig/state.json every 5 seconds
    start_state_writer(&ctx, &mut join_set);

    // Startup: discover looms, create state files, start watchers
    let looms = run_startup(&ctx, &config.rig_dir).unwrap_or_else(|e| {
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

    let app = crate::adapters::inbound::build_app(ctx);

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
    // 5. ConfigEventHandler: its config_rx.recv() yields None (config_tx
    //    was dropped with NotifyEventSource) → task exits naturally.
    //
    // 6. JoinSet drained: `while let Some` loop waits for ALL tasks to
    //    complete. No tasks are aborted — they all exit cooperatively.
    //
    // 7. LoomStopped written to each loom-log.
    //
    // Safety net: if a task has a bug and never exits, the JoinSet Drop
    // will abort it. This is a last resort, not the primary mechanism.

    // Drain all pipeline tasks with a timeout safety net.
    //
    // The cooperative cascade (channel closure → recv()→None → exit) is
    // the primary shutdown mechanism. But the notify background thread
    // holds an Arc reference to the event senders, which can delay channel
    // closure by tens of milliseconds. If the drain doesn't complete within
    // the timeout, abort remaining tasks as a last resort.
    //
    // See ADR-003 pattern 4 for the timeout safety net rationale.
    let drain_timeout = Duration::from_secs(5);
    let drain_result = tokio::time::timeout(drain_timeout, async {
        while let Some(res) = join_set.join_next().await {
            if let Err(e) = res {
                eprintln!("Background task failed: {e}");
            }
        }
    })
    .await;

    if drain_result.is_err() {
        eprintln!(
            "WARNING: pipeline tasks did not drain within {:?}, aborting",
            drain_timeout
        );
        join_set.abort_all();
    }

    // Write LoomStopped to each loom's activity log.
    for loom_id in &shutdown_loom_ids {
        let _ = shutdown_log_port.append(
            domain::events::LoomEvent::LoomStopped {
                loom_id: loom_id.clone(),
                timestamp: application::usecases::format_timestamp(),
            },
        );
    }

    Ok(())
}
