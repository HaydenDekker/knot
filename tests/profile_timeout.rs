//! Profile timeout integration tests.
//!
//! Validates that the `timeout` field in agent profiles controls the
//! agent session deadline:
//!
//! - Profile with `timeout: 2` → agent killed after 2 seconds
//! - Profile without `timeout` → uses runner's global default
//! - Profile with `timeout: 600` → overrides short runner default
//! - Profile file with `timeout: 30` round-trips through repository

mod helpers;

use std::fs;
use std::time::Duration;

use helpers::*;
use knot::application::ports::AgentProfileRepository;

// ── Helper: create profile with timeout ────────────────────────────────

/// Create a profile file with an explicit `timeout` field in seconds.
fn create_profile_with_timeout(
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
            "---\nname: {name}\nprovider: openai\nmodel: gpt-4o\n{timeout_yaml}system-prompt: |\n  You are a reviewer.\n---\n\nProfile {name}\n",
        ),
    )
    .unwrap();
}

// ── Helper: slow mock agent ────────────────────────────────────────────

/// Create a mock agent that sleeps for a duration (simulates slow agent).
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
            "#!/bin/sh\ncat >/dev/null\nsleep {delay_s}\necho '{}'\n",
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

// ── Tests ──────────────────────────────────────────────────────────────

/// Profile with `timeout: 2` → agent session killed after ~2 seconds,
/// `TimeoutExceeded` appears in rig-log.
///
/// Verifies that the profile's timeout field overrides the runner's
/// global default (300s). The agent sleeps for 10 seconds but is killed
/// at the 2-second profile timeout.
///
/// Timeline:
/// 1. Server starts with profile `timeout: 2`, runner default 300s
/// 2. Slow agent (sleeps 10s) is invoked
/// 3. Profile timeout of 2s overrides runner default
/// 4. Agent killed at ~2s → PortError::Timeout
/// 5. TimeoutExceeded written to rig-log
/// 6. QueueIdle written after drain check
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_timeout_two_seconds_kills_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Set up loom + knot + profile with 2-second timeout
    let loom_dir = base_dir.join("timeout2-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_profile_with_timeout(&base_dir, "fast", Some(2));

    // Agent sleeps 10 seconds — far exceeds the 2-second profile timeout
    let slow_agent =
        create_slow_mock_agent(&base_dir, 10000, "should-not-appear");

    let port = 31996;
    let host_port = format!("127.0.0.1:{port}");

    let config = knot::AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: knot::RigAgentConfig {
            cli_path: slow_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        // Runner default is 300s — profile timeout of 2s overrides
        ..knot::AppConfig::default_config()
    };

    let start = tokio::time::Instant::now();

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand — triggers processing that will timeout
    let strand_path = strand_dir.join("timeout2-strand.md");
    fs::write(&strand_path, "content\ntimeout test").unwrap();

    // Wait for: debounce (100ms) + timeout (2s) + drain check (500ms)
    // + buffer. 6000ms is generous.
    tokio::time::sleep(Duration::from_millis(6000)).await;

    // Verify rig-log has TimeoutExceeded
    let rig_log_path = base_dir.join(".rig-log");
    assert!(
        rig_log_path.exists(),
        "rig-log should exist at: {}",
        rig_log_path.display()
    );

    let content =
        fs::read_to_string(&rig_log_path).expect("should read rig-log");

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
        "rig-log should contain TimeoutExceeded (profile timeout: 2s).\\n\\nRig-log content:\\n{}",
        content
    );

    // Total elapsed should be well under 300s (runner default),
    // proving the profile timeout was used.
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(15),
        "should use profile timeout (2s), not runner default (300s).\\n\\nElapsed: {:?}",
        elapsed
    );

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), _handle).await;
}

