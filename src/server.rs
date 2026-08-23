//! Composition root and server lifecycle.
//!
//! Wires all hexagonal layers together and manages the server lifecycle
//! (startup, event pipeline, graceful shutdown).

use crate::adapters::outbound::{DiskBackedEventQueue, FileSystemStateWriter};
use crate::adapters::pi_json::PiJsonAgentRunner;
use crate::adapters::pi_stdio::PiStdioAgentRunner;
use crate::application;
use crate::application::ports::{GitVersioningPort, StateWriterPort, StrandEventQueue};
use crate::domain;
use crate::domain::entities::Loom;
use crate::domain::knot_file::derive_runtime_root;
use crate::domain::pending_event::PendingEventOrShutdown;
use crate::domain::events::{ConfigEvent, StrandEvent};
use crate::adapters::outbound::event_source::WatchType;
use crate::domain::value_objects::{AgentAdapter, RigAgentConfig};

use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

// ── AppContext ────────────────────────────────────────────────────────────

/// Application context passed to all layers.
///
/// Holds port instances, the in-memory store, and rig configuration.
/// Cloned and passed to use cases and background tasks.
#[derive(Clone)]
pub struct AppContext {
    /// In-memory loom registry.
    pub store: application::store::LoomStore,
    /// Loom repository port.
    pub loom_repo: Arc<dyn application::ports::LoomRepository>,
    /// Loom log port.
    pub loom_log_port: Arc<dyn application::ports::LoomLogPort>,
    /// Tie-off sink port.
    pub tie_off_sink: Arc<dyn application::ports::TieOffSink>,
    /// File-system event source — used to watch/unwatch source dirs.
    pub event_source: Arc<dyn application::ports::EventSource>,
    /// Debounce engine sender — feed raw strand events.
    pub event_sender: mpsc::Sender<StrandEvent>,
    /// Agent runner for subprocess execution.
    pub agent_runner: Arc<dyn application::ports::AgentRunner>,
    /// Agent profile repository for dynamic profile resolution.
    pub profile_repo: Arc<dyn application::ports::AgentProfileRepository>,
    /// Model registry (rig/models.yml) for resolving `model-ref`
    /// aliases — read fresh per strand and per state write.
    pub model_registry: Arc<dyn application::ports::ModelRegistryPort>,
    /// Rig-log port for recording operational events (timeouts, idle).
    pub rig_log_port: Arc<dyn application::ports::RigLogPort>,
    /// Rig-level agent configuration.
    pub rig_config: RigAgentConfig,
    /// Discovered loom IDs (populated at startup, used for shutdown logging).
    pub loom_ids: Vec<domain::entities::LoomId>,
    /// Rig directory path — used by discover and config endpoints.
    /// Holds reusable rig source only (no runtime data).
    pub rig_dir: PathBuf,
    /// Project-side runtime root — `tie-offs/<rig-basename>/` under the
    /// project root. Holds all runtime artifacts (tie-offs, dispatch
    /// dirs, loom-logs, state.json, .rig-log, events/).
    pub runtime_root: PathBuf,
    /// State writer port — writes `state.json` at the runtime root.
    pub state_writer: Arc<dyn StateWriterPort>,
    /// Git versioning port — commits agent work at the project root and
    /// ensures the rig has its own git repository at startup.
    pub git_versioning: Arc<dyn GitVersioningPort>,
    /// Strand event queue — shared with WriteState for queue visibility.
    pub strand_queue: Arc<std::sync::Mutex<Option<Arc<dyn StrandEventQueue>>>>,
}

// ── Configuration ─────────────────────────────────────────────────────────

/// Configuration for starting the Knot service.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Rig directory for filesystem adapters.
    pub rig_dir: PathBuf,
    /// Rig-level agent configuration.
    pub rig_config: RigAgentConfig,
    /// Timeout for subprocess agent runner.
    pub agent_timeout: Duration,
    /// Optional explicit path to the agent CLI binary.
    ///
    /// When `Some`, overrides the default PATH-based resolution of
    /// the `pi` binary. Used by integration smoke tests to inject
    /// a mock agent without manipulating process-global `PATH`.
    pub cli_path: Option<PathBuf>,
}

impl AppConfig {
    /// Create default configuration: rig dir `./rig`.
    pub fn default_config() -> Self {
        let rig_dir = std::env::current_dir()
            .map(|cwd| cwd.join("rig"))
            .unwrap_or_else(|_| PathBuf::from("./rig"));
        Self {
            rig_dir,
            rig_config: RigAgentConfig::default_config(),
            agent_timeout: Duration::from_secs(300),
            cli_path: None,
        }
    }

    /// Create configuration with an explicit rig directory.
    ///
    /// All other fields use the same defaults as `default_config()`
    /// (default rig config, 300s agent timeout).
    pub fn with_rig_dir(rig_dir: PathBuf) -> Self {
        Self {
            rig_dir,
            rig_config: RigAgentConfig::default_config(),
            agent_timeout: Duration::from_secs(300),
            cli_path: None,
        }
    }

