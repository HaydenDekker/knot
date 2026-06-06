//! Integration tests for the full Knot application.
//!
//! These tests spin up the actual server and verify end-to-end behaviour.

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;


/// Two looms with different source dirs and tie-off points.
///
/// 1. Create strand in loom A → tie-off in A's point only
/// 2. Create strand in loom B → tie-off in B's point only
/// 3. No cross-interference (A's knots don't process B's strands)
#[test]
fn multiple_looms_independent() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Loom A with its own strand and tie-off directories
    let loom_a_dir = base_dir.join("loom-a-loom");
    fs::create_dir(&loom_a_dir).unwrap();
    let strand_dir_a = base_dir.join("loom-a-strands");
    let tie_off_dir_a = base_dir.join("loom-a-tieoffs");
    fs::create_dir_all(&strand_dir_a).unwrap();
    fs::create_dir_all(&tie_off_dir_a).unwrap();
    let knot_a_content = format!(
        "---\nname: review-knot\nagent-config:\n  goal: \"Review A\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nstrand-dir: \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review A's documents.\n---\n",
        strand_dir_a.display(),
        tie_off_dir_a.display()
    );
    fs::write(loom_a_dir.join("review.md"), knot_a_content).unwrap();

    // Loom B with its own strand and tie-off directories
    let loom_b_dir = base_dir.join("loom-b-loom");
    fs::create_dir(&loom_b_dir).unwrap();
    let strand_dir_b = base_dir.join("loom-b-strands");
    let tie_off_dir_b = base_dir.join("loom-b-tieoffs");
    fs::create_dir_all(&strand_dir_b).unwrap();
    fs::create_dir_all(&tie_off_dir_b).unwrap();
    let knot_b_content = format!(
        "---\nname: review-knot\nagent-config:\n  goal: \"Review B\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nstrand-dir: \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review B's documents.\n---\n",
        strand_dir_b.display(),
        tie_off_dir_b.display()
    );
    fs::write(loom_b_dir.join("review.md"), knot_b_content).unwrap();

    let port = 31994;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: "sh".to_string(),
            cli_args: vec![
                "-c".to_string(),
                "echo 'processed'".to_string(),
            ],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify both looms are registered
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 2, "should have 2 looms");

    // Collect loom IDs from the response
    let loom_ids: Vec<_> =
        summaries.iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(
        loom_ids.contains(&"loom-a-loom"),
        "loom-a-loom should be registered"
    );
    assert!(
        loom_ids.contains(&"loom-b-loom"),
        "loom-b-loom should be registered"
    );

    // 1. Create strand in loom A
    let strand_a_path = strand_dir_a.join("strand-a.md");
    fs::write(&strand_a_path, "content for A").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Tie-off appears only in A's output directory
    let tie_off_a = tie_off_dir_a.join("strand-a.md.output");
    assert!(
        tie_off_a.exists(),
        "tie-off should exist in loom A: {}",
        tie_off_a.display()
    );

    // 2. Create strand in loom B
    let strand_b_path = strand_dir_b.join("strand-b.md");
    fs::write(&strand_b_path, "content for B").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Tie-off appears only in B's output directory
    let tie_off_b = tie_off_dir_b.join("strand-b.md.output");
    assert!(
        tie_off_b.exists(),
        "tie-off should exist in loom B: {}",
        tie_off_b.display()
    );

    // 3. No cross-interference
    // A's tie-off dir should NOT contain B's strand output
    let files_in_a: Vec<_> =
        fs::read_dir(&tie_off_dir_a)
            .expect("should read tie-off dir A")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    assert!(
        !files_in_a.iter().any(|f| f.contains("strand-b")),
        "loom A should not contain loom B's strand output, got {files_in_a:?}"
    );

    // B's tie-off dir should NOT contain A's strand output
    let files_in_b: Vec<_> =
        fs::read_dir(&tie_off_dir_b)
            .expect("should read tie-off dir B")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    assert!(
        !files_in_b.iter().any(|f| f.contains("strand-a")),
        "loom B should not contain loom A's strand output, got {files_in_b:?}"
    );

    let _ = shutdown.send(());
}

// ── Phase 4: Graceful Shutdown ────────────────────────────────────────────

/// Send shutdown signal; file watcher stops, no new events are processed
/// after shutdown.
///
/// Verified by:
/// 1. Starting server with a loom
/// 2. Sending shutdown signal
/// 3. Waiting briefly for shutdown to complete
/// 4. Creating a new strand file — should NOT produce a tie-off
/// 5. Confirming the tie-off file does NOT exist
#[test]
fn graceful_shutdown_stops_watchers() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("shutdown-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let port = 31995;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: "sh".to_string(),
            cli_args: vec![
                "-c".to_string(),
                "echo 'processed'".to_string(),
            ],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify server is healthy before shutdown
    let (status, _) =
        http_get(&host_port, "/health").expect("health should respond");
    assert!(status.contains("200"), "server should be healthy");

    // Send shutdown signal
    let _ = shutdown.send(());

    // Give shutdown time to complete (drop watcher, drain pipeline)
    std::thread::sleep(Duration::from_millis(1000));

    // Create a strand file AFTER shutdown — should NOT be processed
    let strand_path = strand_dir.join("post-shutdown-strand.md");
    fs::write(&strand_path, "this should not be processed").unwrap();

    // Wait a bit to confirm no processing happens
    std::thread::sleep(Duration::from_millis(500));

    // Tie-off file should NOT exist (watcher was stopped)
    let tie_off_path =
        tie_off_dir.join("post-shutdown-strand.md.output");
    assert!(
        !tie_off_path.exists(),
        "tie-off should NOT exist after shutdown: {}",
        tie_off_path.display()
    );
}

