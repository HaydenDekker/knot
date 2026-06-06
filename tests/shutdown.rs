//! Graceful shutdown integration tests.
//!
//! Verifies that Knot stops file watchers and logs shutdown events
//! when the shutdown signal is received.

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

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
    let (knot_content, strand_dir, tie_off_dir) =
        make_knot_content_with_dirs(&base_dir);
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
    let (knot_content, _strand_dir, _tie_off_dir) =
        make_knot_content_with_dirs(&base_dir);
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