    /// Create configuration with an explicit agent CLI path.
    ///
    /// Clones all fields from the given config and overrides `cli_path`.
    /// When set, the agent runner uses this path directly instead of
    /// resolving `pi` from PATH or `KNOT_TEST_CLI_PATH`.
    pub fn with_cli_path(mut self, cli_path: PathBuf) -> Self {
        self.cli_path = Some(cli_path);
        self
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
        match rig_config.agent_adapter {
            AgentAdapter::PiJson => {
                if let Some(ref cli_path) = config.cli_path {
                    Arc::new(PiJsonAgentRunner::with_cli_path_and_timeout(
                        cli_path.to_string_lossy().to_string(),
                        config.agent_timeout,
                    ))
                } else {
                    Arc::new(PiJsonAgentRunner::with_timeout(
                        config.agent_timeout,
                    ))
                }
            }
            AgentAdapter::PiStdio => {
                if let Some(ref cli_path) = config.cli_path {
                    Arc::new(PiStdioAgentRunner::with_cli_path_and_timeout(
                        cli_path.to_string_lossy().to_string(),
                        config.agent_timeout,
                    ))
                } else {
                    Arc::new(PiStdioAgentRunner::with_timeout(
                        config.agent_timeout,
                    ))
                }
            }
        };
    let profile_repo: Arc<dyn application::ports::AgentProfileRepository> =
        Arc::new(
            crate::adapters::outbound::FileSystemAgentProfileRepository::new(
                config.rig_dir.join("profiles"),
            ),
        );

    // Model registry: reads rig/models.yml fresh on every load (no
    // caching) so a model swap behind an alias is picked up on the next
    // strand, without a restart.
    let model_registry: Arc<dyn application::ports::ModelRegistryPort> =
        Arc::new(crate::adapters::outbound::FileSystemModelRegistry::new(
            config.rig_dir.clone(),
        ));

    // Runtime root: all runtime artifacts (tie-offs, dispatch dirs,
    // loom-logs, state.json, .rig-log, events/) live under
    // tie-offs/<rig-basename>/ at the project level — the rig directory
    // holds reusable source only.
    let runtime_root = derive_runtime_root(&config.rig_dir);

    let rig_log_port: Arc<dyn application::ports::RigLogPort> = Arc::new(
        crate::adapters::outbound::FileSystemRigLog::new(runtime_root.clone()),
    );

    // State writer: writes state.json at the runtime root on a poll cycle.
    let state_writer: Arc<dyn StateWriterPort> =
        Arc::new(FileSystemStateWriter::new(runtime_root.clone()));

    // Project root is the parent of the rig directory, matching the
    // resolution in FileSystemLoomRepository::scan(). This ensures
    // relative strand_dir paths resolve against the project root,
    // not the rig directory.
    let project_root = config.rig_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.rig_dir.clone());

    // Git versioning: commits agent work at the project root, keeps the
    // rig out of every project commit (gitlink/stale-file guard), and
    // ensures the rig has its own git repository at startup
    // (ensure_rig_repo).
    let git_versioning: Arc<dyn GitVersioningPort> = Arc::new(
        crate::adapters::outbound::FileSystemGitVersioner::new(
            project_root.clone(),
            config.rig_dir.clone(),
        ),
    );

    // Event channels: NotifyEventSource sends StrandEvents and ConfigEvents.
    // Strand receiver is wired into the debounce engine.
    // Config receiver is wired into the ConfigEventHandler.
    let (strand_tx, strand_rx) = mpsc::channel(100);
    let (config_tx, config_rx) = mpsc::channel(100);

    // File-system event source — created once, shared via AppContext.
    // Handlers can pass this to use cases for watch/unwatch.
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
            model_registry,
            rig_log_port,
            rig_config,
            loom_ids: Vec::new(),
            rig_dir: config.rig_dir.clone(),
            runtime_root,
            state_writer,
            git_versioning,
            strand_queue: Arc::new(std::sync::Mutex::new(None)),
        },
        strand_rx,
        config_rx,
    )
}