/// Shutdown writes `LoomStopped` to each loom's activity log.
///
/// Verified by:
/// 1. Starting server with a loom
/// 2. Sending shutdown signal
/// 3. Reading the loom-log file
/// 4. Confirming it contains `LoomStopped` event
#[test]
fn shutdown_logs_loom_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("log-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let port = 31996;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir,
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);

    // Wait for server to start listening
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify server is healthy before shutdown
    let (status, _) =
        http_get(&host_port, "/health").expect("health should respond");
    assert!(status.contains("200"), "server should be healthy");

    // Send shutdown signal
    let _ = shutdown.send(());

    // Give shutdown time to complete (including LoomStopped log write)
    std::thread::sleep(Duration::from_millis(1000));

    // Read the loom-log file
    let log_file = loom_dir.join(".loom-log");
    assert!(
        log_file.exists(),
        "loom log file should exist: {}",
        log_file.display()
    );

    let log_content =
        fs::read_to_string(&log_file).expect("should read log file");

    // Verify log contains LoomStopped entry
    assert!(
        log_content.contains("LoomStopped"),
        "log should contain LoomStopped entry, got: {log_content}"
    );

    // Also verify the log still has the startup entries
    assert!(
        log_content.contains("LoomStarted"),
        "log should still contain LoomStarted entry"
    );
    assert!(
        log_content.contains("KnotRegistered"),
        "log should still contain KnotRegistered entry"
    );
}

// ── Phase 2: Agent Error Logging in Knot-State and Loom-Log ────────────

/// Full pipeline test with a nonexistent agent CLI.
///
/// 1. Create a rig with a loom
/// 2. Configure `cli_path` to a nonexistent binary
/// 3. Create a strand — agent will fail
/// 4. Verify knot-state shows `Failed` with error message
/// 5. Verify loom-log contains `StrandProcessed` with error field
#[test]
fn full_pipeline_agent_error_in_state_and_log() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("error-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let port = 31998;
    let host_port = format!("127.0.0.1:{port}");

    // Use a nonexistent CLI path
    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: "/nonexistent/path/to/fake-agent".to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create a strand to trigger processing
    let strand_path = strand_dir.join("error-strand.md");
    fs::write(&strand_path, "error strand content").unwrap();

    // Wait for debounce + processing
    std::thread::sleep(Duration::from_millis(500));

    // 1. Verify knot status shows `Failed` with error message
    let (status, body) =
        http_get(&host_port, "/looms/error-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "failed",
        "knot status should be failed"
    );
    assert!(
        knot_status["last_error"].is_string(),
        "knot status should have error message"
    );
    let error_msg = knot_status["last_error"].as_str().unwrap();
    assert!(
        error_msg.contains("command not found"),
        "error should mention command not found, got: {error_msg}"
    );

    // 2. Verify loom-log contains StrandProcessed with error field
    let log_path = base_dir.join("error-loom/.loom-log");
    assert!(
        log_path.exists(),
        "loom log should exist: {}",
        log_path.display()
    );
    let log_content =
        fs::read_to_string(&log_path).expect("should read log file");
    assert!(
        log_content.contains("StrandProcessed"),
        "loom log should contain StrandProcessed entry"
    );
    assert!(
        log_content.contains("command not found"),
        "loom log should contain error details, got: {log_content}"
    );

    let _ = shutdown.send(());
}

// ── Phase 3: Full Integration Verification ──────────────────────────────

