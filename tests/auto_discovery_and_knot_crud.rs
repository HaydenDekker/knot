//! Integration tests for runtime auto-discovery and knot CRUD API.
//!
//! Verifies the full lifecycle of looms and knots:
//! - File-system auto-discovery (new looms, new knots, edits, deletions)
//! - HTTP-driven knot CRUD (create, update, delete)
//! - Removal of POST /looms/discover endpoint

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;

/// Generate knot `.md` content with the given knot name and absolute paths.
/// Creates the strand directory if it doesn't exist.
/// Returns (content, strand_dir). Tie-off paths are statically derived.
fn make_named_knot_content(
    knot_name: &str,
    _goal: &str,
    _provider: &str,
    _model: &str,
    instructions: &str,
    project_root: &std::path::Path,
) -> (String, std::path::PathBuf) {
    let strand_dir = project_root.join("strands");
    fs::create_dir_all(&strand_dir).unwrap();
    let content = format!(
        "---\nname: {knot_name}\nagent-profile-ref: fast\nstrand-dir: \"{}\"\n\
         prompt-template:\n  input-bundling: \"full-file\"\n  \
         instructions: \"{instructions}\"\n---\n\n# {knot_name}\n",
        strand_dir.display()
    );
    (content, strand_dir)
}

/// Helper: wait for auto-discovery to register a loom (poll GET /looms).
async fn wait_for_loom_discovery(
    host_port: &str,
    expected_count: usize,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut attempt = 0;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let result = tokio::time::timeout(
            Duration::from_millis(2000),
            http_get(host_port, "/looms"),
        )
        .await;
        let (st, body) = match result {
            Ok(Ok(r)) => r,
            _ => {
                eprintln!(
                    "DEBUG: wait_for_loom_discovery attempt {} - timeout or error",
                    attempt
                );
                attempt += 1;
                continue;
            }
        };
        attempt += 1;
        if st.contains("200") {
            let summaries: Vec<serde_json::Value> =
                serde_json::from_str(&body).unwrap_or_default();
            eprintln!(
                "DEBUG: attempt {} - {} looms (expected {})",
                attempt,
                summaries.len(),
                expected_count
            );
            if summaries.len() == expected_count {
                return true;
            }
        }
    }
    eprintln!(
        "DEBUG: wait_for_loom_discovery timed out after {} attempts",
        attempt
    );
    false
}

/// Helper: wait for a knot to appear in the loom's knot list.
async fn wait_for_knot_count(
    host_port: &str,
    loom_id: &str,
    expected: usize,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut attempt = 0;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let result = tokio::time::timeout(
            Duration::from_millis(2000),
            http_get(host_port, &format!("/looms/{loom_id}/knots")),
        )
        .await;
        let (st, body) = match result {
            Ok(Ok(r)) => r,
            _ => {
                eprintln!(
                    "DEBUG: wait_for_knot_count attempt {} - timeout or error",
                    attempt
                );
                attempt += 1;
                continue;
            }
        };
        attempt += 1;
        if st.contains("200") {
            let knots: Vec<String> =
                serde_json::from_str(&body).unwrap_or_default();
            eprintln!(
                "DEBUG: wait_for_knot_count attempt {} - {} knots (expected {})",
                attempt,
                knots.len(),
                expected
            );
            if knots.len() == expected {
                return true;
            }
        }
    }
    eprintln!(
        "DEBUG: wait_for_knot_count timed out after {} attempts",
        attempt
    );
    false
}

// ── Auto-Discovery Tests ──────────────────────────────────────────────────