/// Set up the event processing pipeline (queue + debounce engine).
///
/// Wires:
/// NotifyEventSource → event_sender → event_rx → DebounceEngine
/// → StrandEventQueue (disk-backed)
///
/// The `event_rx` parameter is the receiver from the channel that
/// `NotifyEventSource` sends raw events into.
///
/// This creates the queue, loads persisted events, and spawns the
/// debounce engine into the provided `JoinSet`. The process-strand
/// loop is **not** spawned here — call `spawn_process_strand_loop()`
/// after `run_startup()` completes so that persisted events are only
/// processed after looms have been discovered.
///
/// Returns the `Arc<dyn StrandEventQueue>` so it can be shared with
/// `start_state_writer` for queue visibility.
pub fn start_event_pipeline(
    ctx: &AppContext,
    event_rx: mpsc::Receiver<domain::events::StrandEvent>,
    join_set: &mut tokio::task::JoinSet<()>,
) -> Arc<DiskBackedEventQueue> {
    // Wire event_rx into the debounce engine, spawned into the join set.
    //
    // The debounce engine pushes `PendingEvent` for debounced events
    // and calls `push_shutdown()` as a sentinel after flushing pending
    // entries. ProcessStrand reads from the queue directly using
    // pop() + notified().await, breaking on Shutdown.
    // Read test debounce timing from env vars (set by test helpers),
    // falling back to production defaults.
    let debounce_window = std::env::var("KNOT_TEST_DEBOUNCE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(application::debounce::DEFAULT_DEBOUNCE_WINDOW);
    let check_interval = std::env::var("KNOT_TEST_CHECK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(application::debounce::DEFAULT_CHECK_INTERVAL);

    // Event queue lives at the runtime root (tie-offs/<rig-basename>/events/).
    let events_dir = ctx.runtime_root.join("events");
    std::fs::create_dir_all(&events_dir).unwrap_or_else(|e| {
        eprintln!("WARNING: failed to create events dir {}: {e}", events_dir.display());
    });
    let debounce_queue: Arc<DiskBackedEventQueue> =
        Arc::new(DiskBackedEventQueue::new(events_dir));
    // Load persisted events from disk before starting the debounce engine.
    // This ensures any events left over from a previous run are re-queued
    // before new file-watcher events arrive.
    let loaded = debounce_queue.load_persisted();
    if loaded > 0 {
        eprintln!("[startup] loaded {} persisted event(s) from disk", loaded);
    }

    let debounce_queue = application::debounce::DebounceEngine::spawn_with_receiver_with_window_and_queue(
        event_rx, join_set, debounce_window, check_interval, debounce_queue,
    );

    // Store the queue Arc in AppContext so WriteState can snapshot it.
    {
        let mut guard = ctx.strand_queue.lock().unwrap();
        *guard = Some(Arc::clone(&debounce_queue) as Arc<dyn StrandEventQueue>);
    }

    debounce_queue
}

/// Spawn the process-strand loop as a background task.
///
/// Call this **after** `run_startup()` completes so that looms are
/// discovered before any persisted events are processed. If called
/// before discovery, persisted events referencing those looms will
/// fail with "loom not found".
///
/// Wires the process-strand loop to read from the disk-backed queue,
/// execute agent invocations, and log queue idle events.
pub fn spawn_process_strand_loop(
    ctx: &AppContext,
    debounce_queue: Arc<DiskBackedEventQueue>,
    join_set: &mut tokio::task::JoinSet<()>,
) {
    // ProcessStrand loop: read debounced events and process them.
    let store = ctx.store.clone();
    let log_port = Arc::clone(&ctx.loom_log_port);
    let agent_runner = Arc::clone(&ctx.agent_runner);
    let tie_off_sink = Arc::clone(&ctx.tie_off_sink);
    let rig_config = ctx.rig_config.clone();
    let rig_dir = ctx.rig_dir.clone();
    let profile_repo = Arc::clone(&ctx.profile_repo);
    let model_registry = Arc::clone(&ctx.model_registry);
    let rig_log_port = Arc::clone(&ctx.rig_log_port);

    // Git versioning — wired in the composition root (project root).
    let git_versioning_port = Arc::clone(&ctx.git_versioning);

    // Clone debounce_queue before moving into the closure.
    let debounce_queue_inner = Arc::clone(&debounce_queue);
    join_set.spawn(async move {
        let use_case = Arc::new(application::usecases::ProcessStrand::new(
            store,
            log_port,
            agent_runner,
            tie_off_sink,
            rig_config,
            rig_dir,
            profile_repo,
            model_registry,
            rig_log_port.clone(),
            git_versioning_port,
            Arc::new(
                crate::adapters::outbound::ContentInspectorChecker,
            ),
            Arc::new(
                crate::adapters::outbound::event_dispatcher::FileSystemEventDispatcher::new(),
            ),
            Some(debounce_queue_inner.clone() as Arc<dyn domain::events::StrandQueueAccessor>),
        ));

        // Process strand events with queue idle detection.
        //
        // After each event, poll for 500ms — if no event arrives,
        // write QueueIdle to the rig-log and go back to blocking.
        //
        // `is_burst_active` controls whether the next recv is blocking
        // (idle, wait for first event) or timed (drain check, detect end
        // of burst). This keeps a single flat loop with no nesting.
        //
        // The queue holds `PendingEventOrShutdown`:
        // `PendingEventOrShutdown::Event` for real events,
        // `PendingEventOrShutdown::Shutdown` for the shutdown sentinel
        // from the debounce engine. The inner pop+notified loop drains
        // the queue; the outer match handles events vs. shutdown vs.
        // timeout.
        let poll_window = Duration::from_millis(500);
        let mut is_burst_active = false;
        let mut shutdown_signaled = false;

        loop {
            // Read next item from the StrandEventQueue.
            // pop() returns Option<PendingEventOrShutdown>:
            //   Some(Event) → real event
            //   Some(Shutdown) → shutdown sentinel
            //   None → queue empty (no sentinel), wait for notification
            let next_item: Option<PendingEventOrShutdown> = if is_burst_active {
                match tokio::time::timeout(poll_window, async {
                    loop {
                        let item = debounce_queue_inner.pop();
                        if item.is_some() {
                            break item;
                        }
                        debounce_queue_inner.notified().await;
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
                        let item = debounce_queue_inner.pop();
                        if item.is_some() {
                            break item;
                        }
                        debounce_queue_inner.notified().await;
                    }
                }.await
            };

            // Handle shutdown sentinel — if received but queue might have
            // more events, keep draining. Break only when shutdown + empty.
            if matches!(next_item, Some(PendingEventOrShutdown::Shutdown)) {
                if !shutdown_signaled {
                    shutdown_signaled = true;
                    // Don't break yet — queue might have more events.
                    // Next pop() will return None (empty) or Shutdown again.
                    continue;
                }
                // Shutdown already signaled and queue is empty.
                break;
            }
            // If pop() returned None but shutdown was previously signaled,
            // the queue is empty — exit.
            if next_item.is_none() && shutdown_signaled {
                break;
            }

            // Handle the event.
            if let Some(PendingEventOrShutdown::Event(pending)) = next_item {
                is_burst_active = true;
                // Convert PendingEvent → StrandEvent for processing.
                let event: domain::events::StrandEvent = match pending.try_into() {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!("[pipeline] invalid pending event: {e}");
                        continue;
                    }
                };
                // Run agent execution on a blocking thread so the tokio
                // task yields. This allows the task to be aborted during
                // shutdown — without this, the synchronous execute() call
                // blocks the tokio thread with no yield point, preventing
                // graceful shutdown (process hangs on Ctrl+C).
                let use_case = Arc::clone(&use_case);
                let result = tokio::task::spawn_blocking(
                    move || use_case.execute(event),
                ).await;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        eprintln!("[pipeline] ProcessStrand error: {e}");
                    }
                    Err(e) => {
                        eprintln!("[pipeline] ProcessStrand blocking task failed: {e}");
                    }
                }
                // Loop continues — next poll will use timeout (drain check).
            }
        }
    });
}

/// Move one legacy runtime path to the runtime root.
///
/// No-op when `src` does not exist. When `dst` already exists, the
/// destination is kept and the source is left in place for manual
/// resolution (no data loss) — a warning is logged in either
/// failure case, never an error: startup continues with whatever
/// layout is on disk.
fn move_legacy_path(src: &StdPath, dst: &StdPath, label: &str, moved: &mut Vec<String>) {
    if !src.exists() {
        return;
    }
    if dst.exists() {
        eprintln!(
            "WARNING: [startup] migration conflict: {label} — {} already exists; legacy {} kept for manual resolution",
            dst.display(),
            src.display(),
        );
        return;
    }
    match std::fs::rename(src, dst) {
        Ok(()) => moved.push(label.to_string()),
        Err(e) => eprintln!(
            "WARNING: [startup] migration: failed to move {label} ({} → {}): {e}",
            src.display(),
            dst.display(),
        ),
    }
}