/// End-to-end test combining `.loom-config.yaml` external directories with
/// a nonexistent agent CLI.
///
/// 1. Loom in a subdirectory with nonexistent agent CLI (`/no/such/agent`).
/// 2. Create strand → triggers processing → agent fails.
/// 3. Verify:
///    - Knot status shows `Failed` with descriptive error.
///    - Loom-log contains `StrandProcessed` with error details.
///    - Tie-off file written at loom's `.knot-output` with `Failed` status.
#[test]
fn full_pipeline_external_source_with_agent_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Rig subdirectory (what the server scans for looms).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory with knot definition.
    let loom_dir = rig.join("error-external-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(root);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let port = 32001;
    let host_port = format!("127.0.0.1:{port}");

    // Use a nonexistent agent CLI path.
    let config = AppConfig {
        base_dir: rig.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: "/no/such/agent".to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Verify loom is discovered.
    let (status, _body) =
        http_get_retry(&host_port, "/looms/error-external-loom", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");

    // 2. Create strand in loom source directory → triggers processing.
    let strand_path = strand_dir.join("error-strand.md");
    fs::write(&strand_path, "external error strand content").unwrap();

    // Wait for debounce + processing.
    std::thread::sleep(Duration::from_millis(500));

    // 3. Verify knot status shows `Failed` with descriptive error.
    let (status, body) =
        http_get(&host_port, "/looms/error-external-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "failed",
        "knot status should be failed"
    );
    assert!(
        knot_status["last_error"].is_string(),
        "knot status should have error message"
    );
    let error_msg = knot_status["last_error"].as_str().unwrap();
    assert!(
        error_msg.contains("command not found"),
        "error should mention command not found, got: {error_msg}"
    );

    // 4. Verify loom-log contains `StrandProcessed` with error details.
    let log_path = rig.join("error-external-loom/.loom-log");
    assert!(
        log_path.exists(),
        "loom log should exist: {}",
        log_path.display()
    );
    let log_content =
        fs::read_to_string(&log_path).expect("should read log file");
    assert!(
        log_content.contains("StrandProcessed"),
        "loom log should contain StrandProcessed entry"
    );
    assert!(
        log_content.contains("command not found"),
        "loom log should contain error details, got: {log_content}"
    );

    // 5. Verify tie-off file written with Failed content.
    let tie_off_path =
        tie_off_dir.join("error-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        tie_off_content.contains("Processing failed"),
        "tie-off should contain Processing failed, got: {tie_off_content}"
    );
    assert!(
        tie_off_content.contains("command not found"),
        "tie-off should contain error details, got: {tie_off_content}"
    );

    let _ = shutdown.send(());
}

// ── Phase 3: Stub pi CLI Integration Tests ────────────────────────────────

/// Full happy path using a stub `pi` CLI that mimics `pi -p` behaviour.
///
/// The stub reads `--system-prompt` and `@<file>` args, then echoes them
/// back. This verifies that Knot constructs the correct CLI invocation
/// from the knot's agent config and prompt template, and that the
/// subprocess runner passes strand content to the agent.
///
/// 1. Create loom with knot (provider/openai, model/gpt-4o)
/// 2. Start server with stub-pi.sh as cli_path
/// 3. Create strand → tie-off contains system prompt + strand content
/// 4. Verify knot-state is `completed`
/// 5. Verify loom-log contains `StrandProcessed` with no error
#[test]
fn full_pipeline_with_pi_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory with a knot definition file
    let loom_dir = base_dir.join("pi-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Create the stub-pi script that echoes received args and content
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32003;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create a strand file to trigger processing
    let strand_path = strand_dir.join("test-strand.md");
    fs::write(&strand_path, "This is the strand content for review.")
        .expect("should create strand file");

    // Wait for debounce + processing
    std::thread::sleep(Duration::from_millis(500));

    // 1. Verify tie-off exists and contains the agent output
    let tie_off_path = tie_off_dir.join("test-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );

    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");

    // Tie-off should contain the system prompt from --system-prompt arg
    assert!(
        tie_off_content.contains("Review the goals section"),
        "tie-off should contain system prompt, got: {tie_off_content}"
    );

    // Tie-off should contain the strand file content (read via @<file>)
    assert!(
        tie_off_content.contains("This is the strand content for review."),
        "tie-off should contain strand content, got: {tie_off_content}"
    );

    // Tie-off should contain the model name (proves --model was passed)
    assert!(
        tie_off_content.contains("gpt-4o"),
        "tie-off should contain model name, got: {tie_off_content}"
    );

    // 2. Verify knot status is `completed`
    let (status, body) =
        http_get(&host_port, "/looms/pi-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    // 3. Verify loom-log contains StrandProcessed with no error
    let log_path = base_dir.join("pi-loom/.loom-log");
    assert!(
        log_path.exists(),
        "loom log should exist: {}",
        log_path.display()
    );
    let log_content =
        fs::read_to_string(&log_path).expect("should read log file");
    assert!(
        log_content.contains("StrandProcessed"),
        "loom log should contain StrandProcessed entry"
    );

    let _ = shutdown.send(());
}

/// Verify the stub `pi` CLI receives system prompt and strand content,
/// and that a nonexistent model causes knot-state to show `failed`.
///
/// 1. Start server with stub-pi.sh and a knot using `nonexistent-model`
/// 2. Create strand → stub exits with code 1 (simulates model not found)
/// 3. Verify knot-state shows `failed` with error message
/// 4. Verify tie-off contains error details
/// 5. Verify loom-log contains `StrandProcessed` with error field
#[test]
fn pi_agent_receives_system_prompt_and_strand() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create strand and tie-off directories
    let strand_dir = base_dir.join("strands");
    let tie_off_dir = base_dir.join("tie-offs");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();

    // Create a loom directory with a knot that uses a nonexistent model
    let loom_dir = base_dir.join("error-loom");
    fs::create_dir(&loom_dir).unwrap();
    let knot_content = format!(
        "---\nname: review-knot\nagent-config:\n  goal: \"Review with nonexistent model\"\n  provider: \"openai\"\n  model: \"nonexistent-model-xyz\"\nstrand-dir: \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review the goals section of this PRD.\n---\n\n# Error Test Knot\n\nThis knot tests error handling.\n",
        strand_dir.display(),
        tie_off_dir.display()
    );
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Create the stub-pi script (exits 1 for "nonexistent" models)
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32004;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create a strand file to trigger processing
    let strand_path = strand_dir.join("error-strand.md");
    fs::write(&strand_path, "Error test strand content")
        .expect("should create strand file");

    // Wait for debounce + processing
    std::thread::sleep(Duration::from_millis(500));

    // 1. Verify knot status shows `failed` with error message
    let (status, body) =
        http_get(&host_port, "/looms/error-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "failed",
        "knot status should be failed for nonexistent model"
    );
    assert!(
        knot_status["last_error"].is_string(),
        "knot status should have error message"
    );
    let error_msg = knot_status["last_error"].as_str().unwrap();
    assert!(
        error_msg.contains("agent execution failed")
            || error_msg.contains("exited with code 1"),
        "error should mention agent failure, got: {error_msg}"
    );

    // 2. Verify tie-off contains error details
    let tie_off_path = tie_off_dir.join("error-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        tie_off_content.contains("Processing failed"),
        "tie-off should contain Processing failed, got: {tie_off_content}"
    );

    // 3. Verify loom-log contains StrandProcessed with error
    let log_path = base_dir.join("error-loom/.loom-log");
    assert!(
        log_path.exists(),
        "loom log should exist: {}",
        log_path.display()
    );
    let log_content =
        fs::read_to_string(&log_path).expect("should read log file");
    assert!(
        log_content.contains("StrandProcessed"),
        "loom log should contain StrandProcessed entry"
    );
    // The error field should be present (non-null) in the log
    assert!(
        log_content.contains("agent execution failed")
            || log_content.contains("exited with code"),
        "loom log should contain error details, got: {log_content}"
    );

    let _ = shutdown.send(());
}

// ── Phase 4: Demo Verification ────────────────────────────────────────────

/// Demo verification: the `knot-test` loom config has provider/model fields,
/// Knot processes `sample-document.md` and produces a populated tie-off,
/// and the loom-log records successful processing.
///
/// This test mirrors the demo workflow:
/// 1. Create a rig with a `knot-test` loom (provider + model in config)
/// 2. Place `sample-document.md` in the source directory
/// 3. Start Knot with stub-pi agent
/// 4. Verify tie-off is populated (contains system prompt + strand content)
/// 5. Verify loom-log records `StrandProcessed` with no error
#[test]
fn demo_knot_test_processes_sample_document() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create strand and tie-off directories
    let strand_dir = base_dir.join("strands");
    let tie_off_dir = base_dir.join("tie-offs");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();

    // Create knot-test-loom directory with provider/model in config
    let loom_dir = base_dir.join("knot-test-loom");
    fs::create_dir(&loom_dir).unwrap();
    let knot_content = format!(
        "---\nname: review-knot\nagent-config:\n  goal: \"Review and summarize documents\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nstrand-dir: \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review the provided document. Provide a concise summary\n    of its key points and any recommendations.\n---\n\n# Review Knot\n\nThis knot reviews and summarizes documents.\n",
        strand_dir.display(),
        tie_off_dir.display()
    );
    fs::write(&loom_dir.join("review-knot.md"), knot_content).unwrap();

    // Create the sample-document.md strand
    fs::write(
        &strand_dir.join("sample-document.md"),
        r#"# Sample Document for Knot Processing

## Introduction

This is a sample document that demonstrates the Knot file
processing pipeline.

## Key Points

1. The Knot service watches a source directory for file events.
2. When a file is created or modified, the configured agent
   processes its content.
3. The agent output (tie-off) is written to the output directory.
4. Processing events are recorded in the loom-log file.

## Recommendations

- Keep documents concise for faster processing.
- Use markdown format for best results.
- Monitor the loom-log for processing status.
"#,
    )
    .unwrap();

    // Create stub-pi agent script
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32005;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Wait for initial file processing (file already exists at startup,
    // but notify may emit a Create event during discovery)
    std::thread::sleep(Duration::from_millis(500));

    // If the initial file hasn't been processed yet (startup race),
    // create a new file to trigger processing explicitly.
    let tie_off_path =
        tie_off_dir.join("sample-document.md.output");
    if !tie_off_path.exists() {
        // Touch the file to trigger a Modify event
        fs::write(&strand_dir.join("sample-document.md"),
            "# Sample Document for Knot Processing\n\n## Updated\n\nContent.")
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
    }

    // 1. Verify tie-off exists and contains populated content
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );

    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");

    // Tie-off should contain system prompt (proves --system-prompt was passed)
    assert!(
        tie_off_content.contains("Review the provided document"),
        "tie-off should contain system prompt, got: {tie_off_content}"
    );

    // Tie-off should contain model name (proves --model was passed)
    assert!(
        tie_off_content.contains("gpt-4o"),
        "tie-off should contain model name, got: {tie_off_content}"
    );

    // Tie-off should contain strand content (proves @<file> was used)
    assert!(
        tie_off_content.contains("Sample Document")
            || tie_off_content.contains("Knot Processing"),
        "tie-off should contain strand content, got: {tie_off_content}"
    );

    // 2. Verify knot status is `completed` via HTTP
    let (status, body) =
        http_get(&host_port, "/looms/knot-test-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    // 3. Verify loom-log records successful processing
    let log_path = base_dir.join("knot-test-loom/.loom-log");
    assert!(
        log_path.exists(),
        "loom log should exist: {}",
        log_path.display()
    );
    let log_content =
        fs::read_to_string(&log_path).expect("should read loom log");
    assert!(
        log_content.contains("StrandProcessed"),
        "loom log should contain StrandProcessed entry"
    );
    assert!(
        log_content.contains("sample-document.md"),
        "loom log should reference sample-document.md"
    );

    let _ = shutdown.send(());
}

/// Demo verification: knot-test loom with tools configured.
///
/// Uses a knot config with `tools: [fs, web]` to verify the
/// `build_cli_args` path that emits `--tools fs,web`.
#[test]
fn demo_knot_test_with_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create strand and tie-off directories
    let strand_dir = base_dir.join("strands");
    let tie_off_dir = base_dir.join("tie-offs");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();

    // Create knot-test-loom with tools in agent-config
    let loom_dir = base_dir.join("knot-test-loom");
    fs::create_dir(&loom_dir).unwrap();
    let knot_content = format!(
        "---\nname: review-knot\nagent-config:\n  goal: \"Review with tools\"\n  provider: \"anthropic\"\n  model: \"claude-sonnet-4-20250514\"\n  tools:\n    - fs\n    - web\nstrand-dir: \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  instructions: |\n    Review the document with tool access.\n---\n\n# Review Knot With Tools\n",
        strand_dir.display(),
        tie_off_dir.display()
    );
    fs::write(&loom_dir.join("review-knot.md"), knot_content).unwrap();

    // Create stub-pi agent that echoes all received flags
    let stub_pi = create_stub_pi_agent(&base_dir);

    let port = 32006;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: stub_pi.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create a strand file
    fs::write(&strand_dir.join("input.md"), "Document to review.").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Verify tie-off exists and contains the model from knot config
    let tie_off_path = tie_off_dir.join("input.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let tie_off_content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        tie_off_content.contains("claude-sonnet-4-20250514"),
        "tie-off should contain the configured model, got: {tie_off_content}"
    );

    // Verify knot status is completed
    let (status, body) =
        http_get(&host_port, "/looms/knot-test-loom/knots/review-knot")
            .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    let _ = shutdown.send(());
}

// ── Phase 5: Per-Knot Source Directory Integration Test ────────────────────

/// Two knots in one loom, each with its own source directory.
///
/// 1. Create a rig with a loom containing two knot files.
/// 2. Each knot defines its own `strand-dir` pointing to a separate dir.
/// 3. Start the server with a mock agent.
/// 4. Create a strand in knot A's source → processed by knot A only.
/// 5. Create a strand in knot B's source → processed by knot B only.
/// 6. Verify both knots reach `completed` status independently.
#[test]
fn server_starts_with_per_knot_source_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Rig subdirectory (what the server scans).
    let rig = root.join("rig");
    fs::create_dir(&rig).unwrap();

    // Loom directory (contains knot definitions).
    let loom_dir = rig.join("multi-knot-loom");
    fs::create_dir(&loom_dir).unwrap();

    // External source directories for each knot.
    let source_a = root.join("source-a");
    fs::create_dir(&source_a).unwrap();
    let source_b = root.join("source-b");
    fs::create_dir(&source_b).unwrap();

    // Tie-off directories for each knot.
    let tieoff_a = root.join("tieoff-a");
    fs::create_dir(&tieoff_a).unwrap();
    let tieoff_b = root.join("tieoff-b");
    fs::create_dir(&tieoff_b).unwrap();

    // Knot A — watches source-a.
    let knot_a_content = format!(
        "---
name: knot-a
agent-config:
  goal: \"Review A\"
  provider: \"openai\"
  model: \"gpt-4o\"
strand-dir: \"{}\"
tie-off-dir: \"{}\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review A\"
---
",
        source_a.display(),
        tieoff_a.display()
    );
    fs::write(loom_dir.join("knot-a.md"), knot_a_content).unwrap();

    // Knot B — watches source-b.
    let knot_b_content = format!(
        "---
name: knot-b
agent-config:
  goal: \"Review B\"
  provider: \"openai\"
  model: \"gpt-4o\"
strand-dir: \"{}\"
tie-off-dir: \"{}\"
prompt-template:
  input-bundling: \"full-file\"
  instructions: \"Review B\"
---
",
        source_b.display(),
        tieoff_b.display()
    );
    fs::write(loom_dir.join("knot-b.md"), knot_b_content).unwrap();

    // Mock agent script — ignores all CLI args built by ProcessStrand.
    let mock_agent = create_mock_agent(&root, "processed");

    let port = 32010;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: rig.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Verify loom is discovered with 2 knots.
    let (status, body) =
        http_get_retry(&host_port, "/looms/multi-knot-loom", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().expect("knots should be array");
    assert_eq!(knots.len(), 2, "loom should have 2 knots");

    // Verify both knots are present.
    let knot_ids: Vec<_> = knots
        .iter()
        .map(|k| k["id"].as_str().unwrap())
        .collect();
    assert!(
        knot_ids.contains(&"knot-a"),
        "knot-a should be present"
    );
    assert!(
        knot_ids.contains(&"knot-b"),
        "knot-b should be present"
    );

    // 1. Create a strand in source-a → should trigger knot-a.
    let strand_a_path = source_a.join("strand-a.md");
    fs::write(&strand_a_path, "content for A").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Verify knot-a reaches completed status.
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/multi-knot-loom/knots/knot-a",
            30,
            100,
        )
        .expect("knot-a status should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_a_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_a_status["status"].as_str().unwrap(),
        "completed",
        "knot-a status should be completed"
    );

    // 2. Create a strand in source-b → should trigger knot-b.
    let strand_b_path = source_b.join("strand-b.md");
    fs::write(&strand_b_path, "content for B").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    // Verify knot-b reaches completed status.
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/multi-knot-loom/knots/knot-b",
            30,
            100,
        )
        .expect("knot-b status should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_b_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_b_status["status"].as_str().unwrap(),
        "completed",
        "knot-b status should be completed"
    );

    // Verify knot-a strand_path references source-a file.
    assert!(
        knot_a_status["last_strand_path"]
            .as_str()
            .unwrap_or("")
            .contains("strand-a.md"),
        "knot-a should reference strand-a.md, got: {knot_a_status:?}"
    );

    // Verify knot-b strand_path references source-b file.
    assert!(
        knot_b_status["last_strand_path"]
            .as_str()
            .unwrap_or("")
            .contains("strand-b.md"),
        "knot-b should reference strand-b.md, got: {knot_b_status:?}"
    );

    let _ = shutdown.send(());
}

