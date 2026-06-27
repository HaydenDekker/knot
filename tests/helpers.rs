//! Shared test helpers for Knot integration tests.
//!
//! Provides file-based polling helpers to verify rig state via
//! `rig/state.json`, replacing the previous HTTP-based verification.
//! Also includes fixtures for creating knots, profiles, mock agents,
//! and git repository setup.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde_json::Value;

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

/// Create an agent profile with custom settings.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `name` - Profile name (written as `profiles/{name}.md`)
/// * `provider` - LLM provider identifier
/// * `model` - Model name
/// * `prompt` - Profile-level system prompt
pub fn create_agent_profile(
    rig_dir: &Path,
    name: &str,
    provider: &str,
    model: &str,
    prompt: &str,
) {
    let profiles_dir = rig_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    fs::write(
        profiles_dir.join(format!("{name}.md")),
        format!(
            "---\nname: {name}\nprovider: {provider}\nmodel: {model}\n---\n\n\
{prompt}\n"
        ),
    )
    .unwrap();
}

// ── Mock Agent Fixtures ───────────────────────────────────────────────────

/// Create a mock agent script that writes deterministic output.
///
/// Creates a shell script at `rig/mock-agent` that echoes a fixed
/// response and exits with code 0. Used to simulate agent execution
/// without requiring `pi` or any external CLI.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `response` - The stdout output the mock agent should produce
///
/// # Returns
///
/// Path to the created mock agent script.
pub fn create_mock_agent(rig_dir: &Path, response: &str) -> PathBuf {
    let agent_path = rig_dir.join("mock-agent");
    let script = format!(
        "#!/usr/bin/env bash\n\
         echo \"{response}\"\n\
         exit 0\n"
    );
    fs::write(&agent_path, script).unwrap_or_else(|e| {
        panic!("failed to write mock agent: {}", e)
    });
    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&agent_path, fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|e| {
                panic!("failed to set mock agent permissions: {}", e)
            });
    }
    agent_path
}

/// Create a stub `pi` agent that simulates the pi CLI interface.
///
/// Creates a shell script at `rig/stub-pi` that accepts arguments
/// and produces deterministic output. The stub reads stdin and echoes
/// a fixed response.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `response` - The response the stub should echo
///
/// # Returns
///
/// Path to the created stub agent script.
pub fn create_stub_pi_agent(rig_dir: &Path, response: &str) -> PathBuf {
    let agent_path = rig_dir.join("stub-pi");
    let script = format!(
        "#!/usr/bin/env bash\n\
         # Stub pi agent - ignores all arguments\n\
         cat > /dev/null\n\
         echo \"{response}\"\n\
         exit 0\n"
    );
    fs::write(&agent_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&agent_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    agent_path
}

/// Create a workspace agent config that points to a stub `pi` binary.
///
/// Creates a stub `pi` script at `{rig_dir}/bin/pi` and writes
/// `.workspace-agent-config.yaml` so Knot uses it as the agent CLI.
/// The stub reads stdin (discards it) and echoes the given response.
///
/// This is the preferred way to test agent integration — it wires
/// the real `SubprocessAgentRunner` with a deterministic mock.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `response` - The stdout output the stub should produce
///
/// # Returns
///
/// Path to the created stub `pi` binary.
pub fn create_mock_pi(rig_dir: &Path, response: &str) -> PathBuf {
    // Create bin directory and stub pi script
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script = format!(
        "#!/usr/bin/env bash\n\
         # Stub pi for Knot tests - consumes stdin, echoes response\n\
         cat > /dev/null\n\
         echo \"{response}\"\n\
         exit 0\n"
    );
    fs::write(&pi_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    // Write workspace agent config — adapter hardcodes binary path,
    // so config only selects adapter. Stub is found via PATH.
    let config = "agent-adapter: pi-stdio\n";
    fs::write(rig_dir.join(".workspace-agent-config.yaml"), config).unwrap();

    // Prepend bin dir to PATH so the stub is discoverable as "pi".
    // Unsafe: set_var is unsafe due to global mutable state,
    // but this is a single-threaded test context.
    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), existing));
    }

    pi_path
}

/// Create a workspace agent config pointing to a stub `pi` binary that
/// captures stdin to a file for inspection.
///
/// Creates a stub `pi` script at `{rig_dir}/bin/pi` that writes all stdin
/// content to the given capture path before echoing the response. This
/// allows integration tests to verify what prompt the agent received.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `response` - The stdout output the stub should produce
/// * `capture_path` - Path where stdin will be written for inspection
///
/// # Returns
///
/// Path to the created stub `pi` binary.
pub fn create_mock_pi_capturing_stdin(
    rig_dir: &Path,
    response: &str,
    capture_path: &Path,
) -> PathBuf {
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    // Capture stdin to file, then echo response.
    // Multiple invocations append so each agent run's prompt is preserved.
    let script = format!(
        "#!/usr/bin/env bash\n\
         # Stub pi - captures stdin to file, echoes response\n\
         cat > \"{capture_path}\"\n\
         echo \"{response}\"\n\
         exit 0\n",
        capture_path = capture_path.display(),
    );
    fs::write(&pi_path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    let config = "agent-adapter: pi-stdio\n";
    fs::write(rig_dir.join(".workspace-agent-config.yaml"), config).unwrap();

    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), existing));
    }

    pi_path
}

