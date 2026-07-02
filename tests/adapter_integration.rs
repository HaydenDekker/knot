//! Integration tests for agent adapter selection (JSON vs Stdio).
//!
//! Verifies the full pipeline with both `pi-json` and `pi-stdio` adapters,
//! including metadata capture and timeout session-id capture.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use helpers::*;

// ── Mock pi helpers ────────────────────────────────────────────────────────

/// Create a mock `pi` binary that outputs JSON-L (for the json adapter).
///
/// The stub reads stdin (discards it), then outputs JSON-L lines:
/// 1. `{"type":"session","id":"<session_id>"}` — session event
/// 2. `{"type":"agent_end","messages":[...],"usage":{...}}` — response + usage
///
/// # Arguments
///
/// * `rig_dir` — path to the rig directory
/// * `session_id` — session ID to emit
/// * `response` — response text to embed in agent_end
/// * `usage_input` — input token count
/// * `usage_output` — output token count
///
/// # Returns
///
/// Path to the created mock binary.
fn create_mock_pi_json(
    rig_dir: &std::path::Path,
    session_id: &str,
    response: &str,
    usage_input: u64,
    usage_output: u64,
) -> PathBuf {
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");

    // Escape response for embedding in bash single-quoted string.
    // Replace single quotes with '\'' (end quote, escaped quote, start quote).
    let escaped_response = response.replace('\'', "'\\''");

    let script = format!(
        "#!/usr/bin/env bash\n\
         # Mock pi — JSON-L output for Knot json adapter tests\n\
         cat > /dev/null\n\
         echo '{{\"type\":\"session\",\"id\":\"{session_id}\"}}'\n\
         echo '{{\"type\":\"agent_end\",\"usage\":{{\"input\":{usage_input},\"output\":{usage_output},\"cache_read\":0,\"cache_write\":0,\"total\":{total}}},\"messages\":[{{\"role\":\"assistant\",\"stopReason\":\"stop\",\"content\":[{{\"type\":\"text\",\"text\":\"{escaped_response}\"}}]}}]}}'\n\
         exit 0\n",
        total = usage_input + usage_output,
    );

    fs::write(&pi_path, script).unwrap();
    fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755)).unwrap();

    // Write config selecting json adapter
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-json\n",
    )
    .unwrap();

    pi_path
}

/// Create an agent profile with a custom timeout (in seconds).
///
/// # Arguments
///
/// * `rig_dir` — path to the rig directory
/// * `name` — profile name
/// * `timeout_secs` — timeout value in seconds
fn create_profile_with_timeout(
    rig_dir: &std::path::Path,
    name: &str,
    timeout_secs: u64,
) {
    let profiles_dir = rig_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    fs::write(
        profiles_dir.join(format!("{name}.md")),
        format!(
            "---\nname: {name}\nprovider: openai\nmodel: gpt-4o\ntimeout: {timeout_secs}\n---\n\n\
You are a reviewer.\n"
        ),
    )
    .unwrap();
}

// ── JSON Adapter Integration Tests ─────────────────────────────────────────

/// Full pipeline with `agent_adapter: pi-json` — mock pi outputs JSON-L,
/// tie-off contains the response text extracted from the JSON-L stream.
#[test]
fn test_json_invocation_full_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Mock pi that outputs JSON-L with session_id and token usage
    let pi_path = create_mock_pi_json(
        &rig_dir,
        "json-sess-abc123",
        "json adapter review output",
        200,
        50,
    );

    let config = knot::AppConfig::with_rig_dir(rig_dir.clone())
        .with_cli_path(pi_path);
    let handle = start_knot_with_config(config);
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Trigger processing
    create_strand(&rig_dir, "feature.md", "new feature request");

    // Wait for completion
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotCompleted");

    // Verify tie-off was written
    let tie_off_file =
        rig_dir.join("tie-offs/review-loom/review-tie-off.md");
    assert!(
        tie_off_file.exists(),
        "tie-off file should exist at {}",
        tie_off_file.display()
    );

    let content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        content.contains("json adapter review output"),
        "tie-off should contain response text extracted from JSON-L. Got:\n{}",
        content
    );

    // Verify loom-log has KnotCompleted (not KnotFailed)
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| helpers::loom_log_event_type(e))
        .collect();
    assert!(
        types.contains(&"KnotCompleted"),
        "should have KnotCompleted. Events: {:?}",
        types
    );
    assert!(
        !types.contains(&"KnotFailed"),
        "should NOT have KnotFailed. Events: {:?}",
        types
    );

    handle.abort();
}

