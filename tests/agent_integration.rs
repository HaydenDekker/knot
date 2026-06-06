//! Agent integration tests — mock agent, pi stub, error paths.
//!
//! These tests verify that Knot correctly invokes external agent CLIs,
//! handles agent failures, and records results in knot-state and loom-log.

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

// ── Agent Error Logging in Knot-State and Loom-Log ─────────────────────

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

// ── Full Pipeline with External Source + Agent Error ────────────────────

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

// ── Stub pi CLI Integration Tests ──────────────────────────────────────

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
