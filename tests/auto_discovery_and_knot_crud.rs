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
/// Creates the strand and tie-off directories if they don't exist.
/// Returns (content, strand_dir, tie_off_dir).
fn make_named_knot_content(
    knot_name: &str,
    goal: &str,
    provider: &str,
    model: &str,
    instructions: &str,
    project_root: &std::path::Path,
) -> (String, std::path::PathBuf, std::path::PathBuf) {
    let strand_dir = project_root.join("strands");
    let tie_off_dir = project_root.join("tie-offs");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();
    let content = format!(
        "---\nname: {knot_name}\nagent-config:\n  goal: \"{goal}\"\n  \
         provider: \"{provider}\"\n  model: \"{model}\"\nstrand-dir: \"{}\"\n\
         tie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \"full-file\"\n  \
         instructions: \"{instructions}\"\n---\n\n# {knot_name}\n",
        strand_dir.display(),
        tie_off_dir.display()
    );
    (content, strand_dir, tie_off_dir)
}

// ── Auto-Discovery Tests ──────────────────────────────────────────────────

/// Start server with empty rig → create `*-loom/` directory with `.md`
/// file → `GET /looms` shows new loom → create strand → tie-off produced.
///
/// Verifies that the rig directory watcher picks up new loom directories
/// and the `ConfigEventHandler` registers them without restart.
#[test]
fn runtime_loom_auto_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create rig directory (empty at startup)
    fs::create_dir_all(&base_dir).unwrap();

    // Mock agent for processing after discovery
    let mock_agent =
        create_mock_agent(&base_dir, "auto-discovered-output");

    let port = 32100;
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

    // 1. GET /looms should be empty (no looms at startup)
    let (status, body) =
        http_get_retry(&host_port, "/looms", 30, 100)
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
    let (knot_content, strand_dir, tie_off_dir) =
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
    eprintln!(
        "DEBUG: Created loom at {:?}, knot file exists: {}",
        loom_dir,
        loom_dir.join("review-knot.md").exists()
    );

    // 3. Wait for auto-discovery to pick up the new loom
    // Use a loop to check progressively
    for attempt in 0..20 {
        std::thread::sleep(Duration::from_millis(200));
        let (st, body) = match http_get(&host_port, "/looms") {
            Ok(r) => r,
            Err(e) => {
                eprintln!("DEBUG: attempt {} - GET failed: {}", attempt, e);
                continue;
            }
        };
        if st.contains("200") {
            let summaries: Vec<serde_json::Value> =
                serde_json::from_str(&body).unwrap_or_default();
            eprintln!(
                "DEBUG: attempt {} - {} looms found",
                attempt,
                summaries.len()
            );
            if summaries.len() == 1 {
                // Success - break out
                break;
            }
        }
    }

    // 4. GET /looms should now show the new loom
    let (status, body) =
        http_get(&host_port, "/looms")
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
    std::thread::sleep(Duration::from_millis(800));

    // 7. Verify tie-off was produced
    let tie_off_path = tie_off_dir.join("test-strand.md.output");
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

    let _ = shutdown.send(());
}

/// Start server with existing loom → drop new `.md` file in loom dir
/// → `GET /looms/{id}/knots` shows new knot.
///
/// Verifies that the rig directory watcher detects new knot definition
/// files and the `ConfigEventHandler` adds them to the loom.
#[test]
fn runtime_knot_auto_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom with one knot at startup
    let loom_dir = base_dir.join("existing-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, _strand_dir, _tie_off_dir) =
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
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Verify loom is discovered with 1 knot
    let (status, body) =
        http_get_retry(&host_port, "/looms/existing-loom/knots", 30, 100)
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
    let tie_off_dir = base_dir.join("tie-offs2");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();
    let new_knot_content = format!(
        "---\nname: summary-knot\nagent-config:\n  goal: \"Summarize \
         content\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nstrand-dir: \
         \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \
         \"full-file\"\n  instructions: \"Summarize the document\"\n---\n\n# \
         Summary Knot\n",
        strand_dir.display(),
        tie_off_dir.display()
    );
    fs::write(loom_dir.join("summary-knot.md"), new_knot_content).unwrap();

    // 3. Wait for auto-discovery
    std::thread::sleep(Duration::from_millis(2000));

    // 4. GET /looms/{id}/knots should now show 2 knots
    let (status, body) =
        http_get_retry(&host_port, "/looms/existing-loom/knots", 30, 100)
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

    let _ = shutdown.send(());
}