// ── Git Repository Helpers ─────────────────────────────────────────────────

/// Initialize a git repository in the given directory.
///
/// Creates `.git`, configures user.name and user.email, and creates
/// an initial empty commit.
///
/// # Arguments
///
/// * `path` - Path to the directory to initialize as a git repo
pub fn init_git_repo(path: &Path) {
    run_git(path, &["init"]);
    run_git(path, &["config", "user.name", "Test"]);
    run_git(path, &["config", "user.email", "test@knot.local"]);

    // Create an initial commit
    let initial_file = path.join(".gitkeep");
    fs::write(&initial_file, "").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "Initial commit"]);
}

/// Get the hash of the latest commit in a git repository.
///
/// # Arguments
///
/// * `path` - Path to the git repository
///
/// # Returns
///
/// The full commit hash as a `String`.
pub fn get_latest_commit(path: &Path) -> String {
    let output = Command::new("git")
        .current_dir(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("should run git rev-parse")
        .stdout;
    String::from_utf8(output).expect("valid utf8").trim().to_string()
}

/// Count the number of commits in a git repository.
///
/// # Arguments
///
/// * `path` - Path to the git repository
///
/// # Returns
///
/// Number of commits as `usize`.
pub fn count_commits(path: &Path) -> usize {
    let output = Command::new("git")
        .current_dir(path)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .expect("should run git rev-list")
        .stdout;
    String::from_utf8(output)
        .expect("valid utf8")
        .trim()
        .parse()
        .expect("should parse commit count")
}

/// Run a git command in the given directory.
///
/// # Arguments
///
/// * `path` - Path to the git repository (used as current_dir)
/// * `args` - Git subcommand and arguments (e.g. `["add", "."]`)
fn run_git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .expect("should run git command");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "git {} failed: {}",
            args.join(" "),
            stderr.trim()
        );
    }
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

/// Start Knot in a background thread.
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
/// * `rig_dir` - Path to the rig directory
///
/// # Returns
///
/// A `KnotHandle` for cleanup.
pub fn start_knot(rig_dir: PathBuf) -> KnotHandle {
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
                let config = knot::AppConfig::with_rig_dir(rig_dir);
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

/// Poll `rig/state.json` until a JSON field matches the expected value.
///
/// Uses dot-notation selectors to navigate the JSON structure.
/// Array indices are supported (e.g. `"looms.0.id"` accesses
/// the `id` field of the first loom).
///
/// Polls every 200ms with a 30-second timeout.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `selector` - Dot-notation JSON path (e.g. `"looms.0.id"`)
/// * `expected` - Expected string value
///
/// # Panics
///
/// Panics if the field is not found within the timeout.
pub fn wait_for_state_field(
    rig_dir: &Path,
    selector: &str,
    expected: &str,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);

    loop {
        if std::time::Instant::now() > deadline {
            panic!(
                "timeout waiting for state field '{}' == '{}'\n\
                 selector: {}\n\
                 state_path: {}",
                selector,
                expected,
                selector,
                rig_dir.join("state.json").display()
            );
        }

        match read_state_file(rig_dir) {
            Ok(state) => {
                let value = resolve_selector(&state, selector);
                if let Some(val) = value {
                    if let Some(str_val) = val.as_str() {
                        if str_val == expected {
                            return;
                        }
                    } else if let Some(num_val) = val.as_i64() {
                        if num_val.to_string() == expected {
                            return;
                        }
                    }
                }
            }
            Err(_) => {
                // File not ready yet, keep polling
            }
        }

        thread::sleep(Duration::from_millis(50));
    }
}

/// Resolve a dot-notation selector against a JSON value.
///
/// Supports object keys and array indices (numeric strings).
///
/// # Arguments
///
/// * `root` - Root JSON value
/// * `selector` - Dot-notation path (e.g. `"looms.0.knots.1.status"`)
///
/// # Returns
///
/// `Some(&Value)` if the path resolves, `None` otherwise.
fn resolve_selector<'a>(
    root: &'a Value,
    selector: &str,
) -> Option<&'a Value> {
    let mut current = root;
    for part in selector.split('.') {
        current = if let Ok(idx) = part.parse::<usize>() {
            current.get(idx)
        } else {
            current.get(part)
        }?;
    }
    Some(current)
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

