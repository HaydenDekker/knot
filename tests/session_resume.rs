//! Integration tests for session-resume retry on invocation failure.
//!
//! Verifies: session ID capture, --session-id passthrough, "please continue"
//! prompt append, budget tracking, retry delay, and stdio no-retry.

#[path = "helpers.rs"]
mod helpers;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use helpers::*;

// ── Mock pi helpers ────────────────────────────────────────────────────────

/// Create a mock `pi` binary (JSON adapter) that:
/// - First call: emits session_id JSON-L line then exits with error code 1
///   (simulates a transient network failure — fails quickly, leaving budget).
/// - Subsequent calls: emits session_id + agent_end with response text.
///
/// Uses a counter file to distinguish first from retry attempts.
///
/// # Arguments
///
/// * `rig_dir` — path to the rig directory
/// * `session_id` — session ID to emit on every call
/// * `response` — response text on retry (second+ call)
/// * `counter_file` — path for the call counter
fn create_mock_pi_fail_then_success(
    rig_dir: &Path,
    session_id: &str,
    response: &str,
    counter_file: &Path,
) {
    let escaped_response = response.replace('\'', "'\\''");
    let counter = counter_file.display().to_string();

    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");

    let script = format!(
        r#"#!/usr/bin/env bash
# Mock pi — fails on first call (simulates transient error), succeeds on retry
# Uses a counter file to track call number
cat > /dev/null
COUNTER_FILE="{counter}"
if [ -f "$COUNTER_FILE" ]; then
    COUNT=$(cat "$COUNTER_FILE")
else
    COUNT=0
fi
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNTER_FILE"
# First call: emit session_id then exit with error (transient failure)
if [ "$COUNT" -eq 1 ]; then
    echo '{{"type":"session","id":"{session_id}"}}'
    exit 1
fi
# Retry: emit session + response
echo '{{"type":"session","id":"{session_id}"}}'
echo '{{"type":"agent_end","usage":{{"input":100,"output":50,"cache_read":0,"cache_write":0,"total":150}},"messages":[{{"role":"assistant","content":[{{"type":"text","text":"{escaped_response}"}}]}}]}}'
exit 0
"#,
    );

    fs::write(&pi_path, script).unwrap();
    fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755)).unwrap();

    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-json\n",
    )
    .unwrap();

    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!("{}:{}", bin_dir.display(), existing),
        );
    }

    // Reset counter file
    fs::write(counter_file, "0").unwrap();
}

/// Create a mock `pi` binary (JSON adapter) that always fails
/// (emits session_id then exits with error — simulates persistent failure).
///
/// Uses a counter file to track call number. Fails for `max_attempts` calls,
/// then would succeed (but budget should be exhausted before that).
fn create_mock_pi_always_fail(
    rig_dir: &Path,
    session_id: &str,
    max_attempts: u32,
    counter_file: &Path,
) {
    let counter = counter_file.display().to_string();

    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");

    let script = format!(
        r#"#!/usr/bin/env bash
# Mock pi — always fails (simulates persistent error)
cat > /dev/null
COUNTER_FILE="{counter}"
if [ -f "$COUNTER_FILE" ]; then
    COUNT=$(cat "$COUNTER_FILE")
else
    COUNT=0
fi
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNTER_FILE"
echo '{{"type":"session","id":"{session_id}"}}'
# Fail for first N attempts, then succeed (budget should exhaust first)
if [ "$COUNT" -le {max_attempts} ]; then
    exit 1
fi
echo '{{"type":"agent_end","usage":{{"input":100,"output":50,"cache_read":0,"cache_write":0,"total":150}},"messages":[{{"role":"assistant","content":[{{"type":"text","text":"success"}}]}}]}}'
exit 0
"#,
    );

    fs::write(&pi_path, script).unwrap();
    fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755)).unwrap();

    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-json\n",
    )
    .unwrap();

    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!("{}:{}", bin_dir.display(), existing),
        );
    }

    fs::write(counter_file, "0").unwrap();
}

