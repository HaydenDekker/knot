//! Shared test helpers for Knot integration tests.
//!
//! Provides file-based polling helpers to verify rig state via
//! `rig/state.json`, replacing the previous HTTP-based verification.
//! Also includes fixtures for creating knots, profiles, and mock agents.
//!
//! The [`ProcessStrandBuilder`] provides a fluent builder for constructing
//! `ProcessStrand` with all mock ports wired up, replacing the duplicated
//! local `build_process_strand` functions that existed in each test file.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use knot::application::ports::AgentRunner;
use knot::application::store::LoomStore;
use knot::application::usecases::test_fixtures::*;
use knot::application::usecases::ProcessStrand;
use knot::domain::entities::Loom;
use knot::domain::value_objects::AgentProfile;
use knot::RigAgentConfig;
use serde_json::Value;

// ── ProcessStrandBuilder ──────────────────────────────────────────────

/// Result of building a [`ProcessStrand`] use case with mocked ports.
///
/// All common handles are always present. Optional tracking ports are
/// `Some` only when explicitly requested via the builder.
pub struct ProcessStrandResult {
    /// The `ProcessStrand` use case instance.
    pub strand: ProcessStrand,
    /// Captured loom-log events.
    pub log_events: Arc<Mutex<Vec<knot::domain::events::LoomEvent>>>,
    /// Recorded tie-off appends (one per `append()` call).
    pub tie_off_appends: Arc<Mutex<Vec<knot::domain::entities::TieOff>>>,
    /// Captured rig-log events.
    pub rig_events: Arc<Mutex<Vec<knot::domain::events::RigLogEvent>>>,
    /// Tie-off content keyed by path display string.
    pub tie_off_content: Arc<Mutex<HashMap<String, String>>>,
    /// The mock agent runner (captures execution contexts).
    pub agent_runner: Arc<MockAgentRunner>,
    /// Git versioning port — present only when `.with_tracking_git()` is used.
    pub git_port: Option<Arc<MockGitVersioningPort>>,
    /// Git commits recorded by the tracking git port.
    pub git_commits: Option<Arc<Mutex<Vec<(knot::domain::entities::LoomId, knot::domain::entities::KnotId, String, String, String)>>>>,
    /// Strand file checker — present only when `.with_tracking_file_checker()` is used.
    pub file_checker: Option<Arc<MockStrandFileChecker>>,
    /// Event dispatcher — present only when `.with_tracking_event_dispatcher()` is used.
    pub event_dispatcher: Option<Arc<MockEventDispatcher>>,
}

/// Builder for constructing [`ProcessStrand`] with all mock ports wired up.
///
/// Replaces the duplicated local `build_process_strand` functions that
/// existed in each integration test file. Use the fluent setters to
/// configure optional tracking ports and profile overrides:
///
/// ```ignore
/// let result = ProcessStrandBuilder::new(loom, runner).build();
/// let ProcessStrandResult { strand, log_events, .. } = result;
/// ```
///
/// ```ignore
/// let result = ProcessStrandBuilder::new(loom, runner)
///     .with_profile(custom_profile)
///     .with_tracking_git()
///     .build();
/// ```
pub struct ProcessStrandBuilder {
    /// Looms to register in the store (single loom by default).
    looms: Vec<Loom>,
    /// The mock agent runner.
    agent_runner: Arc<MockAgentRunner>,
    /// Custom profile override (uses `default_profile()` if `None`).
    profile: Option<AgentProfile>,
    /// Whether to create a tracking git port.
    tracking_git: bool,
    /// Whether to expose the tracking event dispatcher in the result.
    tracking_event_dispatcher: bool,
    /// Whether to expose the file checker in the result.
    tracking_file_checker: bool,
}

impl ProcessStrandBuilder {
    /// Create a new builder with a single loom and the given agent runner.
    pub fn new(loom: Loom, agent_runner: Arc<MockAgentRunner>) -> Self {
        Self {
            looms: vec![loom],
            agent_runner,
            profile: None,
            tracking_git: false,
            tracking_event_dispatcher: false,
            tracking_file_checker: false,
        }
    }