// ── Phase 5: Integration Test — Full Lifecycle ─────────────────────────────

/// `POST /looms` registers a loom with knots → create strand file in source
/// dir → tie-off produced → verify via `GET /looms/:id/knots/:knot_name`.
///
/// Verifies end-to-end: HTTP → RegisterLoom → EventSource::watch() → file
/// creation → debounce → agent → tie-off.
#[test]
fn http_register_then_process_strand() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Mock agent script — ignores all CLI args built by ProcessStrand.
    let mock_agent = create_mock_agent(&base_dir, "http-processed");

    let port = 32020;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // Create source directory AFTER server start so startup discovery
    // doesn't find it — we test POST /looms registration path.
    let source_dir = base_dir.join("http-reg-loom");
    fs::create_dir(&source_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(source_dir.join("review.md"), knot_content).unwrap();

    // 1. POST /looms to register the loom with knot definitions.
    let body = serde_json::json!({
        "id": "http-reg-loom",
        "knots": [
            {
                "name": "review-knot",
                "agent_config": {
                    "goal": "Review documents",
                    "provider": "openai",
                    "model": "gpt-4o"
                },
                "prompt_template": {
                    "input_bundling": "full-file",
                    "instructions": "Review docs"
                },
                "strand_dir": strand_dir.to_string_lossy(),
                "tie_off_dir": tie_off_dir.to_string_lossy()
            }
        ]
    });
    let (status, _resp) =
        http_post_json(&host_port, "/looms", &body)
            .expect("register loom should respond");
    assert!(
        status.contains("201"),
        "register loom should return 201, got: {status}"
    );

    // 2. Verify loom is registered and has knots.
    let (status, body) =
        http_get_retry(&host_port, "/looms/http-reg-loom", 30, 100)
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().expect("knots should be array");
    assert_eq!(knots.len(), 1, "loom should have 1 knot");
    assert_eq!(
        knots[0]["id"].as_str().unwrap(),
        "review-knot",
        "knot id should match"
    );

    // 3. Create a strand file in the strand directory (watched by the loom).
    let strand_path = strand_dir.join("new-strand.md");
    fs::write(&strand_path, "strand content via http").unwrap();

    // Wait for debounce + processing.
    std::thread::sleep(Duration::from_millis(800));

    // 4. Verify tie-off was produced in tie_off_dir.
    let tie_off_path =
        tie_off_dir.join("new-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("http-processed"),
        "tie-off should contain agent output, got: {content}"
    );

    // 5. Verify via GET /looms/:id/knots/:knot_name.
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/http-reg-loom/knots/review-knot",
            30,
            100,
        )
        .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    let _ = shutdown.send(());
}