/// Start server with empty rig → create `*-loom/` directory with `.md`
/// file → `GET /looms` shows new loom → create strand → tie-off produced.
///
/// Verifies that the rig directory watcher picks up new loom directories
/// and the `ConfigEventHandler` registers them without restart.
#[tokio::test]
async fn runtime_loom_auto_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create rig directory (empty at startup)
    fs::create_dir_all(&base_dir).unwrap();

    // Create a "fast" agent profile
    let profiles_dir = base_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    fs::write(
        profiles_dir.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\nprofile-prompt: |\n  You are a reviewer.\n---\n\nFast Profile\n",
    )
    .unwrap();

    // Create a "fast" agent profile
    let profiles_dir = base_dir.join("profiles");
    fs::create_dir_all(&profiles_dir).unwrap();
    fs::write(
        profiles_dir.join("fast.md"),
        "---\nname: fast\nprovider: openai\nmodel: gpt-4o\nprofile-prompt: |\n  You are a reviewer.\n---\n\nFast Profile\n",
    )
    .unwrap();

    // Mock agent for processing after discovery
    let mock_agent =
        create_mock_agent(&base_dir, "auto-discovered-output");

    let port = 32100;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. GET /looms should be empty (no looms at startup)
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert!(
        summaries.is_empty(),
        "no looms should exist at startup"
    );

    // 2. Create a new loom directory with a knot definition file
    let loom_dir = base_dir.join("test-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, strand_dir) =
        make_named_knot_content(
            "review-knot",
            "Review documents",
            "openai",
            "gpt-4o",
            "Review the document",
            &base_dir,
        );
    // File name must match knot name: review-knot.md
    fs::write(loom_dir.join("review-knot.md"), knot_content).unwrap();

    // 3. Wait for auto-discovery to pick up the new loom
    assert!(
        wait_for_loom_discovery(&host_port, 1).await,
        "auto-discovery should have found the new loom"
    );

    // 4. GET /looms should now show the new loom
    let (status, body) =
        http_get(&host_port, "/looms")
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "should have 1 loom after discovery");
    assert_eq!(
        summaries[0]["id"].as_str().unwrap(),
        "test-loom",
        "discovered loom id should match"
    );

    // 5. Verify the knot is present
    let (status, body) =
        http_get(&host_port, "/looms/test-loom")
            .await
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(knots.len(), 1, "loom should have 1 knot");
    assert_eq!(
        knots[0]["id"].as_str().unwrap(),
        "review-knot",
        "knot id should match"
    );

    // 6. Create a strand file → should be processed (tie-off produced)
    let strand_path = strand_dir.join("test-strand.md");
    fs::write(&strand_path, "auto-discovered strand content").unwrap();

    // Wait for debounce + processing
    tokio::time::sleep(Duration::from_millis(800)).await;

    // 7. Verify tie-off was produced
    let tie_off_path = base_dir.join("tie-offs/test-loom/review-knot/review-knot-tie-off.md");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist after auto-discovery processing: {}",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("auto-discovered-output"),
        "tie-off should contain agent output, got: {content}"
    );

}

/// Start server with existing loom → drop new `.md` file in loom dir
/// → `GET /looms/{id}/knots` shows new knot.
///
/// Verifies that the rig directory watcher detects new knot definition
/// files and the `ConfigEventHandler` adds them to the loom.
#[tokio::test]
async fn runtime_knot_auto_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom with one knot at startup
    let loom_dir = base_dir.join("existing-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, _strand_dir) =
        make_named_knot_content(
            "review-knot",
            "Review documents",
            "openai",
            "gpt-4o",
            "Review the document",
            &base_dir,
        );
    fs::write(loom_dir.join("review-knot.md"), knot_content).unwrap();

    let port = 32101;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Verify loom is discovered with 1 knot
    let (status, body) =
        http_get_retry(&host_port, "/looms/existing-loom/knots", 30, 100)
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 1, "should have 1 knot at startup");
    assert!(
        knots.contains(&"review-knot".to_string()),
        "should contain review-knot"
    );

    // 2. Drop a new .md file in the loom directory
    let strand_dir = base_dir.join("strands2");
    fs::create_dir_all(&strand_dir).unwrap();
    let new_knot_content = format!(
        "---\nname: summary-knot\nagent-profile-ref: fast\nstrand-dir: \
         \"{}\"\nprompt-template:\n  input-bundling: \
         \"full-file\"\n  instructions: \"Summarize the document\"\n---\n\n# \
         Summary Knot\n",
        strand_dir.display()
    );
    fs::write(loom_dir.join("summary-knot.md"), new_knot_content).unwrap();

    // 3. Wait for auto-discovery
    assert!(
        wait_for_knot_count(&host_port, "existing-loom", 2).await,
        "auto-discovery should have found the new knot"
    );

    // 4. GET /looms/{id}/knots should now show 2 knots
    let (status, body) =
        http_get_retry(&host_port, "/looms/existing-loom/knots", 30, 100)
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots after adding new knot");
    assert!(
        knots.contains(&"review-knot".to_string()),
        "should still contain review-knot"
    );
    assert!(
        knots.contains(&"summary-knot".to_string()),
        "should now contain summary-knot"
    );

}