    /// Replace the single-loom with multiple looms.
    ///
    /// Used by tests that need event consumers in a different loom.
    pub fn with_looms(mut self, looms: Vec<Loom>) -> Self {
        self.looms = looms;
        self
    }

    /// Override the default agent profile.
    ///
    /// Used by tests that need a custom timeout or other profile settings.
    pub fn with_profile(mut self, profile: AgentProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Enable tracking of git versioning calls.
    ///
    /// Returns `git_port` and `git_commits` in the result.
    pub fn with_tracking_git(mut self) -> Self {
        self.tracking_git = true;
        self
    }

    /// Enable tracking of event dispatch calls.
    ///
    /// Returns `event_dispatcher` in the result.
    pub fn with_tracking_event_dispatcher(mut self) -> Self {
        self.tracking_event_dispatcher = true;
        self
    }

    /// Enable tracking of strand file checks.
    ///
    /// Returns `file_checker` in the result.
    pub fn with_tracking_file_checker(mut self) -> Self {
        self.tracking_file_checker = true;
        self
    }

    /// Build the [`ProcessStrand`] use case with all mocked ports.
    pub fn build(self) -> ProcessStrandResult {
        let store = LoomStore::new();
        for loom in &self.looms {
            store.register(loom.clone());
        }

        let (log_port, log_events) = MockLoomLogPort::new();
        let (tie_off_sink, tie_off_appends, tie_off_content) =
            TrackingTieOffSink::new();
        let (rig_log, rig_events) = MockRigLogPort::new();

        let profile = self.profile.unwrap_or_else(default_profile);
        let profile_repo = Arc::new(MockProfileRepository {
            profiles: Arc::new(Mutex::new(HashMap::from_iter([
                ("fast".to_string(), profile.clone()),
            ]))),
        });

        let git_port: Arc<dyn knot::application::ports::GitVersioningPort>;
        let git_port_concrete: Option<Arc<MockGitVersioningPort>>;
        let git_commits: Option<Arc<Mutex<Vec<(knot::domain::entities::LoomId, knot::domain::entities::KnotId, String, String, String)>>>>;

        if self.tracking_git {
            let (gp, gc) = MockGitVersioningPort::new();
            git_port_concrete = Some(Arc::new(gp));
            git_commits = Some(gc);
            git_port = git_port_concrete.clone().unwrap();
        } else {
            git_port_concrete = None;
            git_commits = None;
            git_port = Arc::new(MockGitVersioningPort::default());
        }

        let file_checker: Arc<dyn knot::domain::entities::StrandFileChecker>;
        let file_checker_concrete: Option<Arc<MockStrandFileChecker>>;

        if self.tracking_file_checker {
            file_checker_concrete = Some(Arc::new(MockStrandFileChecker::new()));
            file_checker = file_checker_concrete.clone().unwrap();
        } else {
            file_checker_concrete = None;
            file_checker = Arc::new(MockStrandFileChecker::new());
        }

        let event_dispatcher: Arc<dyn knot::application::ports::EventDispatcherPort>;
        let event_dispatcher_concrete: Option<Arc<MockEventDispatcher>>;

        if self.tracking_event_dispatcher {
            event_dispatcher_concrete = Some(Arc::new(MockEventDispatcher::default()));
            event_dispatcher = event_dispatcher_concrete.clone().unwrap();
        } else {
            event_dispatcher_concrete = None;
            event_dispatcher = Arc::new(MockEventDispatcher::default());
        }

        let strand = ProcessStrand::new(
            store.clone(),
            Arc::new(log_port),
            self.agent_runner.clone() as Arc<dyn AgentRunner>,
            Arc::new(tie_off_sink),
            RigAgentConfig::default_config(),
            PathBuf::from("/rig"),
            profile_repo,
            Arc::new(rig_log),
            git_port,
            file_checker,
            event_dispatcher,
            None,
        );

        ProcessStrandResult {
            strand,
            log_events,
            tie_off_appends,
            rig_events,
            tie_off_content,
            agent_runner: self.agent_runner,
            git_port: git_port_concrete,
            git_commits,
            file_checker: file_checker_concrete,
            event_dispatcher: event_dispatcher_concrete,
        }
    }
}

// ── Knot Content Fixtures ──────────────────────────────────────────────────

/// Create knot definition YAML frontmatter and body for a knot file.
///
/// Writes a valid knot `.md` file with the given name, profile reference,
/// and strand directory.
///
/// # Arguments
///
/// * `name` - The knot identifier (used in YAML `name` field)
/// * `agent_profile_ref` - Profile name to reference (e.g. "fast")
/// * `strand_dir` - Relative path to the strand source directory
pub fn make_knot_content(
    name: &str,
    agent_profile_ref: &str,
    strand_dir: &str,
) -> String {
    [
        "---",
        &format!("name: {name}"),
        &format!("agent-profile-ref: {agent_profile_ref}"),
        &format!("strand-dir: \"{strand_dir}\""),
        "git-versioned: false",
        "---",
        "",
        &format!("Test knot: {name}."),
        "",
    ].join("\n")
}

/// Create a knot definition file inside a loom directory.
///
/// Creates the loom directory if it doesn't exist.
///
/// # Arguments
///
/// * `loom_dir` - Path to the `*-loom` directory
/// * `name` - The knot identifier
pub fn create_knot_file(loom_dir: &Path, name: &str) {
    fs::create_dir_all(loom_dir).unwrap_or_else(|e| {
        panic!("failed to create loom dir {}: {}", loom_dir.display(), e)
    });
    let content = make_knot_content(name, "fast", "./strands");
    fs::write(loom_dir.join(format!("{name}.md")), content).unwrap_or_else(
        |e| {
            panic!(
                "failed to write knot file {}: {}",
                loom_dir.join(format!("{name}.md")).display(),
                e
            )
        },
    );
}

// ── Profile Fixtures ──────────────────────────────────────────────────────

/// Create a "fast" agent profile in a rig's profiles directory.
///
/// Writes `profiles/fast.md` with minimal OpenAI gpt-4o configuration.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory (e.g. `./dev-rig`)
pub fn create_fast_profile(rig_dir: &Path) {
    let profiles_dir = rig_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap_or_else(|e| {
        panic!(
            "failed to create profiles dir {}: {}",
            profiles_dir.display(),
            e
        )
    });
    fs::write(
        profiles_dir.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\n---\n\n\
You are a reviewer.\n",
    )
    .unwrap_or_else(|e| {
        panic!("failed to write fast profile: {}", e)
    });
}



// ── Knot Server Helpers ───────────────────────────────────────────────────

/// Handle for a background Knot process.
///
/// Signals the Knot runtime to shut down on drop.
#[derive(Debug)]
pub struct KnotHandle {
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl KnotHandle {
    /// Abort the Knot task and wait for the thread to finish.
    pub fn abort(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(th) = self.thread.take() {
            let _ = th.join();
        }
    }
}

impl Drop for KnotHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Fast debounce timing for integration tests.
///
/// Reduces the 100ms debounce window and 5ms check interval to
/// 20ms / 2ms, cutting per-event wait time from ~105ms to ~22ms.
/// With multiple events per test, this saves several seconds.
const TEST_DEBOUNCE_MS: u64 = 20;
const TEST_CHECK_MS: u64 = 2;

/// Start Knot in a background thread with a custom `AppConfig`.
///
/// Spawns `knot::start_knot(config)` in its own `tokio::runtime::Runtime`
/// on a dedicated OS thread. Returns a `KnotHandle` that signals the
/// thread to shut down on drop.
///
/// Sets `KNOT_TEST_DEBOUNCE_MS` and `KNOT_TEST_CHECK_MS` env vars
/// so the debounce engine runs with fast (20ms/2ms) timing instead
/// of the production defaults (100ms/5ms).
///
/// This allows integration tests to verify file-based state (reading
/// `rig/state.json`) without needing an HTTP server.
///
/// # Arguments
///
/// * `config` - Full `AppConfig` (rig dir, rig config, agent timeout,
///   and optionally `cli_path` for injecting a mock agent binary)
///
/// # Returns
///
/// A `KnotHandle` for cleanup.
pub fn start_knot_with_config(config: knot::AppConfig) -> KnotHandle {
    // Set fast debounce timing — env vars are process-global and
    // read by the server at debounce engine startup. Only affects
    // this test binary (integration test), not unit tests.
    unsafe {
        std::env::set_var("KNOT_TEST_DEBOUNCE_MS", TEST_DEBOUNCE_MS.to_string());
        std::env::set_var("KNOT_TEST_CHECK_MS", TEST_CHECK_MS.to_string());
    }

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let thread = thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new()
            .expect("should create tokio runtime");

        rt.block_on(async {
            let task = rt.spawn(async move {
                let _ = knot::start_knot(config).await;
            });

            // Wait for shutdown signal, then abort the task.
            // The task blocks on Ctrl+C which never fires in tests,
            // so we need to explicitly abort it.
            if shutdown_rx.await.is_ok() {
                task.abort();
                // Await the handle so block_on can return.
                // Aborted handles return JoinError immediately.
                let _ = task.await;
            } else {
                // Sender dropped without sending — abort anyway
                task.abort();
                let _ = task.await;
            }
        });
    });