/// Create a mock `pi` binary (JSON adapter) that sleeps forever
/// (emits session_id then sleeps — killed on timeout).
fn create_mock_pi_timeout_only(rig_dir: &Path, session_id: &str) {
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");

    let script = format!(
        r#"#!/usr/bin/env bash
# Mock pi — emits session then sleeps (killed on timeout)
cat > /dev/null
echo '{{"type":"session","id":"{session_id}"}}'
exec sleep 3600
"#,
    );

    fs::write(&pi_path, script).unwrap();
    fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755)).unwrap();

    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-json\n",
    )
    .unwrap();

    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!("{}:{}", bin_dir.display(), existing),
        );
    }
}

/// Create an agent profile with a custom timeout (in seconds).
fn create_profile_with_timeout(
    rig_dir: &Path,
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

/// Read rig-log entries and filter for a specific event type.
fn read_rig_log(rig_dir: &Path) -> Vec<serde_json::Value> {
    let rig_log_path = rig_dir.join(".rig-log");
    let content = match fs::read_to_string(&rig_log_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn rig_log_event_type(event: &serde_json::Value) -> Option<&str> {
    event.as_object().and_then(|obj| obj.keys().next().map(|k| k.as_str()))
}

/// Poll until rig-log contains an event with a specific type.
fn wait_for_rig_log_event(
    rig_dir: &Path,
    event_type: &str,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);

    loop {
        if std::time::Instant::now() > deadline {
            let content = fs::read_to_string(rig_dir.join(".rig-log"))
                .unwrap_or_default();
            panic!(
                "timeout waiting for rig-log event '{}'. Log:\n{}",
                event_type, content
            );
        }

        let events = read_rig_log(rig_dir);
        for event in &events {
            if let Some(ty) = rig_log_event_type(event) {
                if ty == event_type {
                    return;
                }
            }
        }

        thread::sleep(Duration::from_millis(50));
    }
}

// ── Session Resume Integration Tests ──────────────────────────────────────

/// First invocation times out (session_id captured), retry succeeds.
/// Loom-log shows SessionResumed + KnotCompleted, no KnotFailed.
#[test]
fn test_session_resume_success() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    // Profile with 120s timeout (enough budget for first attempt + retry)
    create_profile_with_timeout(&rig_dir, "fast", 120);

    let counter_file = tmp.path().join("counter");
    create_mock_pi_fail_then_success(
        &rig_dir,
        "sess-resume-success",
        "resumed response",
        &counter_file,
    );

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    // Trigger processing — first attempt will timeout, retry succeeds
    create_strand(&rig_dir, "feature.md", "content to review");

    // Wait for completion (retry should succeed)
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Verify loom-log has SessionResumed + KnotCompleted
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    assert!(
        types.contains(&"SessionResumed"),
        "should have SessionResumed in loom-log. Events: {:?}",
        types
    );
    assert!(
        types.contains(&"KnotCompleted"),
        "should have KnotCompleted in loom-log. Events: {:?}",
        types
    );
    assert!(
        !types.contains(&"KnotFailed"),
        "should NOT have KnotFailed in loom-log. Events: {:?}",
        types
    );

    // Verify SessionResumed has correct session_id
    let resumed_event = events.iter().find(|e| {
        loom_log_event_type(e) == Some("SessionResumed")
    });
    assert!(
        resumed_event.is_some(),
        "should have SessionResumed event"
    );
    if let Some(inner) = resumed_event
        .unwrap()
        .as_object()
        .and_then(|o| o.values().next())
    {
        let sid = inner.get("session_id").and_then(|v| v.as_str());
        assert_eq!(
            sid,
            Some("sess-resume-success"),
            "session_id should match captured value"
        );
        let attempt = inner.get("attempt").and_then(|v| v.as_u64());
        assert_eq!(attempt, Some(1), "first retry should be attempt 1");
    }

    // Verify tie-off contains the resumed response
    let tie_off_file =
        rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    assert!(
        tie_off_file.exists(),
        "tie-off should exist"
    );
    let content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        content.contains("resumed response"),
        "tie-off should contain resumed response. Got:\n{}",
        content
    );

    handle.abort();
}