/// Create loom directory on disk → `POST /looms/discover` → create strand
/// file → tie-off produced.
///
/// Verifies the discover path: filesystem directory → HTTP discover →
/// registration with watchers → processing.
#[test]
fn discover_then_process_strand() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom directory on disk with a knot definition.
    let loom_dir = base_dir.join("discover-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Mock agent script.
    let mock_agent = create_mock_agent(&base_dir, "discovered-processed");

    let port = 32021;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Loom was discovered at startup.
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "should have 1 loom at startup");
    assert_eq!(
        summaries[0]["id"].as_str().unwrap(),
        "discover-loom",
        "loom id should match"
    );

    // 2. POST /looms/discover should return empty (loom already registered).
    let (status, body) =
        http_post_json(&host_port, "/looms/discover", &serde_json::json!({}))
            .expect("discover endpoint should respond");
    assert!(
        status.contains("200"),
        "discover should return 200, got: {status}"
    );
    let discovered: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert!(
        discovered.is_empty(),
        "discover should return empty (loom already registered), \
         got {discovered:?}"
    );

    // 3. Create a strand file in the source directory.
    let strand_path = strand_dir.join("discover-strand.md");
    fs::write(&strand_path, "discovered strand content").unwrap();

    // Wait for debounce + processing.
    std::thread::sleep(Duration::from_millis(800));

    // 4. Verify tie-off was produced.
    let tie_off_path =
        tie_off_dir.join("discover-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("discovered-processed"),
        "tie-off should contain agent output, got: {content}"
    );

    // 5. Verify via GET /looms/:id/knots/:knot_name.
    let (status, body) =
        http_get_retry(
            &host_port,
            "/looms/discover-loom/knots/review-knot",
            30,
            100,
        )
        .expect("knot status endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot status should be completed"
    );

    let _ = shutdown.send(());
}