    KnotHandle {
        shutdown_tx: Some(shutdown_tx),
        thread: Some(thread),
    }
}

// ── State File Polling Helpers ────────────────────────────────────────────

/// Read and parse `rig/state.json` from the given rig directory.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
///
/// # Returns
///
/// Parsed `serde_json::Value`, or `Err` if the file doesn't exist
/// or isn't valid JSON.
pub fn read_state_file(rig_dir: &Path) -> Result<Value, std::io::Error> {
    let state_path = rig_dir.join("state.json");
    let content = fs::read_to_string(&state_path).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!(
                "failed to read {}: {}",
                state_path.display(),
                e
            ),
        )
    })?;
    serde_json::from_str(&content).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("failed to parse state.json: {}", e),
        )
    })
}



/// Poll `rig/state.json` until a loom with the given ID appears
/// with the expected number of knots.
///
/// Polls every 200ms with a 30-second timeout.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `loom_id` - The loom's ID (without the `-loom` suffix)
/// * `expected_knots` - Expected number of knots in the loom
///
/// # Panics
///
/// Panics if the loom is not found within the timeout.
pub fn wait_for_loom_in_state(
    rig_dir: &Path,
    loom_id: &str,
    expected_knots: usize,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);

    loop {
        if std::time::Instant::now() > deadline {
            let state = read_state_file(rig_dir)
                .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
                .unwrap_or_else(|_| "state.json not found".to_string());

            panic!(
                "timeout waiting for loom '{}' in state.json\n\
                 expected_knots: {}\n\
                 state:\n{}",
                loom_id,
                expected_knots,
                state
            );
        }

        match read_state_file(rig_dir) {
            Ok(state) => {
                if let Some(looms) = state.get("looms").and_then(|v| v.as_array())
                {
                    for loom in looms {
                        if let Some(id) = loom.get("id").and_then(|v| v.as_str()) {
                            if id == loom_id {
                                let knot_count = loom
                                    .get("knots")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| arr.len())
                                    .unwrap_or(0);

                                if knot_count == expected_knots {
                                    return;
                                }

                                // Loom found but wrong knot count — keep polling
                                // (knots may still be being discovered)
                                break;
                            }
                        }
                    }
                }
            }
            Err(_) => {
                // File not ready yet
            }
        }

        thread::sleep(Duration::from_millis(50));
    }
}