/// Edit a `.md` file (change model) → `GET /looms/{id}` shows updated config.
///
/// Verifies that modifying a knot definition file triggers an update
/// in the in-memory store.
#[test]
fn runtime_knot_edit_picks_up_change() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom with a knot at startup
    let loom_dir = base_dir.join("edit-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, _strand_dir, _tie_off_dir) =
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
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Verify initial model
    let (status, body) =
        http_get_retry(&host_port, "/looms/edit-loom", 30, 100)
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(
        knots[0]["agent_config"]["model"].as_str().unwrap(),
        "gpt-4o",
        "initial model should be gpt-4o"
    );

    // 2. Edit the .md file to change the model
    let updated_content = format!(
        "---\nname: review-knot\nagent-config:\n  goal: \"Review PRD goals \
         for clarity\"\n  provider: \"anthropic\"\n  model: \"claude-sonnet\"\n\
         strand-dir: \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  \
         input-bundling: \"full-file\"\n  instructions: \"Review the goals \
         section of this PRD\"\n---\n\n# review-knot\n",
        base_dir.join("strands").display(),
        base_dir.join("tie-offs").display()
    );
    fs::write(loom_dir.join("review-knot.md"), updated_content).unwrap();

    // 3. Wait for auto-discovery to pick up the change
    std::thread::sleep(Duration::from_millis(2000));

    // 4. GET /looms/{id} should show updated model
    let (status, body) =
        http_get_retry(&host_port, "/looms/edit-loom", 30, 100)
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(
        knots[0]["agent_config"]["model"].as_str().unwrap(),
        "claude-sonnet",
        "updated model should be claude-sonnet"
    );
    assert_eq!(
        knots[0]["agent_config"]["provider"].as_str().unwrap(),
        "anthropic",
        "updated provider should be anthropic"
    );

    let _ = shutdown.send(());
}

/// Delete a `.md` file → `GET /looms/{id}/knots` no longer shows the knot.
///
/// Verifies that removing a knot definition file triggers deregistration
/// from the in-memory store.
#[test]
fn runtime_knot_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom with two knots at startup
    let loom_dir = base_dir.join("delete-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, _strand_dir, _tie_off_dir) =
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
    let tie_off_dir2 = base_dir.join("tie-offs2");
    fs::create_dir_all(&strand_dir2).unwrap();
    fs::create_dir_all(&tie_off_dir2).unwrap();
    let second_knot = format!(
        "---\nname: second-knot\nagent-config:\n  goal: \"Second knot \
         goal\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nstrand-dir: \
         \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \
         \"full-file\"\n  instructions: \"Second knot\"\n---\n\n# second-knot\n",
        strand_dir2.display(),
        tie_off_dir2.display()
    );
    fs::write(loom_dir.join("second-knot.md"), second_knot).unwrap();

    let port = 32103;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Verify loom has 2 knots
    let (status, body) =
        http_get_retry(&host_port, "/looms/delete-loom/knots", 30, 100)
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots at startup");

    // 2. Delete one of the knot files
    fs::remove_file(loom_dir.join("second-knot.md")).unwrap();

    // 3. Wait for auto-discovery to pick up the deletion
    std::thread::sleep(Duration::from_millis(2000));

    // 4. GET /looms/{id}/knots should show only 1 knot
    let (status, body) =
        http_get_retry(&host_port, "/looms/delete-loom/knots", 30, 100)
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

    let _ = shutdown.send(());
}

// ── HTTP Knot CRUD Tests ──────────────────────────────────────────────────