/// `DELETE /looms/:id` stops processing — new strand files are NOT
/// processed after unregistration (watcher removed).
///
/// Verifies: Register → Unregister → create strand → no tie-off produced.
#[test]
fn unregister_stops_processing() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create source directory with knot definition file.
    let source_dir = base_dir.join("unreg-loom");
    fs::create_dir(&source_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(source_dir.join("review.md"), knot_content).unwrap();

    // Mock agent script.
    let mock_agent = create_mock_agent(&base_dir, "should-not-run");

    let port = 32022;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Verify loom was discovered at startup.
    let (status, _body) =
        http_get_retry(&host_port, "/looms/unreg-loom", 30, 100)
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");

    // 2. DELETE /looms/:id to unregister the loom (stops watcher).
    let (status, _body) =
        http_delete(&host_port, "/looms/unreg-loom")
            .expect("unregister should respond");
    assert!(
        status.contains("204"),
        "unregister should return 204, got: {status}"
    );

    // Give a brief moment for the watcher to be removed.
    std::thread::sleep(Duration::from_millis(200));

    // 3. Create a strand file AFTER unregistration.
    let strand_path = source_dir.join("post-unreg-strand.md");
    fs::write(&strand_path, "this should not be processed").unwrap();

    // Wait to confirm no processing happens.
    std::thread::sleep(Duration::from_millis(800));

    // 4. Verify NO tie-off was produced.
    let tie_off_path =
        source_dir.join(".knot-output/post-unreg-strand.md.output");
    assert!(
        !tie_off_path.exists(),
        "tie-off should NOT exist after unregister: {}",
        tie_off_path.display()
    );

    // 5. Verify loom is no longer in the list.
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    let ids: Vec<_> =
        summaries.iter().map(|s| s["id"].as_str().unwrap_or(""))
            .collect();
    assert!(
        !ids.contains(&"unreg-loom"),
        "unregistered loom should not appear in list"
    );

    let _ = shutdown.send(());
}