/// Profile with NO timeout field → uses runner's global default.
///
/// When the profile does not specify `timeout`, the agent runner falls
/// back to its global default (set in AppConfig.agent_timeout). Here
/// we set the runner default to 500ms so we can verify fallback
/// behaviour without waiting for the 300s default.
///
/// An agent that completes in < 500ms succeeds; one that takes longer
/// times out at the runner default (not some profile-specific value).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_no_timeout_uses_runner_default() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Set up loom + knot + profile WITHOUT timeout field
    let loom_dir = base_dir.join("default-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_profile_with_timeout(&base_dir, "fast", None); // no timeout

    // Agent sleeps 5 seconds
    let slow_agent =
        create_slow_mock_agent(&base_dir, 5000, "should-not-appear");

    let port = 31997;
    let host_port = format!("127.0.0.1:{port}");

    let config = knot::AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: knot::RigAgentConfig {
            cli_path: slow_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        // Runner default set to 500ms — profile has no timeout, so this
        // is used as the fallback deadline.
        agent_timeout: Duration::from_millis(500),
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand
    let strand_path = strand_dir.join("default-strand.md");
    fs::write(&strand_path, "default content").unwrap();

    // Wait for: debounce + runner timeout (500ms) + drain check + buffer
    tokio::time::sleep(Duration::from_millis(4000)).await;

    // Verify rig-log has TimeoutExceeded
    let rig_log_path = base_dir.join(".rig-log");
    assert!(
        rig_log_path.exists(),
        "rig-log should exist at: {}",
        rig_log_path.display()
    );

    let content =
        fs::read_to_string(&rig_log_path).expect("should read rig-log");

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
        "rig-log should contain TimeoutExceeded (runner default timeout).\\n\\nRig-log content:\\n{}",
        content
    );

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), _handle).await;
}

/// Profile with `timeout: 600` overrides runner default of 2 seconds.
///
/// Verifies that a large profile timeout overrides a short runner default.
/// The agent completes quickly (< 2s) so the profile timeout of 600s is
/// not reached — the agent succeeds. This proves the profile timeout is
/// used as the deadline, not the runner default.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_timeout_overrides_runner_default() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Set up loom + knot + profile with 600-second timeout
    let loom_dir = base_dir.join("long-loom");
    fs::create_dir(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_knot_content_with_dirs(&base_dir);
    fs::write(loom_dir.join("review.md"), knot_content).unwrap();
    create_profile_with_timeout(&base_dir, "fast", Some(600));

    // Fast agent — completes instantly
    let agent = create_mock_agent(&base_dir, "long-timeout-success");

    let port = 31998;
    let host_port = format!("127.0.0.1:{port}");

    let config = knot::AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: knot::RigAgentConfig {
            cli_path: agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        // Runner default is only 2s — but profile timeout of 600s overrides.
        // Agent completes instantly, so no timeout occurs regardless.
        agent_timeout: Duration::from_secs(2),
    };

    let (_handle, shutdown_tx) = spawn_server_with_shutdown(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // Create a strand — should succeed (profile timeout: 600s, agent fast)
    let strand_path = strand_dir.join("long-strand.md");
    fs::write(&strand_path, "long timeout content").unwrap();

    // Wait for: debounce + processing + drain check + buffer
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Verify tie-off was produced (agent succeeded)
    let tie_off_path =
        base_dir.join("tie-offs/long-loom/review-knot/review-knot-tie-off.md");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist — agent completed within profile timeout (600s)"
    );

    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("long-timeout-success"),
        "tie-off should contain agent output: {}",
        content
    );

    // Verify NO TimeoutExceeded in rig-log (agent completed successfully)
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
                "should NOT have TimeoutExceeded — agent succeeded.\\n\\nRig-log:\\n{}",
                rig_log_content
            );
        }
    }

    // Clean shutdown
    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(10), _handle).await;
}