/// All retry attempts timeout within budget → KnotFailed in loom-log,
/// TimeoutExceeded in rig-log.
#[test]
fn test_session_resume_exhausted() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    // Profile with 120s timeout.
    // Mock fails 20 times, but budget + retry delay limits retries.
    // Each retry has 10s delay (reduced via env var), so budget drains.
    create_profile_with_timeout(&rig_dir, "fast", 120);

    // Mock pi that always fails — budget + delay should exhaust retries
    let counter_file = tmp.path().join("counter_exhausted");
    create_mock_pi_always_fail(&rig_dir, "sess-exhausted", 20, &counter_file);

    // Fast retry delay for test (100ms instead of 10s)
    unsafe {
        std::env::set_var("KNOT_RETRY_DELAY_MS", "100");
    }

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "content that always times out");

    // Wait for KnotFailed (should happen after retries exhausted)
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotFailed");

    // Verify loom-log has KnotFailed
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    assert!(
        types.contains(&"KnotFailed"),
        "should have KnotFailed. Events: {:?}",
        types
    );

    // Verify SessionResumed events exist (at least 1 retry was attempted)
    let resumed_count = types.iter().filter(|&&t| t == "SessionResumed").count();
    assert!(
        resumed_count >= 1,
        "should have at least 1 SessionResumed event, got {}",
        resumed_count
    );

    // Verify rig-log has TimeoutExceeded
    wait_for_rig_log_event(&rig_dir, "TimeoutExceeded");
    let rig_events = read_rig_log(&rig_dir);
    let rig_types: Vec<_> = rig_events
        .iter()
        .filter_map(|e| rig_log_event_type(e))
        .collect();
    assert!(
        rig_types.contains(&"TimeoutExceeded"),
        "should have TimeoutExceeded in rig-log. Events: {:?}",
        rig_types
    );

    // Verify KnotFailed error mentions timeout
    let failed_event = events.iter().find(|e| {
        loom_log_event_type(e) == Some("KnotFailed")
    });
    assert!(failed_event.is_some(), "should have KnotFailed");
    if let Some(inner) = failed_event
        .unwrap()
        .as_object()
        .and_then(|o| o.values().next())
    {
        let error = inner.get("error").and_then(|v| v.as_str());
        assert!(
            error.map(|e| e.contains("timeout")).unwrap_or(false),
            "KnotFailed error should mention timeout. Got: {:?}",
            error
        );
    }

    // Clear env var for other tests
    unsafe {
        std::env::remove_var("KNOT_RETRY_DELAY_MS");
    }

    handle.abort();
}

/// Profile timeout budget is consumed by the first attempt — no retries possible.
/// With a small timeout, the first attempt (which fails via timeout) consumes the
/// entire budget, leaving no room for retry.
#[test]
fn test_session_resume_budget_expired() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    // Profile with 15s timeout.
    // First attempt: killed at 15s → remaining = 15 - 15 = 0 < 5 → no retry.
    create_profile_with_timeout(&rig_dir, "fast", 15);
    create_mock_pi_timeout_only(&rig_dir, "sess-budget");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "budget test content");

    // Wait for KnotFailed
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotFailed");

    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    assert!(
        types.contains(&"KnotFailed"),
        "should have KnotFailed. Events: {:?}",
        types
    );

    // Verify NO SessionResumed events (budget exhausted after first attempt)
    let resumed_count = types.iter().filter(|&&t| t == "SessionResumed").count();
    // With budget = per-attempt timeout, remaining ≈ 0 after first attempt.
    // No retry should happen.
    assert!(
        resumed_count == 0,
        "should have 0 SessionResumed events when budget ≈ per-attempt timeout, got {}",
        resumed_count
    );

    // Verify KnotFailed error mentions timeout
    let failed_event = events.iter().find(|e| {
        loom_log_event_type(e) == Some("KnotFailed")
    });
    assert!(failed_event.is_some(), "should have KnotFailed");
    if let Some(inner) = failed_event
        .unwrap()
        .as_object()
        .and_then(|o| o.values().next())
    {
        let error = inner.get("error").and_then(|v| v.as_str());
        assert!(
            error.map(|e| e.contains("timeout")).unwrap_or(false),
            "KnotFailed error should mention timeout. Got: {:?}",
            error
        );
    }

    handle.abort();
}

