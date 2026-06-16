//! Rig-log integration tests.
//!
//! Validates queue idle detection, timeout handling, and tie-off
//! preservation via the rig-log.
//!
//! - `QueueIdle` — written after processing when no events arrive
//!   within the poll window.
//! - `TimeoutExceeded` — written when the agent exceeds its deadline;
//!   tie-off is preserved unchanged.
//! - Successful processing — no rig-log entry.
//! - Non-timeout failure — error IS written to tie-off (regression).

mod helpers;

use std::fs;
use std::time::Duration;

use helpers::*;

// ── Helper: create profile with timeout ────────────────────────────────

/// Create a profile file with an explicit `timeout` field in seconds.
pub fn create_profile_with_timeout(
    dir: &std::path::Path,
    name: &str,
    timeout_secs: Option<u64>,
) {
    let profiles_dir = dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    let timeout_yaml = match timeout_secs {
        Some(secs) => format!("timeout: {secs}\n"),
        None => String::new(),
    };
    fs::write(
        profiles_dir.join(format!("{name}.md")),
        format!(
            "---\nname: {name}\nprovider: openai\nmodel: gpt-4o\n{}profile-prompt: |\n  You are a reviewer.\n---\n\nProfile {name}\n",
            timeout_yaml,
        ),
    )
    .unwrap();
}

// ── Helper: slow mock agent ────────────────────────────────────────────

/// Create a mock agent script that reads stdin, sleeps, then produces
/// output. Used to simulate agent execution that exceeds the timeout.
///
/// `delay_ms` is the sleep duration in milliseconds. The agent sleeps
/// *after* consuming stdin so the subprocess runner can write the prompt.
fn create_slow_mock_agent(
    dir: &std::path::Path,
    delay_ms: u64,
    output: &str,
) -> std::path::PathBuf {
    let script_path = dir.join("slow-agent");
    let delay_s = delay_ms as f64 / 1000.0;
    fs::write(
        &script_path,
        format!(
            "#!/bin/sh\ncat >/dev/null\nsleep {delay_s}\necho '{}'
",
            output,
        ),
    )
    .expect("should write slow agent script");
    fs::set_permissions(
        &script_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("should set script as executable");
    script_path
}

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
        rig_dir: base_dir.clone(),
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
            .unwrap_or_else(|_| panic!("rig-log line should be valid JSON: {line}"));
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
        rig_dir: base_dir.clone(),
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
            .unwrap_or_else(|_| panic!("rig-log line should be valid JSON: {line}"));
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

// ── Timeout Tests ─────────────────────────────────────────────────────

/// Timeout → rig-log `TimeoutExceeded` entry + tie-off preserved unchanged.
///
/// Timeline:
/// 1. Server starts with profile `timeout: 2` (2 second deadline)
/// 2. Slow agent sleeps 5 seconds (exceeds deadline)
/// 3. SubprocessAgentRunner kills agent, returns PortError::Timeout
/// 4. ProcessStrand writes TimeoutExceeded to rig-log
/// 5. ProcessStrand does NOT write error to tie-off
/// 6. After drain check (500ms), QueueIdle written to rig-log
/// 7. Read .rig-log → TimeoutExceeded + QueueIdle present
/// 8. Tie-off file does NOT contain "Processing failed"
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_writes_rig_log_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Set up loom + knot + profile with 2-second timeout
    let loom_dir = base_dir.join("timeout-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_profile_with_timeout(&base_dir, "fast", Some(2));

    // Agent sleeps 5 seconds — exceeds the 2-second profile timeout
    let slow_agent =
        create_slow_mock_agent(&base_dir, 5000, "should-not-appear");

    let port = 31992;
    let host_port = format!("127.0.0.1:{port}");

    let config = knot::AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: knot::RigAgentConfig {
            cli_path: slow_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        // Runner default is 300s — profile timeout of 2s overrides this
        ..knot::AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand — triggers processing that will timeout
    let strand_path = strand_dir.join("timeout-strand.md");
    fs::write(&strand_path, "timeout content").unwrap();

    // Wait for: debounce (100ms) + timeout (2s) + drain check (500ms) + buffer
    // Total: ~2700ms. 6000ms is generous.
    tokio::time::sleep(Duration::from_millis(6000)).await;

    // Read the rig-log file directly
    let rig_log_path = base_dir.join(".rig-log");
    assert!(
        rig_log_path.exists(),
        "rig-log file should exist at: {}",
        rig_log_path.display()
    );

    let content =
        fs::read_to_string(&rig_log_path).expect("should read rig-log file");

    // Parse each line as JSON and look for TimeoutExceeded
    let mut has_timeout = false;
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let event: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|_| panic!("rig-log line should be valid JSON: {line}"));
        if event.get("TimeoutExceeded").is_some() {
            has_timeout = true;
        }
    }

    assert!(
        has_timeout,
        "rig-log should contain TimeoutExceeded event.\n\nRig-log content:\n{}",
        content
    );

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), _handle).await;
}