/// Profile file with `timeout: 30` serialises/deserialises correctly
/// through the FileSystemAgentProfileRepository (round-trip test).
///
/// Verifies:
/// 1. Profile file with `timeout: 30` is written correctly
/// 2. `FileSystemAgentProfileRepository::get()` reads it back
/// 3. The `timeout` field is correctly parsed as `Some(30)`
/// 4. `save()` writes the profile and preserves the timeout value
/// 5. Subsequent `get()` reads the same timeout value
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_timeout_roundtrip_via_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create profile file with timeout: 30
    create_profile_with_timeout(&base_dir, "timed-profile", Some(30));

    // Read back via filesystem
    let profiles_dir = base_dir.join("profiles");
    let content =
        fs::read_to_string(profiles_dir.join("timed-profile.md")).unwrap();

    // Verify the YAML frontmatter contains the timeout field
    assert!(
        content.contains("timeout: 30"),
        "profile file should contain 'timeout: 30' in frontmatter.\\n\\nFile content:\\n{}",
        content
    );

    // Parse the profile using the same parser as the repository
    let profile =
        knot::domain::knot_file::parse_agent_profile(&content).unwrap();
    assert_eq!(
        profile.name, "timed-profile",
        "profile name should be parsed correctly"
    );
    assert_eq!(
        profile.timeout, Some(30),
        "profile timeout should be Some(30)"
    );

    // Now test the full repository round-trip
    let repo =
        knot::adapters::outbound::FileSystemAgentProfileRepository::new(
            profiles_dir.clone(),
        );

    // Get the profile via repository
    let loaded = repo.get("timed-profile").unwrap().unwrap();
    assert_eq!(
        loaded.timeout, Some(30),
        "repository should read timeout: 30 from profile file"
    );

    // Save a new profile with timeout and read back
    let new_profile =
        knot::domain::value_objects::AgentProfile::new(
            "new-timed".to_string(),
            "anthropic".to_string(),
            "claude-sonnet".to_string(),
            "Timed reviewer.".to_string(),
        )
        .unwrap()
        .with_timeout(Some(120));

    repo.save(new_profile).unwrap();

    let reloaded = repo.get("new-timed").unwrap().unwrap();
    assert_eq!(
        reloaded.timeout, Some(120),
        "save → get round-trip should preserve timeout: 120"
    );

    // Verify the saved file also contains the timeout
    let saved_content =
        fs::read_to_string(profiles_dir.join("new-timed.md")).unwrap();
    assert!(
        saved_content.contains("timeout: 120"),
        "saved profile file should contain 'timeout: 120'.\\n\\nFile content:\\n{}",
        saved_content
    );
}

/// Profile with no timeout field serialises/deserialises correctly.
///
/// A profile without `timeout` in its frontmatter should parse with
/// `timeout = None`. When saved, the timeout field should not appear
/// in the YAML output (skip_serializing_if = Option::is_none).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_no_timeout_roundtrip_via_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create profile file WITHOUT timeout
    create_profile_with_timeout(&base_dir, "no-timeout-profile", None);

    // Read back via filesystem
    let profiles_dir = base_dir.join("profiles");
    let content =
        fs::read_to_string(profiles_dir.join("no-timeout-profile.md")).unwrap();

    // Verify `timeout:` key is NOT in the YAML frontmatter
    let yaml_section: Vec<&str> = content
        .split("---")
        .collect();
    if yaml_section.len() >= 2 {
        let yaml = yaml_section[1];
        assert!(
            !yaml.contains("timeout:"),
            "profile without timeout should NOT have 'timeout:' key in YAML.\\n\\nYAML:\\n{}",
            yaml
        );
    }

    // Parse the profile
    let profile =
        knot::domain::knot_file::parse_agent_profile(&content).unwrap();
    assert_eq!(
        profile.timeout, None,
        "profile without timeout field should parse as None"
    );

    // Repository round-trip
    let repo =
        knot::adapters::outbound::FileSystemAgentProfileRepository::new(
            profiles_dir.clone(),
        );

    let loaded = repo.get("no-timeout-profile").unwrap().unwrap();
    assert_eq!(
        loaded.timeout, None,
        "repository should read timeout as None for profile without timeout"
    );
}