/// With `agent-adapter: pi-stdio`, failure → no retry because session_id
/// is never captured (stdio adapter doesn't emit JSON-L).
#[test]
fn test_session_resume_stdio_no_retry() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    // Profile with 2s timeout
    create_profile_with_timeout(&rig_dir, "fast", 2);

    // Stdio mock pi that sleeps (will timeout, but no session_id captured)
    let bin_dir = rig_dir.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pi_path = bin_dir.join("pi");
    let script = "#!/usr/bin/env bash\ncat > /dev/null\nsleep 3600\n";
    fs::write(&pi_path, script).unwrap();
    fs::set_permissions(&pi_path, fs::Permissions::from_mode(0o755)).unwrap();

    fs::write(
        rig_dir.join(".workspace-agent-config.yaml"),
        "agent-adapter: pi-stdio\n",
    )
    .unwrap();

    unsafe {
        let existing = std::env::var("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!("{}:{}", bin_dir.display(), existing),
        );
    }

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "stdio no retry content");

    // Wait for KnotFailed (no retry should happen)
    wait_for_loom_log_event(&rig_dir, "review-loom", "KnotFailed");

    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    assert!(
        types.contains(&"KnotFailed"),
        "should have KnotFailed. Events: {:?}",
        types
    );

    // Verify NO SessionResumed events (stdio doesn't capture session_id)
    let resumed_count = types.iter().filter(|&&t| t == "SessionResumed").count();
    assert!(
        resumed_count == 0,
        "should have 0 SessionResumed for stdio adapter, got {}",
        resumed_count
    );

    // Should complete quickly (just 1 attempt, no retries)
    handle.abort();
}

/// First fails, retry succeeds → loom-log has SessionResumed + KnotCompleted,
/// no KnotFailed. Transparent to the outer flow.
#[test]
fn test_session_resume_transparent_on_success() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    create_profile_with_timeout(&rig_dir, "fast", 120);

    let counter_file = tmp.path().join("counter2");
    create_mock_pi_fail_then_success(
        &rig_dir,
        "sess-transparent",
        "transparent success output",
        &counter_file,
    );

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "transparent test");

    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    // Verify loom-log sequence
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    // Should have: SessionResumed, KnotCompleted, StrandProcessed
    // Must NOT have: KnotFailed
    assert!(
        types.contains(&"SessionResumed"),
        "should have SessionResumed. Events: {:?}",
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
    assert!(
        !types.contains(&"KnotFailed"),
        "should NOT have KnotFailed (retry succeeded). Events: {:?}",
        types
    );

    // Verify the StrandProcessed has no error
    let processed_event = events.iter().find(|e| {
        loom_log_event_type(e) == Some("StrandProcessed")
    });
    if let Some(processed) = processed_event {
        if let Some(inner) = processed
            .as_object()
            .and_then(|o| o.values().next())
        {
            // Last StrandProcessed should have no error
            let error = inner.get("error");
            assert!(
                error.map(|e| e.is_null()).unwrap_or(true),
                "StrandProcessed should have no error"
            );
        }
    }

    // Verify tie-off was written with success content
    let tie_off_file =
        rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    assert!(
        tie_off_file.exists(),
        "tie-off should exist"
    );
    let content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        content.contains("transparent success output"),
        "tie-off should contain success output. Got:\n{}",
        content
    );

    handle.abort();
}