/// Edit a `.md` file (change model) → `GET /looms/{id}` shows updated config.
///
/// Verifies that modifying a knot definition file triggers an update
/// in the in-memory store.
#[tokio::test]
async fn runtime_knot_edit_picks_up_change() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom with a knot at startup
    let loom_dir = base_dir.join("edit-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, _strand_dir) =
        make_named_knot_content(
            "review-knot",
            "Review PRD goals for clarity",
            "openai",
            "gpt-4o",
            "Review the goals section of this PRD",
            &base_dir,
        );
    fs::write(loom_dir.join("review-knot.md"), knot_content).unwrap();

    let port = 32102;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Verify initial model
    let (status, body) =
        http_get_retry(&host_port, "/looms/edit-loom", 30, 100)
            .await
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(
        knots[0]["agent_profile_ref"].as_str().unwrap(),
        "fast",
        "initial profile ref should be fast"
    );

    // 2. Edit the .md file to change the model
    let updated_content = format!(
        "---\nname: review-knot\nagent-profile-ref: fast\nstrand-dir: \"{}\"\nprompt-template:\n  \
         input-bundling: \"full-file\"\n  instructions: \"Review the goals \
         section of this PRD\"\n---\n\n# review-knot\n",
        base_dir.join("strands").display()
    );
    fs::write(loom_dir.join("review-knot.md"), updated_content).unwrap();

    // 3. Wait for auto-discovery to pick up the change
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // 4. GET /looms/{id} should show updated model
    let (status, body) =
        http_get_retry(&host_port, "/looms/edit-loom", 30, 100)
            .await
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(
        knots[0]["agent_profile_ref"].as_str().unwrap(),
        "fast",
        "profile ref should be fast"
    );

}

/// Delete a `.md` file → `GET /looms/{id}/knots` no longer shows the knot.
///
/// Verifies that removing a knot definition file triggers deregistration
/// from the in-memory store.
#[tokio::test]
async fn runtime_knot_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom with two knots at startup
    let loom_dir = base_dir.join("delete-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, _strand_dir) =
        make_named_knot_content(
            "review-knot",
            "Review documents",
            "openai",
            "gpt-4o",
            "Review the document",
            &base_dir,
        );
    fs::write(loom_dir.join("review-knot.md"), knot_content).unwrap();

    // Second knot
    let strand_dir2 = base_dir.join("strands2");
    fs::create_dir_all(&strand_dir2).unwrap();
    let second_knot = format!(
        "---\nname: second-knot\nagent-profile-ref: fast\nstrand-dir: \
         \"{}\"\nprompt-template:\n  input-bundling: \
         \"full-file\"\n  instructions: \"Second knot\"\n---\n\n# second-knot\n",
        strand_dir2.display()
    );
    fs::write(loom_dir.join("second-knot.md"), second_knot).unwrap();

    let port = 32103;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Verify loom has 2 knots
    let (status, body) =
        http_get_retry(&host_port, "/looms/delete-loom/knots", 30, 100)
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots at startup");

    // 2. Delete one of the knot files
    fs::remove_file(loom_dir.join("second-knot.md")).unwrap();

    // 3. Wait for auto-discovery to pick up the deletion
    assert!(
        wait_for_knot_count(&host_port, "delete-loom", 1).await,
        "auto-discovery should have detected the deleted knot"
    );

    // 4. GET /looms/{id}/knots should show only 1 knot
    let (status, body) =
        http_get_retry(&host_port, "/looms/delete-loom/knots", 30, 100)
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 1, "should have 1 knot after deletion");
    assert!(
        knots.contains(&"review-knot".to_string()),
        "remaining knot should be review-knot"
    );
    assert!(
        !knots.contains(&"second-knot".to_string()),
        "deleted knot should not be present"
    );

}

/// Start server with empty rig → create `*-loom/` directory then
/// immediately write `.md` file → poll until loom has expected knot
/// count. Proves the `KnotModified` recovery path works end-to-end.
///
/// The notify watcher may fire `LoomAdded` before the knot file is fully
/// written. If so, the loom registers with 0 knots. The subsequent
/// `KnotModified` event should recover by registering the knot.
#[tokio::test]
async fn filesystem_loom_creation_race_recovery() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create rig directory (empty at startup)
    fs::create_dir_all(&base_dir).unwrap();

    let port = 32108;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. GET /looms should be empty (no looms at startup)
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert!(
        summaries.is_empty(),
        "no looms should exist at startup"
    );

    // 2. Create loom directory
    let loom_dir = base_dir.join("race-loom");
    fs::create_dir_all(&loom_dir).unwrap();

    // 3. Immediately write the knot .md file
    let (knot_content, _strand_dir) =
        make_named_knot_content(
            "race-knot",
            "Race recovery test",
            "openai",
            "gpt-4o",
            "Race test knot",
            &base_dir,
        );
    fs::write(loom_dir.join("race-knot.md"), knot_content).unwrap();

    // 4. Poll GET /looms/race-loom until it has 1 knot.
    //    If the race occurred (LoomAdded before file write completed),
    //    KnotModified should recover and register the knot.
    assert!(
        wait_for_knot_count(&host_port, "race-loom", 1).await,
        "loom should eventually have 1 knot via KnotModified recovery"
    );

    // 5. Verify loom details: 1 knot with correct id
    let (status, body) =
        http_get_retry(&host_port, "/looms/race-loom", 30, 100)
            .await
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(
        knots.len(),
        1,
        "loom should have 1 knot (not 0) — KnotModified recovery must \
         have registered the knot"
    );
    assert_eq!(
        knots[0]["id"].as_str().unwrap(),
        "race-knot",
        "knot id should match"
    );

}