/// Poll `rig/state.json` until it exists and is valid JSON.
///
/// The state writer writes immediately on startup, so this typically
/// returns within a few seconds.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
///
/// # Returns
///
/// The parsed `serde_json::Value`.
///
/// # Panics
///
/// Panics if the state file is not found within 15 seconds.
pub fn wait_for_state_file(rig_dir: &Path) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);

    loop {
        if std::time::Instant::now() > deadline {
            panic!(
                "timeout waiting for state.json at {}",
                rig_dir.join("state.json").display()
            );
        }

        match read_state_file(rig_dir) {
            Ok(state) => return state,
            Err(_) => {
                thread::sleep(Duration::from_millis(50));
            }
        }
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

/// Extract the inner object from a loom-log JSON entry.
///
/// Loom-log entries are stored as externally tagged JSON with a single
/// key that is the event variant name (e.g. `{"KnotCompleted":{...}}`).
/// This returns the inner object containing the actual fields
/// (`knot_id`, `strand_path`, etc.).
///
/// # Arguments
///
/// * `event` - Parsed JSON value from a loom-log line
///
/// # Returns
///
/// `Some(&Value)` of the inner object, or `None` if not an object.
pub fn loom_log_event_inner<'a>(event: &'a Value) -> Option<&'a Value> {
    event.as_object().and_then(|obj| obj.values().next())
}

/// Poll until a loom-log contains an event with a specific type.
///
/// The loom-log stores events as JSON objects keyed by variant name
/// (e.g. `{"KnotCompleted":{...}}`). This function checks the top-level
/// key of each entry.
///
/// # Arguments
///
/// * `rig_dir` - Path to the rig directory
/// * `loom_id` - The loom's ID
/// * `event_type` - Expected event type string (e.g. "KnotCompleted")
///
/// # Panics
///
/// Panics if the event type is not found within 15 seconds.
pub fn wait_for_loom_log_event(
    rig_dir: &Path,
    loom_id: &str,
    event_type: &str,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);

    loop {
        if std::time::Instant::now() > deadline {
            panic!(
                "timeout waiting for loom-log event '{}' in loom '{}'",
                event_type, loom_id
            );
        }

        let events = read_loom_log(rig_dir, loom_id);
        for event in &events {
            if let Some(ty) = loom_log_event_type(event) {
                if ty == event_type {
                    return;
                }
            }
        }

        thread::sleep(Duration::from_millis(50));
    }
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
    fn create_agent_profile_writes_custom_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_dir = tmp.path();

        create_agent_profile(
            rig_dir, "detailed", "anthropic", "claude-sonnet",
            "You are a detailed reviewer.",
        );

        let profile_path = rig_dir.join("profiles/detailed.md");
        assert!(profile_path.exists());

        let content = fs::read_to_string(&profile_path).unwrap();
        assert!(content.contains("name: detailed"));
        assert!(content.contains("provider: anthropic"));
    }

    #[test]
    fn create_mock_agent_creates_executable_script() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_dir = tmp.path();

        let path = create_mock_agent(rig_dir, "mock response");

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("mock response"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&path).unwrap().permissions();
            assert!(perms.mode() & 0o111 != 0, "should be executable");
        }
    }

    #[test]
    fn create_stub_pi_agent_creates_executable_script() {
        let tmp = tempfile::tempdir().unwrap();
        let rig_dir = tmp.path();

        let path = create_stub_pi_agent(rig_dir, "stub response");

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("stub response"));
    }

    #[test]
    fn resolve_selector_nested_object() {
        let json: Value = serde_json::json!({
            "looms": [
                {
                    "id": "test",
                    "knots": [
                        {"id": "k1", "status": "idle"}
                    ]
                }
            ]
        });

        assert_eq!(
            resolve_selector(&json, "looms.0.id")
                .and_then(|v| v.as_str()),
            Some("test")
        );
        assert_eq!(
            resolve_selector(&json, "looms.0.knots.0.status")
                .and_then(|v| v.as_str()),
            Some("idle")
        );
    }

    #[test]
    fn resolve_selector_missing_path_returns_none() {
        let json: Value = serde_json::json!({
            "looms": []
        });

        assert!(resolve_selector(&json, "looms.0.id").is_none());
        assert!(resolve_selector(&json, "missing.field").is_none());
    }

    #[test]
    fn resolve_selector_numeric_index() {
        let json: Value = serde_json::json!([10, 20, 30]);

        assert_eq!(
            resolve_selector(&json, "0").and_then(|v| v.as_i64()),
            Some(10)
        );
        assert_eq!(
            resolve_selector(&json, "2").and_then(|v| v.as_i64()),
            Some(30)
        );
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