/// Migrate a rig from the legacy layout — runtime artifacts inside the
/// rig directory — to the project-level runtime root.
///
/// Moves (subtree preserved):
/// - `rig/tie-offs/` → the runtime root itself (its `{loom-id}/` tree
///   becomes the runtime root's `{loom-id}/` tree)
/// - `rig/state.json` → `<runtime-root>/state.json`
/// - `rig/.rig-log` → `<runtime-root>/.rig-log`
/// - `rig/events/` → `<runtime-root>/events/`
///
/// Idempotent: a rig already on the new layout is a no-op (and no
/// empty runtime root is created). Destination-exists conflicts keep
/// the destination and warn; the source stays for manual resolution.
/// All failures are non-fatal warnings — the rig directory is left
/// source-only whenever a move succeeds.
///
/// Must run before discovery and watcher registration so that moved
/// dispatch directories, loom-logs, and the event queue are used at
/// their new paths immediately (loom-log appends and watch
/// registrations derive their paths at call time).
fn migrate_legacy_rig_layout(rig_dir: &StdPath) {
    let runtime_root = derive_runtime_root(rig_dir);
    let mut moved: Vec<String> = Vec::new();

    // 1. The legacy tie-off tree becomes the runtime root itself. The
    //    parent (`tie-offs/`) must exist for the rename target.
    let legacy_tieoffs = rig_dir.join("tie-offs");
    if legacy_tieoffs.exists() {
        if let Some(parent) = runtime_root.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            eprintln!(
                "WARNING: [startup] migration: failed to create {}: {e}",
                parent.display(),
            );
            return;
        }
        move_legacy_path(
            &legacy_tieoffs,
            &runtime_root,
            "tie-offs/",
            &mut moved,
        );
    }

    // 2. Remaining runtime artifacts — create the runtime root when
    //    needed (it may not exist if there was no legacy tie-offs
    //    tree). A fresh rig gets no empty runtime root.
    let remaining = [
        ("state.json", "state.json"),
        (".rig-log", ".rig-log"),
        ("events", "events/"),
    ];
    if remaining
        .iter()
        .any(|(name, _)| rig_dir.join(name).exists())
    {
        if let Err(e) = std::fs::create_dir_all(&runtime_root) {
            eprintln!(
                "WARNING: [startup] migration: failed to create runtime root {}: {e}",
                runtime_root.display(),
            );
            return;
        }
        for (name, label) in remaining {
            move_legacy_path(
                &rig_dir.join(name),
                &runtime_root.join(name),
                label,
                &mut moved,
            );
        }
    }

    if !moved.is_empty() {
        eprintln!(
            "[startup] migrated legacy layout: {} → {}",
            moved.join(", "),
            runtime_root.display(),
        );
    }
}

