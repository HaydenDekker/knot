//! Rig-log integration tests.
//!
//! Validates queue idle detection via the rig-log: after processing
//! strand events, the server writes `QueueIdle` to the rig-log when
//! no further events arrive within the poll window.

mod helpers;

use std::fs;
use std::time::Duration;

use helpers::*;

/// After processing a single strand event, `QueueIdle` appears in the
/// rig-log within the poll window (500ms).
///
/// Timeline:
/// 1. Server starts with loom + mock agent
/// 2. Strand created → debounce (100ms) → ProcessStrand → agent (fast)
/// 3. After processing, drain check waits 500ms for next event
/// 4. No event arrives → QueueIdle written to rig-log
/// 5. Read .rig-log file → QueueIdle present
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_event_queue_idle_written() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Set up loom + knot + profile
    let loom_dir = base_dir.join("idle-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_fast_profile(&base_dir);

    let agent = create_mock_agent(&base_dir, "done");

    let port = 31990;
    let host_port = format!("127.0.0.1:{port}");

    let config = knot::AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: knot::RigAgentConfig {
            cli_path: agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..knot::AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand — triggers processing
    let strand_path = strand_dir.join("single-strand.md");
    fs::write(&strand_path, "single event content").unwrap();

    // Wait for processing + drain check (500ms poll window) + small buffer.
    // Timeline: notify poll (~50ms) + debounce (100ms) + agent (~10ms) +
    // drain check (500ms) = ~570ms. 1500ms is generous.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Read the rig-log file directly
    let rig_log_path = base_dir.join(".rig-log");
    assert!(
        rig_log_path.exists(),
        "rig-log file should exist at: {}",
        rig_log_path.display()
    );

    let content = fs::read_to_string(&rig_log_path)
        .expect("should read rig-log file");

    // Parse each line as JSON and look for QueueIdle
    let mut has_queue_idle = false;
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let event: serde_json::Value = serde_json::from_str(line)
            .expect(&format!("rig-log line should be valid JSON: {line}"));
        if event.get("QueueIdle").is_some() {
            has_queue_idle = true;
        }
    }

    assert!(
        has_queue_idle,
        "rig-log should contain QueueIdle event after single event processed.\n\
         Rig-log content:\n{}",
        content
    );

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), _handle).await;
}

/// Rapid burst of 3 events produces only one `QueueIdle` entry (after all
/// complete), not one per event.
///
/// Timeline:
/// 1. 3 strands created rapidly (within debounce window per file)
/// 2. Each is debounced independently (different file paths)
/// 3. ProcessStrand processes event1, drain check sees event2 within window
/// 4. ProcessStrand processes event2, drain check sees event3 within window
/// 5. ProcessStrand processes event3, drain check times out → QueueIdle
/// 6. Only one QueueIdle written (not three)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn burst_events_single_queue_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Set up loom + knot + profile
    let loom_dir = base_dir.join("burst-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_fast_profile(&base_dir);

    let agent = create_mock_agent(&base_dir, "burst-done");

    let port = 31991;
    let host_port = format!("127.0.0.1:{port}");

    let config = knot::AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: knot::RigAgentConfig {
            cli_path: agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..knot::AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create 3 strands rapidly — different file names so each gets
    // its own debounced event (different debounce keys).
    let files = ["alpha.md", "beta.md", "gamma.md"];
    for name in &files {
        fs::write(
            strand_dir.join(name),
            format!("{name} content"),
        )
        .unwrap();
    }

    // Wait for: debounce of all 3 + processing + drain checks between each +
    // final drain check (500ms).
    // 3 events × (100ms debounce + ~10ms processing + up to 500ms drain)
    // But since events arrive in burst, the drain checks for first two
    // should quickly find the next event. Only the last drain check
    // should time out (500ms).
    // Total: ~100ms (debounce) + 3 × ~10ms (processing) + 500ms (final drain)
    // ≈ 700ms. 3000ms is generous.
    tokio::time::sleep(Duration::from_millis(3000)).await;

    // Read the rig-log file directly
    let rig_log_path = base_dir.join(".rig-log");
    assert!(
        rig_log_path.exists(),
        "rig-log file should exist at: {}",
        rig_log_path.display()
    );

    let content = fs::read_to_string(&rig_log_path)
        .expect("should read rig-log file");

    // Count QueueIdle events
    let mut queue_idle_count = 0;
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let event: serde_json::Value = serde_json::from_str(line)
            .expect(&format!("rig-log line should be valid JSON: {line}"));
        if event.get("QueueIdle").is_some() {
            queue_idle_count += 1;
        }
    }

    assert_eq!(
        queue_idle_count, 1,
        "rig-log should contain exactly one QueueIdle event after burst.\n\
         Found {} QueueIdle events in:\n{}",
        queue_idle_count, content
    );

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), _handle).await;
}