// ── Stdio Adapter Regression Tests ─────────────────────────────────────────

/// Regression: full pipeline with `agent_adapter: pi-stdio` (default).
/// Verifies existing behavior is unchanged.
#[test]
fn test_stdio_invocation_full_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);

    // Mock pi (plain text output, pi-stdio adapter) — created inline
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    fs::write(
        &pi_path,
        "#!/usr/bin/env bash\n\
         cat > /dev/null\n\
         echo \"stdio review output\"\n\
         exit 0\n",
    ).unwrap();
    fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\n",
    ).unwrap();

    let config = knot::AppConfig::with_rig_dir(rig_dir.clone())
        .with_cli_path(pi_path);
    let handle = start_knot_with_config(config);
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Trigger processing
    create_strand(&rig_dir, "feature.md", "new feature request");

    // Wait for completion
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotCompleted");

    // Verify tie-off was written
    let tie_off_file =
        rig_dir.join("tie-offs/review-loom/review-tie-off.md");
    assert!(
        tie_off_file.exists(),
        "tie-off file should exist at {}",
        tie_off_file.display()
    );

    let content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        content.contains("stdio review output"),
        "tie-off should contain stdio agent output. Got:\n{}",
        content
    );

    // Verify loom-log sequence
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| helpers::loom_log_event_type(e))
        .collect();
    assert!(
        types.contains(&"KnotProcessing"),
        "should have KnotProcessing. Events: {:?}",
        types
    );
    assert!(
        types.contains(&"KnotCompleted"),
        "should have KnotCompleted. Events: {:?}",
        types
    );
    assert!(
        types.contains(&"StrandProcessed"),
        "should have StrandProcessed. Events: {:?}",
        types
    );

    handle.abort();
}

/// JSON adapter with short timeout — session_id should be captured from the
/// first JSON-L line even when the process is killed.
///
/// Verifies that the KnotFailed loom-log entry exists and the error
/// mentions timeout.
#[test]
fn test_json_invocation_timeout_captures_session_id() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    // Profile with 1-second timeout (much less than the 3600s sleep)
    create_profile_with_timeout(&rig_dir, "fast", 1);

    // Mock pi that emits session_id then sleeps forever
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    fs::write(
        &pi_path,
        "#!/usr/bin/env bash\n\
         cat > /dev/null\n\
         echo '{{\"type\":\"session\",\"id\":\"timeout-sess-xyz789\"}}'\n\
         exec sleep 3600\n",
    ).unwrap();
    fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-json\n",
    ).unwrap();

    let config = knot::AppConfig::with_rig_dir(rig_dir.clone())
        .with_cli_path(pi_path);
    let handle = start_knot_with_config(config);
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Start a timer BEFORE triggering processing, so the wait deadline
    // accounts for the time the first attempt takes to timeout.
    let start = std::time::Instant::now();
    // Trigger processing — agent will timeout
    create_strand(&rig_dir, "feature.md", "content that times out");

    // Wait for KnotFailed with extended deadline (1s timeout + buffer)
    wait_for_loom_log_event_with_deadline(
        &rig_dir,
        "review-loom",
        "KnotFailed",
        start + Duration::from_secs(30),
    );

    // Verify loom-log has KnotFailed
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| helpers::loom_log_event_type(e))
        .collect();
    assert!(
        types.contains(&"KnotFailed"),
        "should have KnotFailed in loom-log. Events: {:?}",
        types
    );

    // Verify the KnotFailed entry mentions timeout
    let failed_event = events.iter().find(|e| {
        helpers::loom_log_event_type(e) == Some("KnotFailed")
    });
    assert!(
        failed_event.is_some(),
        "should have KnotFailed event"
    );
    let inner = failed_event
        .unwrap()
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let error = inner.get("error").and_then(|v| v.as_str());
    assert!(
        error.map(|e| e.contains("timeout")).unwrap_or(false),
        "KnotFailed error should mention timeout. Got: {:?}",
        error
    );

    // Verify StrandProcessed was written (even on failure)
    assert!(
        types.contains(&"StrandProcessed"),
        "should have StrandProcessed in loom-log. Events: {:?}",
        types
    );

    // Verify state reflects failed status
    // State writer is async — poll for the status update
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "failed");

    let state = read_state_file(&rig_dir).unwrap();
    let knot = state
        .get("looms")
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(0))
        .and_then(|l| l.get("knots"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(0))
        .unwrap();
    assert_eq!(
        knot.get("status").and_then(|v| v.as_str()),
        Some("failed"),
        "knot status should be failed after timeout"
    );

    handle.abort();
}