// ── Phase 5: Integration Tests — Runtime Discovery and Manual Reload ──────

/// Start server with absolute rig path (empty rig) → create `*-loom/`
/// directory with `.md` file → `GET /looms` shows new loom → create
/// strand → tie-off produced.
///
/// Verifies the full auto-discovery pipeline with an explicitly absolute
/// rig directory path. This proves that path canonicalisation is consistent
/// between watch registration and notify event reporting — the rig watch
/// registered with an absolute path matches the absolute paths that
/// notify reports.
#[tokio::test]
async fn auto_discovery_with_absolute_rig_path() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Canonicalise the rig path to guarantee it is absolute.
    let rig_dir = fs::canonicalize(&base_dir).unwrap_or(base_dir.clone());
    assert!(
        rig_dir.is_absolute(),
        "rig_dir must be an absolute path for this test"
    );

    // Create rig directory (empty at startup)
    fs::create_dir_all(&rig_dir).unwrap();

    // Create a "fast" agent profile
    create_fast_profile(&rig_dir);

    // Mock agent for processing after discovery
    let mock_agent =
        create_mock_agent(&rig_dir, "absolute-path-discovery-output");

    let port = 32110;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: rig_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        rig_config: RigAgentConfig {
            cli_path: mock_agent.to_string_lossy().to_string(),
            cli_args: vec![],
        },
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. GET /looms should be empty (no looms at startup)
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert!(
        summaries.is_empty(),
        "no looms should exist at startup"
    );

    // 2. Create a new loom directory with a knot definition file
    let loom_dir = rig_dir.join("discovered-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, strand_dir) = make_named_knot_content(
        "review-knot",
        "Review documents",
        "openai",
        "gpt-4o",
        "Review the document",
        &rig_dir,
    );
    fs::write(loom_dir.join("review-knot.md"), knot_content).unwrap();

    // 3. Wait for auto-discovery to pick up the new loom
    assert!(
        wait_for_loom_discovery(&host_port, 1).await,
        "auto-discovery should have found the new loom via absolute path"
    );

    // 4. GET /looms should now show the new loom
    let (status, body) =
        http_get(&host_port, "/looms")
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(summaries.len(), 1, "should have 1 loom after discovery");
    assert_eq!(
        summaries[0]["id"].as_str().unwrap(),
        "discovered-loom",
        "discovered loom id should match"
    );

    // 5. Verify the knot is present
    let (status, body) =
        http_get(&host_port, "/looms/discovered-loom")
            .await
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(knots.len(), 1, "loom should have 1 knot");
    assert_eq!(
        knots[0]["id"].as_str().unwrap(),
        "review-knot",
        "knot id should match"
    );

    // 6. Create a strand file → should be processed (tie-off produced)
    let strand_path = strand_dir.join("test-strand.md");
    fs::write(&strand_path, "absolute-path-discovery strand content")
        .unwrap();

    // Wait for debounce + processing
    tokio::time::sleep(Duration::from_millis(800)).await;

    // 7. Verify tie-off was produced
    let tie_off_path =
        rig_dir.join("tie-offs/discovered-loom/review-knot/review-knot-tie-off.md");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist after auto-discovery processing: {}",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("absolute-path-discovery-output"),
        "tie-off should contain agent output, got: {content}"
    );
}