/// `POST /looms/{id}/knots` creates a new knot → 201 → knot appears in
/// `GET /looms/{id}/knots` → `.md` file on disk → create strand →
/// tie-off produced.
#[test]
fn http_create_knot() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom with one knot at startup
    let loom_dir = base_dir.join("knot-crud-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, _strand_dir, _tie_off_dir) =
        make_named_knot_content(
            "review-knot",
            "Review documents",
            "openai",
            "gpt-4o",
            "Review the document",
            &base_dir,
        );
    fs::write(loom_dir.join("review-knot.md"), knot_content).unwrap();

    // Mock agent for processing
    let mock_agent =
        create_mock_agent(&base_dir, "crud-created-output");

    let port = 32104;
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

    // 1. Verify initial state: 1 knot
    let (status, body) =
        http_get_retry(&host_port, "/looms/knot-crud-loom/knots", 30, 100)
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 1, "should have 1 knot at startup");

    // 2. POST /looms/{id}/knots to create a new knot
    let strand_dir = base_dir.join("crud-strands");
    let tie_off_dir = base_dir.join("crud-tie-offs");
    fs::create_dir_all(&strand_dir).unwrap();
    fs::create_dir_all(&tie_off_dir).unwrap();

    let body = serde_json::json!({
        "name": "new-knot",
        "agent_config": {
            "goal": "Process new content",
            "provider": "openai",
            "model": "gpt-4o-mini",
            "tools": []
        },
        "prompt_template": {
            "input_bundling": "full-file",
            "instructions": "Process the document"
        },
        "strand_dir": strand_dir.to_string_lossy(),
        "tie_off_dir": tie_off_dir.to_string_lossy()
    });

    let (status, _resp) =
        http_post_json(&host_port, "/looms/knot-crud-loom/knots", &body)
            .expect("create knot should respond");
    assert!(
        status.contains("201"),
        "create knot should return 201, got: {status}"
    );

    // 3. Verify knot appears in GET /looms/{id}/knots
    let (status, body) =
        http_get_retry(&host_port, "/looms/knot-crud-loom/knots", 30, 100)
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots after creation");
    assert!(
        knots.contains(&"new-knot".to_string()),
        "new knot should be present"
    );

    // 4. Verify .md file was written on disk
    let knot_file = loom_dir.join("new-knot.md");
    assert!(
        knot_file.exists(),
        "knot .md file should exist on disk"
    );

    // 5. Create a strand → should be processed
    let strand_path = strand_dir.join("crud-strand.md");
    fs::write(&strand_path, "crud strand content").unwrap();
    std::thread::sleep(Duration::from_millis(800));

    // 6. Verify tie-off
    let tie_off_path = tie_off_dir.join("crud-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );

    let _ = shutdown.send(());
}