/// Integration test: full tie-off lifecycle with append mode.
///
/// Note: file watchers may coalesce create+write into a single Modified event,
/// so we test the lifecycle as: first write (Modified) → second write (Modified)
/// → delete (Deleted), verifying append mode preserves history across events.
#[test]
fn full_tie_off_history() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create loom directory with knot definition
    let loom_dir = base_dir.join("history-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    // Simple mock agent — always returns "processed"
    let mock_agent = create_mock_agent(&base_dir, "processed");

    let port = 31999;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    let strand_path = strand_dir.join("lifecycle-strand.md");
    let tie_off_path = tie_off_dir.join("lifecycle-strand.md.output");

    // Step 1: First write (triggers Modified event)
    fs::write(&strand_path, "initial content").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    assert!(
        tie_off_path.exists(),
        "tie-off should exist after first write (expected: {})",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    // Should have event metadata (Modified or Created depending on watcher)
    assert!(
        content.contains("## Event:")
            && content.contains("## Strand:")
            && content.contains("## Timestamp:"),
        "should have event metadata headers: {}", content
    );
    assert!(
        content.contains("processed"),
        "should have agent response: {}", content
    );

    // Step 2: Second write (triggers another Modified event)
    fs::write(&strand_path, "modified content").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    // Should have two sections now
    let delimiter_count = content.matches("---").count();
    assert!(
        delimiter_count >= 2,
        "should have at least 2 sections with delimiters, found {}: {}",
        delimiter_count, content
    );
    // Both sections should have event headers
    let event_count = content.matches("## Event:").count();
    assert!(
        event_count >= 2,
        "should have at least 2 event sections, found {}: {}",
        event_count, content
    );

    // Step 3: Delete strand (triggers Deleted event)
    fs::remove_file(&strand_path).unwrap();
    std::thread::sleep(Duration::from_millis(800));

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("## Event: Deleted"),
        "should have Deleted section: {}", content
    );
    // Should have 3 sections now
    let delimiter_count = content.matches("---").count();
    assert!(
        delimiter_count >= 4,
        "should have 3 sections with delimiters, found {}: {}",
        delimiter_count, content
    );
    let event_count = content.matches("## Event:").count();
    assert!(
        event_count >= 3,
        "should have 3 event sections, found {}: {}",
        event_count, content
    );

    // Verify chronological order: Deleted should come last
    let first_event = content.find("## Event:").unwrap();
    let deleted_event = content.rfind("## Event: Deleted").unwrap();
    assert!(
        first_event < deleted_event,
        "Deleted should come after earlier events"
    );

    let _ = shutdown.send(());
}