/// Successful processing produces no rig-log TimeoutExceeded entry.
///
/// A successful agent run should only produce QueueIdle in the rig-log
/// (no TimeoutExceeded). This is a negative test confirming that
/// TimeoutExceeded is only written on actual timeout.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_processing_no_rig_log_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Set up loom + knot + profile
    let loom_dir = base_dir.join("success-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_fast_profile(&base_dir);

    let agent = create_mock_agent(&base_dir, "success-output");

    let port = 31993;
    let host_port = format!("127.0.0.1:{port}");

    let config = knot::AppConfig {
        rig_dir: base_dir.clone(),
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

    // Create a strand — triggers successful processing
    let strand_path = strand_dir.join("success-strand.md");
    fs::write(&strand_path, "success content").unwrap();

    // Wait for processing + drain check + buffer
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Read the rig-log file directly
    let rig_log_path = base_dir.join(".rig-log");
    assert!(
        rig_log_path.exists(),
        "rig-log file should exist at: {}",
        rig_log_path.display()
    );

    let content =
        fs::read_to_string(&rig_log_path).expect("should read rig-log file");

    // Parse each line and verify NO TimeoutExceeded
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let event: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|_| panic!("rig-log line should be valid JSON: {line}"));
        assert!(
            event.get("TimeoutExceeded").is_none(),
            "successful processing should NOT produce TimeoutExceeded.\n\nRig-log content:\n{}",
            content
        );
    }

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), _handle).await;
}

/// Tie-off file is preserved unchanged when agent times out.
///
/// Verifies that on timeout, ProcessStrand does NOT write any error
/// content to the tie-off file. The tie-off should either not exist
/// (first processing attempt) or contain only previously successful
/// content.
///
/// Timeline:
/// 1. Server starts with profile timeout: 2s
/// 2. Slow agent exceeds timeout → PortError::Timeout
/// 3. ProcessStrand skips tie-off write (preserves existing content)
/// 4. Tie-off file does NOT contain "Processing failed"
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tieoff_preserved_on_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Set up loom + knot + profile with 2-second timeout
    let loom_dir = base_dir.join("preserve-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_profile_with_timeout(&base_dir, "fast", Some(2));

    // Agent sleeps 5 seconds — exceeds the 2-second profile timeout
    let slow_agent =
        create_slow_mock_agent(&base_dir, 5000, "should-not-appear");

    let port = 31994;
    let host_port = format!("127.0.0.1:{port}");

    let config = knot::AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: knot::RigAgentConfig {
            cli_path: slow_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..knot::AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand — triggers processing that will timeout
    let strand_path = strand_dir.join("preserve-strand.md");
    fs::write(&strand_path, "preserve content").unwrap();

    // Wait for: debounce + timeout + drain check + buffer
    tokio::time::sleep(Duration::from_millis(6000)).await;

    // Check the tie-off file
    let tie_off_path = base_dir
        .join("tie-offs/preserve-loom/review-knot/review-knot-tie-off.md");

    if tie_off_path.exists() {
        let content =
            fs::read_to_string(&tie_off_path).expect("should read tie-off");
        assert!(
            !content.contains("Processing failed"),
            "tie-off should NOT contain error content on timeout.\n\nTie-off content:\n{}",
            content
        );
        assert!(
            !content.contains("should-not-appear"),
            "tie-off should NOT contain agent output on timeout.\n\nTie-off content:\n{}",
            content
        );
    }
    // If tie-off doesn't exist, that's also valid (no write was made).

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), _handle).await;
}

/// Non-timeout failure still writes error to tie-off (regression guard).
///
/// Verifies that when the agent fails with a non-zero exit code (not
/// a timeout), the error IS written to the tie-off file. This preserves
/// the existing behaviour and ensures the timeout-specific change
/// doesn't accidentally suppress all error writes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tieoff_receives_error_on_non_timeout_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Set up loom + knot + profile
    let loom_dir = base_dir.join("non-timeout-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_fast_profile(&base_dir);

    // Agent that fails with exit code 1 (non-timeout error)
    let script_path = base_dir.join("failing-agent");
    fs::write(
        &script_path,
        "#!/bin/sh\ncat >/dev/null\necho 'agent error' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(
        &script_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();

    let port = 31995;
    let host_port = format!("127.0.0.1:{port}");

    let config = knot::AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: knot::RigAgentConfig {
            cli_path: script_path.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..knot::AppConfig::default_config()
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand — agent will fail with non-zero exit
    let strand_path = strand_dir.join("fail-strand.md");
    fs::write(&strand_path, "fail content").unwrap();

    // Wait for processing + drain check + buffer
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Tie-off should exist and contain error content
    let tie_off_path = base_dir
        .join("tie-offs/non-timeout-loom/review-knot/review-knot-tie-off.md");
    assert!(
        tie_off_path.exists(),
        "error tie-off should exist: {}",
        tie_off_path.display()
    );

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("Processing failed"),
        "tie-off should contain 'Processing failed' for non-timeout error.\n\nTie-off content:\n{}",
        content
    );

    // Verify NO TimeoutExceeded in rig-log (this was a regular failure)
    let rig_log_path = base_dir.join(".rig-log");
    if rig_log_path.exists() {
        let rig_log_content =
            fs::read_to_string(&rig_log_path).expect("should read rig-log");
        for line in rig_log_content
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let event: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|_| panic!("rig-log line should be valid JSON: {line}"));
            assert!(
                event.get("TimeoutExceeded").is_none(),
                "non-timeout failure should NOT produce TimeoutExceeded.\n\nRig-log content:\n{}",
                rig_log_content
            );
        }
    }

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), _handle).await;
}