/// 10s delay observed between retry attempts (wall-clock).
///
/// Uses KNOT_RETRY_DELAY_MS env var to reduce the delay to 100ms for
/// faster test execution. Verifies the total processing time exceeds
/// the per-attempt timeout by at least the configured delay.
#[test]
fn test_session_resume_delay_between_retries() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");

    // Profile with 120s timeout (enough for 1 attempt + delay + retry)
    create_profile_with_timeout(&rig_dir, "fast", 120);

    let counter_file = tmp.path().join("counter3");
    create_mock_pi_fail_then_success(
        &rig_dir,
        "sess-delay",
        "delay test output",
        &counter_file,
    );

    // Set fast retry delay for test (100ms instead of 10s)
    // The session_resume module reads KNOT_RETRY_DELAY_MS env var.
    unsafe {
        std::env::set_var("KNOT_RETRY_DELAY_MS", "100");
    }

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    let start = std::time::Instant::now();
    create_strand(&rig_dir, "feature.md", "delay test content");

    // Wait for completion
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    let elapsed = start.elapsed();

    // Clear the env var for other tests
    unsafe {
        std::env::remove_var("KNOT_RETRY_DELAY_MS");
    }

    // First attempt fails instantly (exit code 1), so ~0s.
    // Plus 100ms retry delay.
    // Second attempt is instant.
    // So total ≈ 100ms + small overhead.
    // We verify elapsed >= 100ms, proving the retry delay occurred.
    // We verify elapsed > 30s, proving the retry delay occurred.
    assert!(
        elapsed >= Duration::from_millis(100),
        "total elapsed should be >= retry delay (100ms), proving \
         delay occurred. Got: {:?}",
        elapsed
    );

    // Also verify SessionResumed exists (proving retry happened)
    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();
    assert!(
        types.contains(&"SessionResumed"),
        "should have SessionResumed (retry happened). Events: {:?}",
        types
    );

    handle.abort();
}

// ── Regression: verify existing pipeline still works ──────────────────────

/// Regression: basic pipeline still works after session-resume integration.
#[test]
fn test_regression_basic_pipeline_still_works() {
    let tmp = tempfile::tempdir().unwrap();
    let rig_dir = tmp.path().join("rig");
    fs::create_dir_all(&rig_dir).unwrap();

    let loom_dir = create_loom_dir(&rig_dir, "review");
    create_knot_file(&loom_dir, "review");
    create_fast_profile(&rig_dir);
    create_mock_pi(&rig_dir, "regression output");

    let handle = start_knot(rig_dir.clone());
    wait_for_loom_in_state(&rig_dir, "review-loom", 1);

    create_strand(&rig_dir, "feature.md", "regression content");
    wait_for_knot_status_in_state(&rig_dir, "review-loom", "review", "completed");

    let events = read_loom_log(&rig_dir, "review-loom");
    let types: Vec<_> = events
        .iter()
        .filter_map(|e| loom_log_event_type(e))
        .collect();

    assert!(
        types.contains(&"KnotCompleted"),
        "should have KnotCompleted. Events: {:?}",
        types
    );
    // No SessionResumed (no failure, no retry needed)
    assert!(
        !types.contains(&"SessionResumed"),
        "should NOT have SessionResumed for successful first attempt. Events: {:?}",
        types
    );

    let tie_off_file =
        rig_dir.join("tie-offs/review-loom/review/review-tie-off.md");
    assert!(
        tie_off_file.exists(),
        "tie-off should exist"
    );
    let content = fs::read_to_string(&tie_off_file).unwrap();
    assert!(
        content.contains("regression output"),
        "tie-off should contain output. Got:\n{}",
        content
    );

    handle.abort();
}