/// Poll `rig/state.json` until a knot reaches the expected status.
///
/// Searches for the knot within the given loom by ID, then checks
/// its `status` field.
///
/// Polls every 200ms with a 30-second timeout.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `loom_id` - The loom's ID
/// * `knot_id` - The knot's ID
/// * `status` - Expected status string (e.g. "idle", "processing", "completed")
///
/// # Panics
///
/// Panics if the knot status is not found within the timeout.
pub fn wait_for_knot_status_in_state(
    rig_dir: &Path,
    loom_id: &str,
    knot_id: &str,
    status: &str,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);

    loop {
        if std::time::Instant::now() > deadline {
            let state = read_state_file(rig_dir)
                .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
                .unwrap_or_else(|_| "state.json not found".to_string());

            panic!(
                "timeout waiting for knot '{}' (loom '{}') status '{}'\n\
                 state:\n{}",
                knot_id,
                loom_id,
                status,
                state
            );
        }

        match read_state_file(rig_dir) {
            Ok(state) => {
                if let Some(looms) = state.get("looms").and_then(|v| v.as_array())
                {
                    for loom in looms {
                        if let Some(id) = loom.get("id").and_then(|v| v.as_str()) {
                            if id == loom_id {
                                if let Some(knots) =
                                    loom.get("knots").and_then(|v| v.as_array())
                                {
                                    for knot in knots {
                                        if let (Some(kid), Some(kstatus)) = (
                                            knot.get("id").and_then(|v| v.as_str()),
                                            knot.get("status")
                                                .and_then(|v| v.as_str()),
                                        ) {
                                            if kid == knot_id
                                                && kstatus == status
                                            {
                                                return;
                                            }
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
            Err(_) => {
                // File not ready yet
            }
        }

        thread::sleep(Duration::from_millis(50));
    }
}

// ── Loom Directory Helpers ────────────────────────────────────────────────

/// Create a loom directory inside a rig.
///
/// Creates `{rig_dir}/{name}-loom/`.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `name` - Loom name (the `-loom` suffix is added automatically)
///
/// # Returns
///
/// Path to the created loom directory.
pub fn create_loom_dir(
    rig_dir: &Path,
    name: &str,
) -> PathBuf {
    let loom_path = rig_dir.join(format!("{name}-loom"));
    fs::create_dir_all(&loom_path).unwrap_or_else(|e| {
        panic!(
            "failed to create loom dir {}: {}",
            loom_path.display(),
            e
        )
    });
    loom_path
}

/// Create a strands directory in the project root and write a strand file.
///
/// The project root is the parent of `rig_dir`. This matches how Knot
/// resolves `strand_dir: "./strands"` — relative to the project root,
/// not the rig directory.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory (strands dir created at
///   `{rig_dir}/../strands` i.e. project root)
/// * `strand_name` - Filename for the strand (e.g. "feature.md")
/// * `content` - Content to write into the strand file
///
/// # Returns
///
/// Path to the created strand file.
pub fn create_strand(
    rig_dir: &Path,
    strand_name: &str,
    content: &str,
) -> PathBuf {
    // strand_dir is resolved relative to project root (parent of rig_dir)
    let project_root = rig_dir
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| rig_dir.to_path_buf());
    let strands_dir = project_root.join("strands");
    fs::create_dir_all(&strands_dir).unwrap();
    let path = strands_dir.join(strand_name);
    fs::write(&path, content).unwrap();
    path
}

// ── Loom Log Helpers ──────────────────────────────────────────────────────

/// Read all events from a loom's activity log.
///
/// Reads `{rig_dir}/tie-offs/{loom_id}/.loom-log` as JSONL and returns
/// each line as a parsed JSON value.
///
/// The loom-log lives under `tie-offs/` (not in the loom directory itself).
/// The `loom_id` parameter should include the `-loom` suffix
/// (e.g. `"review-loom"`), matching the loom ID stored in state.json.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `loom_id` - The loom's ID (including `-loom` suffix, e.g. "review-loom")
///
/// # Returns
///
/// Vector of parsed JSON values, one per log entry.
pub fn read_loom_log(
    rig_dir: &Path,
    loom_id: &str,
) -> Vec<Value> {
    let log_path = rig_dir.join("tie-offs").join(loom_id).join(".loom-log");
    let content = match fs::read_to_string(&log_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Extract the event type from a loom-log JSON entry.
///
/// Loom-log entries are stored as JSON objects with a single key
/// that is the event variant name (e.g. `{"KnotCompleted":{...}}`).
/// This function extracts that variant key.
///
/// # Arguments
///
/// * `event` - Parsed JSON value from a loom-log line
///
/// # Returns
///
/// `Some("KnotCompleted")` etc., or `None` if not an object.
pub fn loom_log_event_type(event: &Value) -> Option<&str> {
    event.as_object().and_then(|obj| {
        obj.keys().next().map(|k| k.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_knot_content_has_valid_yaml_frontmatter() {
        let content = make_knot_content("review", "fast", "./strands");
        assert!(content.starts_with("---"));
        assert!(content.contains("name: review"));
        assert!(content.contains("agent-profile-ref: fast"));
        assert!(content.contains("strand-dir: \"./strands\""));
        assert!(!content.contains("prompt-template:"), "should not have prompt-template in frontmatter");
        assert!(content.contains("Test knot: review."));
    }

    #[test]
    fn create_fast_profile_writes_valid_file() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_dir = tmp.path();

        create_fast_profile(rig_dir);

        let profile_path = rig_dir.join("profiles/fast.md");
        assert!(profile_path.exists());

        let content = fs::read_to_string(&profile_path).unwrap();
        assert!(content.contains("name: fast"));
        assert!(content.contains("provider: openai"));
        assert!(content.contains("model: gpt-4o"));
    }

    #[test]
    fn create_loom_dir_creates_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_dir = tmp.path();

        let loom_path = create_loom_dir(rig_dir, "test");

        assert!(loom_path.exists());
        assert!(loom_path.is_dir());
        assert_eq!(
            loom_path.file_name().unwrap(),
            "test-loom"
        );
    }

    #[test]
    fn create_knot_file_creates_markdown_file() {
        let tmp = tempfile::tempdir().unwrap();
        let loom_dir = tmp.path().join("test-loom");
        fs::create_dir_all(&loom_dir).unwrap();

        create_knot_file(&loom_dir, "review");

        let knot_path = loom_dir.join("review.md");
        assert!(knot_path.exists());
        let content = fs::read_to_string(&knot_path).unwrap();
        assert!(content.contains("name: review"));
    }

    #[test]
    fn create_strand_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_dir = tmp.path();

        let path = create_strand(rig_dir, "feature.md", "new feature");

        assert!(path.exists());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "new feature"
        );
        // strands dir is in project root (parent of rig_dir)
        let project_root = rig_dir.parent().unwrap();
        assert!(project_root.join("strands").is_dir());
    }

    #[test]
    fn read_state_file_returns_error_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = read_state_file(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn read_state_file_parses_valid_json() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_dir = tmp.path();

        fs::write(
            rig_dir.join("state.json"),
            r#"{"rig_path":"/test","looms":[],"profiles":[],"updated_at":"now"}"#,
        )
        .unwrap();

        let state = read_state_file(rig_dir).unwrap();
        assert_eq!(
            state.get("rig_path").and_then(|v| v.as_str()),
            Some("/test")
        );
    }

    #[test]
    fn read_loom_log_returns_empty_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_dir = tmp.path();

        let events = read_loom_log(rig_dir, "test");
        assert!(events.is_empty());
    }

    #[test]
    fn read_loom_log_parses_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_dir = tmp.path();
        // loom-log lives at rig/tie-offs/{loom_id}/.loom-log
        let log_dir = rig_dir.join("tie-offs/test-loom");
        fs::create_dir_all(&log_dir).unwrap();

        // Events are stored as JSON with variant name as top-level key
        fs::write(
            log_dir.join(".loom-log"),
            r#"{"LoomStarted":{"loom_id":"test-loom","timestamp":"2026-01-01T00:00:00Z"}}
{"KnotRegistered":{"loom_id":"test-loom","knot_id":"k1","timestamp":"2026-01-01T00:00:01Z"}}
"#,
        )
        .unwrap();

        let events = read_loom_log(rig_dir, "test-loom");
        assert_eq!(events.len(), 2);
        assert_eq!(
            loom_log_event_type(&events[0]),
            Some("LoomStarted")
        );
    }
}