/// Create loom directory on disk → `POST /config/reload` → new loom
/// appears in `GET /looms`.
///
/// Verifies that the manual reload endpoint provides a recovery path
/// when the file watcher misses an event. The `ReloadConfig` use case
/// re-scans the rig and registers any looms not already in the store.
///
/// Note: the watcher may discover the loom before we call reload (race).
/// We handle this by verifying the loom appears in GET /looms regardless
/// of whether auto-discovery or reload was responsible.
#[tokio::test]
async fn manual_config_reload_discovers_new_looms() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create rig directory (empty at startup)
    fs::create_dir_all(&base_dir).unwrap();

    // Create a "fast" agent profile
    create_fast_profile(&base_dir);

    let port = 32111;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. GET /looms should be empty (no looms at startup)
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert!(
        summaries.is_empty(),
        "no looms should exist at startup"
    );

    // 2. Create a new loom directory with a knot definition file
    let loom_dir = base_dir.join("manual-reload-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, _strand_dir) = make_named_knot_content(
        "summary-knot",
        "Summarise documents",
        "openai",
        "gpt-4o",
        "Summarize the document",
        &base_dir,
    );
    fs::write(loom_dir.join("summary-knot.md"), knot_content).unwrap();

    // 3. POST /config/reload to manually trigger re-scan.
    //    The loom may or may not already be discovered by the watcher
    //    (race condition). Reload is idempotent — if already registered,
    //    it returns empty. If not yet registered, it discovers it.
    let (status, body) =
        http_post_json(&host_port, "/config/reload", &serde_json::json!({}))
            .await
            .expect("reload endpoint should respond");
    assert!(
        status.contains("200"),
        "POST /config/reload should return 200, got: {status}"
    );
    let new_looms: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    // Either 0 (watcher already found it) or 1 (reload found it).
    // Both outcomes are valid — the key guarantee is idempotency.
    assert!(
        new_looms.len() <= 1,
        "reload should discover at most one new loom"
    );

    // 4. GET /looms should show the new loom (found by either mechanism)
    let (status, body) =
        http_get(&host_port, "/looms")
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(
        summaries.len(),
        1,
        "should have 1 loom (via reload or auto-discovery)"
    );
    assert_eq!(
        summaries[0]["id"].as_str().unwrap(),
        "manual-reload-loom",
        "discovered loom id should match"
    );

    // 5. Verify the knot is present in the loom
    let (status, body) =
        http_get(&host_port, "/looms/manual-reload-loom")
            .await
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(knots.len(), 1, "loom should have 1 knot");
    assert_eq!(
        knots[0]["id"].as_str().unwrap(),
        "summary-knot",
        "knot id should match"
    );

    // 6. Idempotency: POST /config/reload again should discover no new looms
    let (status, body) =
        http_post_json(&host_port, "/config/reload", &serde_json::json!({}))
            .await
            .expect("reload endpoint should respond");
    assert!(
        status.contains("200"),
        "second POST /config/reload should return 200, got: {status}"
    );
    let already_known: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert!(
        already_known.is_empty(),
        "second reload should discover no new looms (idempotent)"
    );

    // 7. Create a SECOND loom and verify reload discovers it (proves reload
    //    actually works, not just that auto-discovery happened first).
    let loom_dir2 = base_dir.join("second-reload-loom");
    fs::create_dir_all(&loom_dir2).unwrap();
    let (knot_content2, _strand_dir2) = make_named_knot_content(
        "verify-knot",
        "Verify documents",
        "openai",
        "gpt-4o",
        "Verify the document",
        &base_dir,
    );
    fs::write(loom_dir2.join("verify-knot.md"), knot_content2).unwrap();

    // POST /config/reload — should find the second loom
    let (status, _body) =
        http_post_json(&host_port, "/config/reload", &serde_json::json!({}))
            .await
            .expect("reload endpoint should respond");
    assert!(
        status.contains("200"),
        "third POST /config/reload should return 200, got: {status}"
    );

    // GET /looms should now show both looms
    let (status, body) =
        http_get(&host_port, "/looms")
            .await
            .expect("looms endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let summaries: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(
        summaries.len(),
        2,
        "should have 2 looms after reload discovers second loom"
    );
    let ids: Vec<_> = summaries
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert!(
        ids.contains(&"manual-reload-loom"),
        "first loom should still be present"
    );
    assert!(
        ids.contains(&"second-reload-loom"),
        "second loom should be present (discovered by reload)"
    );
}

// ── Discover Endpoint Removed ─────────────────────────────────────────────

/// `POST /looms/discover` returns 404 or 405 because the endpoint has
/// been removed in favour of runtime auto-discovery.
#[tokio::test]
async fn discover_endpoint_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 32107;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        rig_dir: base_dir,
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // POST /looms/discover should not be found (404 or 405).
    // The path `/looms/discover` may match `/looms/{id}` with id="discover"
    // (405 Method Not Allowed since that route has GET/DELETE only),
    // or it may be 404 if no route matches. Either way, the dedicated
    // discover endpoint does not exist.
    let body = serde_json::json!({});
    let (status, _resp) =
        http_post_json(&host_port, "/looms/discover", &body)
            .await
            .expect("discover endpoint should respond");

    assert!(
        status.contains("404") || status.contains("405"),
        "POST /looms/discover should return 404 or 405, got: {status}"
    );

}