/// Integration test: parse tie-off markdown sections and verify structure.
///
/// Creates a strand, modifies it, and verifies that the tie-off file
/// contains properly formatted sections with event type, strand path,
/// and timestamp metadata.
#[test]
fn tie_off_sections_readable() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create loom directory with knot definition
    let loom_dir = base_dir.join("sections-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();

    let mock_agent = create_mock_agent(&base_dir, "processed");

    let port = 32000;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    let strand_path = strand_dir.join("sections-strand.md");
    let tie_off_path = tie_off_dir.join("sections-strand.md.output");

    // Create then modify
    fs::write(&strand_path, "content v1").unwrap();
    std::thread::sleep(Duration::from_millis(800));
    fs::write(&strand_path, "content v2").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");

    // Split into sections using --- as delimiter.
    // Structure: [header] --- [content] --- [header] --- [content] ...
    let sections: Vec<&str> = content
        .split("---")
        .filter(|s| !s.trim().is_empty())
        .collect();

    assert!(
        sections.len() >= 2,
        "should have at least 2 sections, found {}: {}",
        sections.len(), content
    );

    // Collect all header sections (those containing ## Event:)
    let header_sections: Vec<&str> = sections
        .iter()
        .filter(|s| s.contains("## Event:"))
        .copied()
        .collect();

    assert!(
        header_sections.len() >= 2,
        "should have at least 2 header sections, found {}: {}",
        header_sections.len(), content
    );

    // Verify each header section has complete metadata
    for (i, section) in header_sections.iter().enumerate() {
        assert!(
            section.contains("## Event:"),
            "header section {} should have event type: {}",
            i, section
        );
        assert!(
            section.contains("## Strand:"),
            "header section {} should have strand path: {}",
            i, section
        );
        assert!(
            section.contains("## Timestamp:"),
            "header section {} should have timestamp: {}",
            i, section
        );

        // Verify timestamp format (ISO 8601)
        if let Some(ts_start) = section.find("## Timestamp:") {
            let ts_line = section[ts_start..]
                .lines()
                .next()
                .unwrap_or("");
            let ts_value = ts_line
                .trim()
                .strip_prefix("## Timestamp:")
                .unwrap_or("")
                .trim();
            assert!(
                ts_value.contains('T') && ts_value.ends_with('Z'),
                "timestamp should be ISO 8601 format, got: {}",
                ts_value
            );
        }

        // Verify strand path is present
        if let Some(strand_start) = section.find("## Strand:") {
            let strand_line = section[strand_start..]
                .lines()
                .next()
                .unwrap_or("");
            let strand_value = strand_line
                .trim()
                .strip_prefix("## Strand:")
                .unwrap_or("")
                .trim();
            assert!(
                !strand_value.is_empty(),
                "strand path should not be empty"
            );
            assert!(
                strand_value.contains("sections-strand.md"),
                "strand path should reference the strand file: {}",
                strand_value
            );
        }
    }

    // Verify markdown structure: sections separated by --- (horizontal rule)
    assert!(
        content.contains("\n---\n"),
        "tie-off should have --- delimiter between sections: {}",
        content
    );

    // Verify markdown structure: sections separated by --- (horizontal rule)
    assert!(
        content.contains("\n---\n"),
        "tie-off should have --- delimiter between sections: {}",
        content
    );

    let _ = shutdown.send(());
}