/// `PATCH /looms/{id}/knots/{name}` updates knot config → 200 →
/// `GET /looms/{id}` shows new model → `.md` file updated on disk.
#[test]
fn http_update_knot() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom with one knot at startup
    let loom_dir = base_dir.join("update-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, strand_dir, tie_off_dir) =
        make_named_knot_content(
            "review-knot",
            "Review PRD goals for clarity",
            "openai",
            "gpt-4o",
            "Review the goals section of this PRD",
            &base_dir,
        );
    fs::write(loom_dir.join("review-knot.md"), knot_content).unwrap();

    let port = 32105;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Verify initial model
    let (status, body) =
        http_get_retry(&host_port, "/looms/update-loom", 30, 100)
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(
        knots[0]["agent_config"]["model"].as_str().unwrap(),
        "gpt-4o",
        "initial model should be gpt-4o"
    );

    // 2. PATCH /looms/{id}/knots/{name} with new model
    let body = serde_json::json!({
        "name": "review-knot",
        "agent_config": {
            "goal": "Updated goal",
            "provider": "anthropic",
            "model": "claude-3-opus",
            "tools": []
        },
        "prompt_template": {
            "input_bundling": "full-file",
            "instructions": "Updated instructions"
        },
        "strand_dir": strand_dir.to_string_lossy(),
        "tie_off_dir": tie_off_dir.to_string_lossy()
    });

    let (status, _resp) = http_patch_json(
        &host_port,
        "/looms/update-loom/knots/review-knot",
        &body,
    )
    .expect("update knot should respond");
    assert!(
        status.contains("200"),
        "update knot should return 200, got: {status}"
    );

    // 3. GET /looms/{id} should show updated model
    let (status, body) =
        http_get_retry(&host_port, "/looms/update-loom", 30, 100)
            .expect("get loom should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let loom: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knots = loom["knots"].as_array().unwrap();
    assert_eq!(
        knots[0]["agent_config"]["model"].as_str().unwrap(),
        "claude-3-opus",
        "updated model should be claude-3-opus"
    );
    assert_eq!(
        knots[0]["agent_config"]["provider"].as_str().unwrap(),
        "anthropic",
        "updated provider should be anthropic"
    );

    // 4. Verify .md file was updated on disk (handler writes review-knot.md)
    let knot_file = loom_dir.join("review-knot.md");
    let file_content =
        fs::read_to_string(&knot_file).expect("should read knot file");
    assert!(
        file_content.contains("claude-3-opus"),
        "knot .md file should contain updated model, got: {}",
        file_content
    );
    assert!(
        file_content.contains("anthropic"),
        "knot .md file should contain updated provider, got: {}",
        file_content
    );

    let _ = shutdown.send(());
}

/// `DELETE /looms/{id}/knots/{name}` → 204 → knot no longer in
/// `GET /looms/{id}/knots` → `.md` file deleted on disk.
#[test]
fn http_delete_knot() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // Create a loom with two knots at startup
    let loom_dir = base_dir.join("del-loom");
    fs::create_dir_all(&loom_dir).unwrap();
    let (knot_content, _strand_dir, _tie_off_dir) =
        make_named_knot_content(
            "review-knot",
            "Review documents",
            "openai",
            "gpt-4o",
            "Review the document",
            &base_dir,
        );
    fs::write(loom_dir.join("review-knot.md"), knot_content).unwrap();

    // Second knot (file name must match knot name)
    let strand_dir2 = base_dir.join("strands2");
    let tie_off_dir2 = base_dir.join("tie-offs2");
    fs::create_dir_all(&strand_dir2).unwrap();
    fs::create_dir_all(&tie_off_dir2).unwrap();
    let second_knot = format!(
        "---\nname: to-delete-knot\nagent-config:\n  goal: \"Will be \
         deleted\"\n  provider: \"openai\"\n  model: \"gpt-4o\"\nstrand-dir: \
         \"{}\"\ntie-off-dir: \"{}\"\nprompt-template:\n  input-bundling: \
         \"full-file\"\n  instructions: \"To delete\"\n---\n\n# to-delete-knot\n",
        strand_dir2.display(),
        tie_off_dir2.display()
    );
    fs::write(loom_dir.join("to-delete-knot.md"), second_knot).unwrap();

    let port = 32106;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // 1. Verify initial state: 2 knots
    let (status, body) =
        http_get_retry(&host_port, "/looms/del-loom/knots", 30, 100)
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots at startup");

    // 2. DELETE /looms/{id}/knots/{name}
    let (status, _body) =
        http_delete(&host_port, "/looms/del-loom/knots/to-delete-knot")
            .expect("delete knot should respond");
    assert!(
        status.contains("204"),
        "delete knot should return 204, got: {status}"
    );

    // 3. GET /looms/{id}/knots should show only 1 knot
    let (status, body) =
        http_get_retry(&host_port, "/looms/del-loom/knots", 30, 100)
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
        !knots.contains(&"to-delete-knot".to_string()),
        "deleted knot should not be present"
    );

    // 4. Verify .md file was deleted on disk
    let knot_file = loom_dir.join("to-delete-knot.md");
    assert!(
        !knot_file.exists(),
        "knot .md file should be deleted from disk"
    );

    let _ = shutdown.send(());
}

// ── Discover Endpoint Removed ─────────────────────────────────────────────

/// `POST /looms/discover` returns 404 or 405 because the endpoint has
/// been removed in favour of runtime auto-discovery.
#[test]
fn discover_endpoint_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 32107;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir,
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let shutdown = spawn_server(config);
    wait_for_port(&host_port, 100, 50)
        .expect("server should start listening");

    // POST /looms/discover should not be found (404 or 405).
    // The path `/looms/discover` may match `/looms/{id}` with id="discover"
    // (405 Method Not Allowed since that route has GET/DELETE only),
    // or it may be 404 if no route matches. Either way, the dedicated
    // discover endpoint does not exist.
    let body = serde_json::json!({});
    let (status, _resp) =
        http_post_json(&host_port, "/looms/discover", &body)
            .expect("discover endpoint should respond");

    assert!(
        status.contains("404") || status.contains("405"),
        "POST /looms/discover should return 404 or 405, got: {status}"
    );

    let _ = shutdown.send(());
}