/// Run the startup discovery and registration sequence.
///
/// After building the AppContext, this:
/// 1. Migrates the legacy layout (runtime artifacts out of the rig)
/// 2. Clears the operational logs (rig-log + every loom-log) — per-run
///    scope: tie-offs hold the durable audit history
/// 3. Ensures the rig has its own git repository
/// 4. Runs DiscoverLooms to scan rig and register looms
/// 5. DiscoverLooms handles log events, storage, and watchers internally
/// 6. Returns list of discovered looms
///
/// Migration precedes the clear so moved legacy logs are cleared at
/// their new paths, and the clear precedes discovery so the fresh
/// `KnotRegistered`/`LoomStarted` events are the first log lines of the
/// run. Clearing is non-fatal: a failure logs a `WARNING:` and startup
/// proceeds.
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

    // Auto-create agent config file if missing so the rig has an explicit
    // config rather than relying on implicit defaults.
    let config_path = rig_dir.join(".workspace-agent-config.yaml");
    if !config_path.exists() {
        let config = r#"# Rig-level agent configuration.
#
# agent-adapter: which adapter to use for Pi invocations.
#   pi-stdio — plain text stdout (default, current behaviour)
#   pi-json  — JSON-L stream with session ID + token usage capture
#
agent-adapter: pi-stdio
"#;
        std::fs::write(&config_path, config).map_err(|e| {
            eprintln!("WARNING: failed to write {}: {e}", config_path.display());
            e
        })?;
    }

    // Auto-create the model registry file if missing so the rig has an
    // explicit (empty) registry rather than an absent file. Idempotent:
    // an existing models.yml is never overwritten.
    let models_path = rig_dir.join("models.yml");
    if !models_path.exists() {
        let models_template = r#"# Rig-level model registry.
#
# Maps model aliases to {provider, model} pairs. Profiles reference an
# alias via `model-ref:` in their frontmatter; the alias is resolved
# fresh on every strand, so swapping the model behind an alias is a
# single edit here — picked up live, without a restart.
#
# Both `provider` and `model` are required per alias, e.g.:
#
# models:
#   default:
#     provider: openai
#     model: gpt-4o
#
# While `models:` is absent (or empty), the registry is empty: profiles
# with `model-ref` fail to resolve (ModelRefNotFound); profiles with a
# direct provider/model spec are unaffected.
"#;
        std::fs::write(&models_path, models_template).map_err(|e| {
            eprintln!("WARNING: failed to write {}: {e}", models_path.display());
            e
        })?;
    }

    // Migrate the legacy layout (runtime artifacts inside the rig dir)
    // to the project-level runtime root. Idempotent and non-fatal —
    // runs after rig-dir creation and before any watcher registration.
    migrate_legacy_rig_layout(rig_dir);

    // Clear operational logs: per-run scope — the tie-offs (git-
    // versioned) hold the durable audit history, so prior-run log
    // content is residue. Runs after migration (moved legacy logs are
    // cleared at their new paths) and before discovery (no fresh event
    // is ever discarded). Non-fatal — a failure must not block startup.
    if let Err(e) = ctx.loom_log_port.clear_all() {
        eprintln!("WARNING: failed to clear loom-logs: {e}");
    }
    if let Err(e) = ctx.rig_log_port.clear() {
        eprintln!("WARNING: failed to clear rig-log: {e}");
    }

    // Ensure the rig has its own git repository and the parent project
    // repo (if any) excludes it. Idempotent and non-fatal — runs after
    // rig-dir creation and before any watcher registration.
    if let Err(e) = ctx.git_versioning.ensure_rig_repo(rig_dir) {
        eprintln!("WARNING: rig git init: {e}");
    }

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
/// it atomically to `{runtime-root}/state.json`. Includes the current
/// strand event queue contents in the snapshot.
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
    let model_registry = Arc::clone(&ctx.model_registry);
    let state_writer = Arc::clone(&ctx.state_writer);
    let rig_dir = ctx.rig_dir.clone();
    let strand_queue = Arc::clone(&ctx.strand_queue);

    join_set.spawn(async move {
        let use_case = application::usecases::WriteState::new(
            store,
            log_port,
            profile_repo,
            model_registry,
            state_writer,
            rig_dir,
            strand_queue,
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

/// Start the Knot service.
///
/// Builds the `AppContext`, runs startup (legacy-layout migration, rig
/// git init, loom discovery, watcher registration), then starts the
/// background pipelines (event, config, state writer) and blocks until
/// Ctrl+C is received.
///
/// Startup completes before any pipeline task is spawned: the
/// migration must precede the first state write (state writer) and the
/// event queue's load of `tie-offs/<rig>/events/` (event pipeline), or
/// those tasks create the runtime root first and the migration skips
/// the conflicting moves. Looms must also be discovered before
/// persisted events referencing them are loaded.
///
/// Graceful shutdown sequence:
/// 1. Awaits Ctrl+C
/// 2. Drains pipeline tasks with timeout safety net
/// 3. Writes `LoomStopped` to each loom's activity log
/// 4. Returns
pub async fn start_knot(config: AppConfig) -> std::io::Result<()> {
    let (mut ctx, strand_rx, config_rx) = build_app_context(&config);

    // JoinSet ties the pipeline task lifetimes to the server task.
    let mut join_set = tokio::task::JoinSet::new();

    // Startup first: migrate the legacy layout, ensure the rig git
    // repo, discover looms, register watchers. This must complete
    // before any pipeline task is spawned — on a multi-threaded
    // runtime a spawned task runs immediately: the state writer's
    // first write and the event queue's events-dir creation would
    // create the runtime root before the migration runs, turning
    // legacy `rig/tie-offs/` and `rig/events/` into false conflicts
    // that are never moved.
    let looms = run_startup(&ctx, &config.rig_dir).unwrap_or_else(|e| {
        eprintln!("WARNING: startup discovery failed: {e}");
        Vec::new()
    });

    // Start the config event pipeline: ConfigEventHandler (child of this task)
    start_config_pipeline(&ctx, config_rx, &mut join_set);

    // Start the strand event pipeline: creates the queue, loads persisted
    // events, and spawns the debounce engine. Looms are already
    // discovered (run_startup), so persisted events referencing them
    // resolve.
    let debounce_queue = start_event_pipeline(&ctx, strand_rx, &mut join_set);

    // Start the state writer: writes state.json (runtime root) every 5 seconds
    start_state_writer(&ctx, &mut join_set);

    // Spawn the process-strand loop. Persisted events loaded during
    // start_event_pipeline can be processed safely since their target
    // looms are registered.
    spawn_process_strand_loop(&ctx, debounce_queue, &mut join_set);

    // Store loom IDs in context for graceful shutdown logging.
    {
        let loom_ids: Vec<_> = looms.iter().map(|l| l.id.clone()).collect();
        ctx.loom_ids = loom_ids;
    }

    // Preserve references needed after AppContext is consumed.
    let shutdown_log_port: Arc<dyn application::ports::LoomLogPort> =
        Arc::clone(&ctx.loom_log_port);
    let shutdown_loom_ids: Vec<_> = looms.iter().map(|l| l.id.clone()).collect();

    // Wait for Ctrl+C
    let _ = tokio::signal::ctrl_c().await;

    // ── Graceful Cascade Shutdown ─────────────────────────────────────
    //
    // The shutdown sequence is a cooperative cascade, not forced abort:
    //
    // 1. Ctrl+C received — AppContext is still alive but no new events
    //    will be triggered.
    //
    // 2. The event_sender clone held by AppContext will be dropped when
    //    ctx goes out of scope (after this function returns).
    //
    // 3. We abort the JoinSet tasks — they are background workers that
    //    will be reaped on next startup. The notify watcher thread
    //    holds its own Arc references and will be cleaned up by the
    //    OS when the process exits.
    //
    // 4. LoomStopped written to each loom-log.

    // Drain all pipeline tasks with a timeout safety net.
    //
    // The cooperative cascade (channel closure → recv()→None → exit) is
    // the primary shutdown mechanism. But the notify background thread
    // holds an Arc reference to the event senders, which can delay channel
    // closure by tens of milliseconds. If the drain doesn't complete within
    // the timeout, abort remaining tasks as a last resort.
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

// ── Composition Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod composition_tests {
    use super::*;
    use crate::application::ports::AgentRunner;
    use std::fs;
    use tempfile::TempDir;

    /// With `agent_adapter: pi-json`, composition wires `PiJsonAgentRunner`.
    #[test]
    fn test_composition_uses_json_runner() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        fs::create_dir_all(&rig_dir).unwrap();

        // Write rig config selecting pi-json adapter
        let config_path = rig_dir.join(".workspace-agent-config.yaml");
        fs::write(&config_path, "agent-adapter: pi-json\n").unwrap();

        let config = AppConfig::with_rig_dir(rig_dir.clone());
        let (ctx, _strand_rx, _config_rx) = build_app_context(&config);

        assert_eq!(
            ctx.agent_runner.runner_type(),
            "pi-json",
            "expected PiJsonAgentRunner for agent_adapter: pi-json",
        );
    }

    /// With `agent_adapter: pi-stdio` or default, composition wires
    /// `PiStdioAgentRunner`.
    #[test]
    fn test_composition_uses_stdio_runner() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        fs::create_dir_all(&rig_dir).unwrap();

        // No config file — defaults to pi-stdio
        let config = AppConfig::with_rig_dir(rig_dir.clone());
        let (ctx, _strand_rx, _config_rx) = build_app_context(&config);

        assert_eq!(
            ctx.agent_runner.runner_type(),
            "pi-stdio",
            "expected PiStdioAgentRunner for default adapter",
        );
    }

    /// Explicit `agent_adapter: pi-stdio` also wires `PiStdioAgentRunner`.
    #[test]
    fn test_composition_uses_stdio_runner_explicit() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        fs::create_dir_all(&rig_dir).unwrap();

        let config_path = rig_dir.join(".workspace-agent-config.yaml");
        fs::write(&config_path, "agent-adapter: pi-stdio\n").unwrap();

        let config = AppConfig::with_rig_dir(rig_dir.clone());
        let (ctx, _strand_rx, _config_rx) = build_app_context(&config);

        assert_eq!(
            ctx.agent_runner.runner_type(),
            "pi-stdio",
            "expected PiStdioAgentRunner for agent_adapter: pi-stdio",
        );
    }

    /// `run_startup()` creates `.workspace-agent-config.yaml` if missing.
    #[test]
    fn test_startup_creates_config_file() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");

        // Rig dir does not exist yet — run_startup creates it
        assert!(!rig_dir.exists());

        let config = AppConfig::with_rig_dir(rig_dir.clone());
        let (ctx, _strand_rx, _config_rx) = build_app_context(&config);
        let _looms = run_startup(&ctx, rig_dir.to_path_buf().as_ref()).unwrap();

        // Rig directory created
        assert!(rig_dir.is_dir());

        // Config file created with default adapter
        let config_path = rig_dir.join(".workspace-agent-config.yaml");
        assert!(config_path.exists(), "config file should be created");
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(
            content.contains("agent-adapter: pi-stdio"),
            "config should default to pi-stdio"
        );
        assert!(
            content.contains("pi-json"),
            "config should document pi-json as available adapter"
        );
    }

    /// `run_startup()` does NOT overwrite existing config file.
    #[test]
    fn test_startup_preserves_existing_config() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        fs::create_dir_all(&rig_dir).unwrap();

        // Pre-existing config selecting pi-json
        let config_path = rig_dir.join(".workspace-agent-config.yaml");
        fs::write(&config_path, "agent-adapter: pi-json\n").unwrap();

        let config = AppConfig::with_rig_dir(rig_dir.clone());
        let (ctx, _strand_rx, _config_rx) = build_app_context(&config);
        let _looms = run_startup(&ctx, rig_dir.to_path_buf().as_ref()).unwrap();

        // Config file should still be pi-json, not overwritten
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(
            content.contains("agent-adapter: pi-json"),
            "existing config should be preserved"
        );
    }

    /// `run_startup()` initialises the rig git repository — verifies the
    /// `GitVersioningPort::ensure_rig_repo` wiring (idempotent, non-fatal,
    /// and no `.gitignore` written into the source-only rig dir).
    #[test]
    fn test_startup_ensures_rig_git_repository() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");

        let config = AppConfig::with_rig_dir(rig_dir.clone());
        let (ctx, _strand_rx, _config_rx) = build_app_context(&config);
        let _looms = run_startup(&ctx, rig_dir.to_path_buf().as_ref()).unwrap();

        assert!(
            rig_dir.join(".git").exists(),
            "run_startup should git init the rig dir"
        );
        assert!(
            !rig_dir.join(".gitignore").exists(),
            "no .gitignore should be written into the rig dir"
        );

        // Idempotent — a second startup does not fail or re-write.
        let _looms = run_startup(&ctx, rig_dir.to_path_buf().as_ref()).unwrap();
        assert!(rig_dir.join(".git").exists());
    }

    // ── Legacy layout migration ─────────────────────────────────────────

    /// Helper: create a rig in the legacy layout — reusable source plus
    /// runtime artifacts inside the rig directory.
    fn setup_legacy_rig(base: &std::path::Path, rig_name: &str) -> PathBuf {
        let rig_dir = base.join(rig_name);
        // Rig source (stays in place)
        let loom_src = rig_dir.join("review-loom");
        fs::create_dir_all(&loom_src).unwrap();
        fs::write(loom_src.join("k.md"), "knot source").unwrap();
        let profiles = rig_dir.join("profiles");
        fs::create_dir_all(&profiles).unwrap();
        fs::write(profiles.join("fast.md"), "profile").unwrap();

        // Legacy runtime artifacts (must move to the runtime root)
        let tieoffs = rig_dir.join("tie-offs");
        fs::create_dir_all(tieoffs.join("review-loom")).unwrap();
        fs::create_dir_all(tieoffs.join("other-loom").join("Ev1")).unwrap();
        fs::write(
            tieoffs.join("review-loom").join("tie-off-k.md"),
            "tie-off",
        )
        .unwrap();
        fs::write(
            tieoffs.join("review-loom").join(".loom-log"),
            "log-line\n",
        )
        .unwrap();
        fs::write(
            tieoffs.join("other-loom").join("Ev1").join("event-1.md"),
            "event",
        )
        .unwrap();
        fs::write(rig_dir.join("state.json"), "{}\n").unwrap();
        fs::write(rig_dir.join(".rig-log"), "rig-log\n").unwrap();
        fs::create_dir_all(rig_dir.join("events")).unwrap();
        fs::write(rig_dir.join("events").join("e1.json"), "{}\n").unwrap();
        rig_dir
    }

    /// The full legacy layout moves to the runtime root: the tie-offs
    /// subtree (loom-logs, dispatch dirs included) becomes the runtime
    /// root's `{loom-id}/` tree, and state.json / .rig-log / events/
    /// move beside it. The rig is left source-only.
    #[test]
    fn test_migrate_moves_full_legacy_layout() {
        let dir = TempDir::new().unwrap();
        let rig_dir = setup_legacy_rig(dir.path(), "rig");
        let runtime_root = dir.path().join("tie-offs").join("rig");

        migrate_legacy_rig_layout(&rig_dir);

        // Tie-off subtree preserved at the runtime root
        assert_eq!(
            fs::read_to_string(
                runtime_root.join("review-loom").join("tie-off-k.md")
            )
            .unwrap(),
            "tie-off"
        );
        assert_eq!(
            fs::read_to_string(
                runtime_root.join("review-loom").join(".loom-log")
            )
            .unwrap(),
            "log-line\n"
        );
        assert_eq!(
            fs::read_to_string(
                runtime_root
                    .join("other-loom")
                    .join("Ev1")
                    .join("event-1.md")
            )
            .unwrap(),
            "event"
        );
        // Runtime files at the runtime root
        assert_eq!(
            fs::read_to_string(runtime_root.join("state.json")).unwrap(),
            "{}\n"
        );
        assert_eq!(
            fs::read_to_string(runtime_root.join(".rig-log")).unwrap(),
            "rig-log\n"
        );
        assert_eq!(
            fs::read_to_string(runtime_root.join("events").join("e1.json"))
                .unwrap(),
            "{}\n"
        );
        // Legacy sources gone
        assert!(!rig_dir.join("tie-offs").exists());
        assert!(!rig_dir.join("state.json").exists());
        assert!(!rig_dir.join(".rig-log").exists());
        assert!(!rig_dir.join("events").exists());
        // Rig source intact
        assert_eq!(
            fs::read_to_string(rig_dir.join("review-loom").join("k.md"))
                .unwrap(),
            "knot source"
        );
        assert_eq!(
            fs::read_to_string(rig_dir.join("profiles").join("fast.md"))
                .unwrap(),
            "profile"
        );
    }

    /// A second migration is a no-op — content is unchanged and nothing
    /// is re-moved or clobbered.
    #[test]
    fn test_migrate_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let rig_dir = setup_legacy_rig(dir.path(), "rig");
        let runtime_root = dir.path().join("tie-offs").join("rig");

        migrate_legacy_rig_layout(&rig_dir);
        migrate_legacy_rig_layout(&rig_dir);

        assert_eq!(
            fs::read_to_string(runtime_root.join("state.json")).unwrap(),
            "{}\n"
        );
        assert_eq!(
            fs::read_to_string(
                runtime_root.join("review-loom").join("tie-off-k.md")
            )
            .unwrap(),
            "tie-off"
        );
        assert!(!rig_dir.join("tie-offs").exists());
        assert!(!rig_dir.join("state.json").exists());
    }

    /// Destination-exists conflict: the destination is kept and the
    /// legacy source stays in place for manual resolution (no data
    /// loss). Non-conflicting items still move.
    #[test]
    fn test_migrate_keeps_destination_on_conflict() {
        let dir = TempDir::new().unwrap();
        let rig_dir = setup_legacy_rig(dir.path(), "rig");
        // The runtime root already exists with its own state.json (the
        // rig already ran on the new layout).
        let runtime_root = dir.path().join("tie-offs").join("rig");
        fs::create_dir_all(&runtime_root).unwrap();
        fs::write(runtime_root.join("state.json"), "new\n").unwrap();

        migrate_legacy_rig_layout(&rig_dir);

        // Destination kept
        assert_eq!(
            fs::read_to_string(runtime_root.join("state.json")).unwrap(),
            "new\n"
        );
        // Legacy source kept for manual resolution — no data loss
        assert!(
            rig_dir.join("state.json").exists(),
            "conflicting legacy source must not be deleted"
        );
        assert!(
            rig_dir.join("tie-offs").exists(),
            "conflicting legacy tie-offs dir must not be deleted"
        );
        // Non-conflicting items still move
        assert_eq!(
            fs::read_to_string(runtime_root.join(".rig-log")).unwrap(),
            "rig-log\n"
        );
        assert!(!rig_dir.join(".rig-log").exists());
        assert!(
            runtime_root.join("events").join("e1.json").exists()
        );
        assert!(!rig_dir.join("events").exists());
    }

    /// A rig already on the new layout is a no-op — no empty runtime
    /// root is created.
    #[test]
    fn test_migrate_noop_when_new_layout() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        fs::create_dir_all(rig_dir.join("review-loom")).unwrap();
        fs::write(rig_dir.join("review-loom").join("k.md"), "knot")
            .unwrap();

        migrate_legacy_rig_layout(&rig_dir);

        assert!(
            !dir.path().join("tie-offs").exists(),
            "no runtime root should be created for a fresh rig"
        );
        assert_eq!(
            fs::read_to_string(rig_dir.join("review-loom").join("k.md"))
                .unwrap(),
            "knot"
        );
    }

    /// A named rig migrates to its own namespaced runtime root
    /// (`tie-offs/dev-rig/`).
    #[test]
    fn test_migrate_named_rig() {
        let dir = TempDir::new().unwrap();
        let rig_dir = setup_legacy_rig(dir.path(), "dev-rig");
        let runtime_root = dir.path().join("tie-offs").join("dev-rig");

        migrate_legacy_rig_layout(&rig_dir);

        assert_eq!(
            fs::read_to_string(runtime_root.join("state.json")).unwrap(),
            "{}\n"
        );
        assert!(
            runtime_root
                .join("review-loom")
                .join("tie-off-k.md")
                .exists()
        );
        assert!(!rig_dir.join("tie-offs").exists());
    }

    /// A partial legacy layout (only some runtime artifacts present)
    /// moves exactly what exists.
    #[test]
    fn test_migrate_partial_legacy() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        fs::create_dir_all(rig_dir.join("events")).unwrap();
        fs::write(rig_dir.join("events").join("e1.json"), "{}\n").unwrap();

        migrate_legacy_rig_layout(&rig_dir);

        assert_eq!(
            fs::read_to_string(
                dir.path()
                    .join("tie-offs")
                    .join("rig")
                    .join("events")
                    .join("e1.json")
            )
            .unwrap(),
            "{}\n"
        );
        assert!(!rig_dir.join("events").exists());
    }

    /// `run_startup()` clears the operational logs AFTER migration and
    /// BEFORE discovery: the rig-log is truncated (prior-run events and
    /// unparseable lines gone), every loom-log — including orphaned
    /// loom dirs — is truncated, the fresh run's `KnotRegistered` is the
    /// first loom-log line, and only log files are touched (tie-off
    /// files and dispatch-dir files survive byte-identical). Knot state
    /// derivation from the cleared log still works.
    #[test]
    fn test_startup_clears_logs_before_discovery() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        // Valid loom with one knot so discovery registers it
        let loom_src = rig_dir.join("review-loom");
        fs::create_dir_all(&loom_src).unwrap();
        fs::write(
            loom_src.join("k.md"),
            "---\nname: k\nagent-profile-ref: fast\nstrand-dir: \"../external-source\"\n---\n\nDo the thing.\n",
        )
        .unwrap();

        // Pre-populate the runtime root (current layout — no migration)
        let runtime_root = dir.path().join("tie-offs").join("rig");
        let review_rt = runtime_root.join("review-loom");
        fs::create_dir_all(review_rt.join("KnotCompleted")).unwrap();
        // Prior-run rig-log, including one unparseable line (the
        // repeated-WARN noise source this plan removes)
        fs::write(
            runtime_root.join(".rig-log"),
            concat!(
                "{\"QueueIdle\":{\"timestamp\":\"2026-08-01T10:00:00Z\"}}\n",
                "corrupt prior-run line\n",
            ),
        )
        .unwrap();
        // Prior-run loom-log (last run's LoomStopped bracket)
        fs::write(
            review_rt.join(".loom-log"),
            "{\"LoomStopped\":{\"loom_id\":\"review-loom\",\"timestamp\":\"2026-08-01T10:30:00Z\"}}\n",
        )
        .unwrap();
        // Durable files that must survive the clear byte-identical
        let tie_off = review_rt.join("tie-off-k.md");
        fs::write(&tie_off, "durable tie-off\n").unwrap();
        let dispatch_file = review_rt.join("KnotCompleted").join("event-1.md");
        fs::write(&dispatch_file, "pending dispatch\n").unwrap();
        // Orphaned loom dir — no `old-loom` in the rig directory
        let orphan = runtime_root.join("old-loom");
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join(".loom-log"), "orphan prior-run line\n")
            .unwrap();

        let config = AppConfig::with_rig_dir(rig_dir.clone());
        let (ctx, _strand_rx, _config_rx) = build_app_context(&config);
        let looms = run_startup(&ctx, rig_dir.to_path_buf().as_ref()).unwrap();
        assert_eq!(looms.len(), 1, "the loom should be discovered");

        // Rig-log: exists and is empty — prior-run events (and the
        // corrupt line) are gone
        let rig_log = runtime_root.join(".rig-log");
        assert!(rig_log.exists(), "rig-log should exist after startup");
        assert_eq!(
            fs::read_to_string(&rig_log).unwrap(),
            "",
            "rig-log must be cleared at startup"
        );

        // Loom-log: exists, non-empty, first event is the fresh
        // KnotRegistered, no prior-run events present
        let loom_log = review_rt.join(".loom-log");
        let content =
            fs::read_to_string(&loom_log).expect("loom-log at runtime root");
        assert!(!content.is_empty(), "fresh run's events must be logged");
        assert!(
            !content.contains("LoomStopped"),
            "prior-run events must be cleared: {content}"
        );
        let first: crate::domain::events::LoomEvent = serde_json::from_str(
            content.lines().next().unwrap(),
        )
        .unwrap();
        match first {
            crate::domain::events::LoomEvent::KnotRegistered { knot_id, .. } => {
                assert_eq!(knot_id.0, "k");
            }
            other => panic!(
                "first loom-log event must be the fresh KnotRegistered, \
                 got {other:?}"
            ),
        }
        assert!(
            content.contains("LoomStarted"),
            "current run's LoomStarted must be logged: {content}"
        );

        // Orphaned loom dir's log is cleared too
        assert_eq!(
            fs::read_to_string(orphan.join(".loom-log")).unwrap(),
            "",
            "orphaned loom-log must be cleared"
        );

        // Only log files touched — durable artifacts byte-identical
        assert_eq!(
            fs::read_to_string(&tie_off).unwrap(),
            "durable tie-off\n"
        );
        assert_eq!(
            fs::read_to_string(&dispatch_file).unwrap(),
            "pending dispatch\n"
        );

        // State derivation is not regressed: a state write after the
        // clear derives the knot as idle with last_event_at set from
        // the fresh KnotRegistered.
        let use_case = application::usecases::WriteState::new(
            ctx.store.clone(),
            std::sync::Arc::clone(&ctx.loom_log_port),
            std::sync::Arc::clone(&ctx.profile_repo),
            std::sync::Arc::clone(&ctx.model_registry),
            std::sync::Arc::clone(&ctx.state_writer),
            rig_dir.clone(),
            std::sync::Arc::clone(&ctx.strand_queue),
        );
        use_case.execute().unwrap();
        let state: crate::domain::entities::RigState = serde_json::from_str(
            &fs::read_to_string(runtime_root.join("state.json")).unwrap(),
        )
        .unwrap();
        let state_loom = state
            .looms
            .iter()
            .find(|l| l.id == "review-loom")
            .expect("review-loom in state");
        assert_eq!(state_loom.knots.len(), 1);
        let knot = &state_loom.knots[0];
        assert_eq!(knot.id, "k");
        assert_eq!(knot.status, "idle");
        assert!(
            knot.last_event_at.is_some(),
            "last_event_at must be set from the fresh KnotRegistered"
        );
    }

    /// `run_startup()` migrates the legacy layout BEFORE the log clear
    /// and discovery: the legacy loom-log moves to the runtime root,
    /// is cleared (per-run scope — its prior content is residue), and
    /// discovery appends the fresh run's `LoomStarted` to it at the NEW
    /// path (loom-log paths are derived at append time, so an append at
    /// the new path proves migration ran first). The rig is left
    /// source-only.
    ///
    /// Migration-order intent preserved: the file at the new path
    /// exists and carries only the current run's events (the moved
    /// legacy line is gone because the clear runs after the move, not
    /// because the move failed).
    #[test]
    fn test_startup_migrates_legacy_layout() {
        let dir = TempDir::new().unwrap();
        let rig_dir = dir.path().join("rig");
        // Valid loom so discovery registers it (knot frontmatter is
        // parsed by the repository scan).
        let loom_src = rig_dir.join("review-loom");
        fs::create_dir_all(&loom_src).unwrap();
        fs::write(
            loom_src.join("k.md"),
            "---\nname: k\nagent-profile-ref: fast\nstrand-dir: \"../external-source\"\n---\n\nDo the thing.\n",
        )
        .unwrap();
        // Legacy runtime artifacts
        let tieoffs = rig_dir.join("tie-offs");
        fs::create_dir_all(tieoffs.join("review-loom")).unwrap();
        fs::write(
            tieoffs.join("review-loom").join(".loom-log"),
            "legacy-line\n",
        )
        .unwrap();
        fs::write(rig_dir.join("state.json"), "{}\n").unwrap();

        let config = AppConfig::with_rig_dir(rig_dir.clone());
        let (ctx, _strand_rx, _config_rx) = build_app_context(&config);
        let looms = run_startup(&ctx, rig_dir.to_path_buf().as_ref()).unwrap();

        // Loom discovered
        assert_eq!(looms.len(), 1, "the loom should be discovered");

        // Legacy artifacts moved to the runtime root
        let runtime_root = dir.path().join("tie-offs").join("rig");
        assert!(runtime_root.join("state.json").exists());
        assert!(!rig_dir.join("state.json").exists());
        assert!(!rig_dir.join("tie-offs").exists(), "rig left source-only");

        // Discovery appended to the moved loom-log at the new path —
        // migration preceded log appends and watcher registration.
        // The startup clear (post-migration, pre-discovery) removed the
        // moved legacy line: the file holds the current run's events
        // only.
        let loom_log = runtime_root.join("review-loom").join(".loom-log");
        let content =
            fs::read_to_string(&loom_log).expect("loom-log at runtime root");
        assert!(
            !content.contains("legacy-line"),
            "moved loom-log must be cleared at its new path: {content}"
        );
        assert!(
            content.contains("LoomStarted"),
            "discovery appended to the new-path loom-log: {content}"
        );
    }
}
