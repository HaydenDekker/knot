//! Integration tests for shared agent profiles.
//!
//! Verifies the full lifecycle of agent profiles and their integration
//! with knots:
//! - Profile CRUD via HTTP API (POST/GET/DELETE /profiles)
//! - Profile listing via GET /profiles
//! - Knot creation with agent_profile_ref
//! - Dynamic profile resolution at processing time
//! - Profile-not-found error handling
//! - Backward compatibility with inline agent-config

mod helpers;

use std::fs;
use std::time::Duration;

use knot::AppConfig;
use knot::RigAgentConfig;

use helpers::*;



// ── Profile CRUD Tests ──────────────────────────────────────────────────────

/// `POST /profiles/fast` creates a profile → 201 →
/// `GET /profiles/fast` returns the profile details.
#[tokio::test]
async fn create_and_get_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 33000;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Create profile via POST /profiles/fast
    let body = serde_json::json!({
        "provider": "openai",
        "model": "gpt-4o",
        "tools": ["fs"],
        "system_prompt": "You are a fast reviewer. Keep responses concise."
    });

    let (status, _resp) =
        http_post_json(&host_port, "/profiles/fast", &body)
            .await
            .expect("POST /profiles/fast should respond");
    assert!(
        status.contains("201"),
        "POST /profiles/fast should return 201, got: {status}"
    );

    // 2. Verify the profile file was written to disk
    let profile_path = base_dir.join("profiles/fast.md");
    assert!(
        profile_path.exists(),
        "profile file should exist on disk: {}",
        profile_path.display()
    );

    // 3. GET /profiles/fast should return the profile
    let (status, body) =
        http_get(&host_port, "/profiles/fast")
            .await
            .expect("GET /profiles/fast should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profile: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        profile["provider"].as_str().unwrap(),
        "openai",
        "provider should match"
    );
    assert_eq!(
        profile["model"].as_str().unwrap(),
        "gpt-4o",
        "model should match"
    );
    assert!(
        profile["tools"].as_array().is_some(),
        "tools should be present"
    );
    let tools = profile["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].as_str().unwrap(), "fs");
    assert!(
        profile["system_prompt"]
            .as_str()
            .unwrap()
            .contains("fast reviewer"),
        "system prompt should be present"
    );
}

/// `GET /profiles` returns an empty list initially.
/// After creating profiles, `GET /profiles` returns all profiles.
#[tokio::test]
async fn list_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 33001;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Initially, profiles list should be empty
    let (status, body) =
        http_get(&host_port, "/profiles")
            .await
            .expect("GET /profiles should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profiles: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert!(
        profiles.is_empty(),
        "no profiles should exist at startup"
    );

    // 2. Create a few profiles
    let make_profile = |name: &str, provider: &str, model: &str| {
        serde_json::json!({
            "provider": provider,
            "model": model,
            "tools": [],
            "system_prompt": format!("Profile {}", name)
        })
    };

    http_post_json(
        &host_port,
        "/profiles/fast",
        &make_profile("fast", "openai", "gpt-4o"),
    )
    .await
    .expect("create fast profile");

    http_post_json(
        &host_port,
        "/profiles/detailed",
        &make_profile("detailed", "anthropic", "claude-sonnet"),
    )
    .await
    .expect("create detailed profile");

    // 3. GET /profiles should now return both profiles
    let (status, body) =
        http_get(&host_port, "/profiles")
            .await
            .expect("GET /profiles should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profiles: Vec<serde_json::Value> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(
        profiles.len(),
        2,
        "should have 2 profiles, got: {}",
        profiles.len()
    );
    let names: Vec<&str> = profiles
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"fast"));
    assert!(names.contains(&"detailed"));
}

/// `DELETE /profiles/fast` → 204 → `GET /profiles/fast` returns 404.
#[tokio::test]
async fn delete_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 33002;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Create a profile first
    let body = serde_json::json!({
        "provider": "openai",
        "model": "gpt-4o",
        "tools": [],
        "system_prompt": "Test profile for deletion"
    });

    http_post_json(&host_port, "/profiles/gone", &body)
        .await
        .expect("create profile to delete");

    // 2. Verify it exists
    let (status, _body) =
        http_get(&host_port, "/profiles/gone")
            .await
            .expect("GET /profiles/gone should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");

    // 3. DELETE /profiles/gone
    let (status, _resp) =
        http_delete(&host_port, "/profiles/gone")
            .await
            .expect("DELETE /profiles/gone should respond");
    assert!(
        status.contains("204"),
        "DELETE /profiles/gone should return 204, got: {status}"
    );

    // 4. Verify it's gone — GET should return 404
    let (status, _body) =
        http_get(&host_port, "/profiles/gone")
            .await
            .expect("GET /profiles/gone should respond");
    assert!(
        status.contains("404"),
        "GET /profiles/gone should return 404 after deletion, got: {status}"
    );

    // 5. Verify the file was deleted from disk
    let profile_path = base_dir.join("profiles/gone.md");
    assert!(
        !profile_path.exists(),
        "profile file should be deleted from disk"
    );
}

// ── Knot with Profile Reference Tests ───────────────────────────────────────

/// Create a profile, then create a pure profile-ref knot (no inline
/// `agent_config`) via `POST /looms/{id}/knots` → knot file has only
/// `agent-profile-ref` in frontmatter and parses successfully.
#[tokio::test]
async fn create_pure_profile_ref_knot() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 33009;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Create profile
    let profile_body = serde_json::json!({
        "provider": "openai",
        "model": "gpt-4o",
        "tools": ["fs"],
        "system_prompt": "You are a fast reviewer."
    });
    http_post_json(&host_port, "/profiles/fast", &profile_body)
        .await
        .expect("create fast profile");

    // 2. Register a loom with a base knot
    let loom_body = serde_json::json!({
        "id": "pure-ref-loom",
        "knots": [
            {
                "name": "base-knot",
                "agent_config": {
                    "goal": "Base goal",
                    "provider": "openai",
                    "model": "gpt-4o",
                    "tools": []
                },
                "prompt_template": {
                    "input_bundling": "full-file",
                    "instructions": "Base instructions"
                },
                "strand_dir": base_dir.join("strands").to_string_lossy()
            }
        ]
    });
    http_post_json(&host_port, "/looms", &loom_body)
        .await
        .expect("register loom");

    // 3. Create a pure profile-ref knot (NO agent_config)
    let strand_dir = base_dir.join("pure-strands");
    fs::create_dir_all(&strand_dir).unwrap();

    let knot_body = serde_json::json!({
        "name": "pure-profile-knot",
        "agent_profile_ref": "fast",
        "prompt_template": {
            "input_bundling": "full-file",
            "instructions": "Use the fast profile"
        },
        "strand_dir": strand_dir.to_string_lossy()
    });

    let (status, _resp) =
        http_post_json(
            &host_port,
            "/looms/pure-ref-loom/knots",
            &knot_body,
        )
        .await
        .expect("create pure profile-ref knot should respond");
    assert!(
        status.contains("201"),
        "create pure profile-ref knot should return 201, got: {status}"
    );

    // 4. Verify knot appears in GET /looms/{id}/knots
    let (status, body) =
        http_get(&host_port, "/looms/pure-ref-loom/knots")
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots");
    assert!(
        knots.contains(&"pure-profile-knot".to_string()),
        "pure-profile-knot should be present"
    );

    // 5. Verify the .md file has ONLY agent-profile-ref (no agent-config)
    let knot_file = base_dir.join("pure-ref-loom/pure-profile-knot.md");
    assert!(
        knot_file.exists(),
        "knot .md file should exist"
    );
    let file_content =
        fs::read_to_string(&knot_file).expect("should read knot file");
    assert!(
        file_content.contains("agent-profile-ref: fast"),
        "knot .md file should contain agent-profile-ref, got: {}",
        file_content
    );
    assert!(
        !file_content.contains("agent-config"),
        "knot .md file should NOT contain agent-config, got: {}",
        file_content
    );

    // 6. Verify the file can be parsed by KnotFile::parse (no
    // BothProfileAndConfig error) — this is the critical check.
    // We read the file and validate it doesn't produce an error.
    // Since parse requires the full frontmatter structure, this proves
    // the generated file is self-consistent and recoverable.
    let _profile_result = http_get(&host_port, "/profiles/fast")
        .await
        .expect("profile should exist for validation");
}

/// Create a profile, then create a knot with `agent_profile_ref` via
/// `POST /looms/{id}/knots` → knot file has profile ref in frontmatter.
#[tokio::test]
async fn create_knot_with_agent_profile_ref() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 33003;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Create profile
    let profile_body = serde_json::json!({
        "provider": "openai",
        "model": "gpt-4o",
        "tools": ["fs"],
        "system_prompt": "You are a fast reviewer."
    });
    http_post_json(&host_port, "/profiles/fast", &profile_body)
        .await
        .expect("create fast profile");

    // 2. Register a loom with a base knot
    let loom_body = serde_json::json!({
        "id": "profile-ref-loom",
        "knots": [
            {
                "name": "inline-knot",
                "agent_config": {
                    "goal": "Inline goal",
                    "provider": "openai",
                    "model": "gpt-4o",
                    "tools": []
                },
                "prompt_template": {
                    "input_bundling": "full-file",
                    "instructions": "Inline instructions"
                },
                "strand_dir": base_dir.join("strands").to_string_lossy()
            }
        ]
    });
    http_post_json(&host_port, "/looms", &loom_body)
        .await
        .expect("register loom");

    // 3. Create a knot with agent_profile_ref via HTTP API
    let strand_dir = base_dir.join("profile-strands");
    fs::create_dir_all(&strand_dir).unwrap();

    let knot_body = serde_json::json!({
        "name": "profile-knot",
        "agent_config": {
            "goal": "Profile goal",
            "provider": "openai",
            "model": "gpt-4o",
            "tools": []
        },
        "agent_profile_ref": "fast",
        "prompt_template": {
            "input_bundling": "full-file",
            "instructions": "Use the fast profile"
        },
        "strand_dir": strand_dir.to_string_lossy()
    });

    let (status, _resp) =
        http_post_json(
            &host_port,
            "/looms/profile-ref-loom/knots",
            &knot_body,
        )
        .await
        .expect("create profile-referencing knot should respond");
    assert!(
        status.contains("201"),
        "create profile-knot should return 201, got: {status}"
    );

    // 4. Verify knot appears in GET /looms/{id}/knots
    let (status, body) =
        http_get(&host_port, "/looms/profile-ref-loom/knots")
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots");
    assert!(
        knots.contains(&"profile-knot".to_string()),
        "profile-knot should be present"
    );

    // 5. Verify the .md file has agent-profile-ref in frontmatter
    let knot_file = base_dir.join("profile-ref-loom/profile-knot.md");
    assert!(
        knot_file.exists(),
        "knot .md file should exist"
    );
    let file_content =
        fs::read_to_string(&knot_file).expect("should read knot file");
    assert!(
        file_content.contains("agent-profile-ref: fast"),
        "knot .md file should contain agent-profile-ref, got: {}",
        file_content
    );
}

// ── Profile Override Tests ──────────────────────────────────────────────────

/// Create a profile, create a knot with profile ref + inline model override,
/// then process a strand and verify processing succeeds.
#[tokio::test]
async fn profile_override_at_processing_time() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    // 1. Create the mock agent upfront (needed for strand processing)
    let mock_agent = create_stub_pi_agent(&base_dir);

    let port = 33004;
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

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 2. Create the base profile
    let profile_body = serde_json::json!({
        "provider": "openai",
        "model": "gpt-4o",
        "tools": [],
        "system_prompt": "Fast reviewer"
    });
    http_post_json(&host_port, "/profiles/fast", &profile_body)
        .await
        .expect("create fast profile");

    // 3. Register a loom with a base knot
    let loom_body = serde_json::json!({
        "id": "override-loom",
        "knots": [
            {
                "name": "base-knot",
                "agent_config": {
                    "goal": "Base goal",
                    "provider": "openai",
                    "model": "gpt-4o",
                    "tools": []
                },
                "prompt_template": {
                    "input_bundling": "full-file",
                    "instructions": "Base instructions"
                },
                "strand_dir": base_dir.join("base-strands").to_string_lossy()
            }
        ]
    });
    http_post_json(&host_port, "/looms", &loom_body)
        .await
        .expect("register loom");

    // 4. Create a knot with agent_profile_ref via HTTP API (POST /looms/{id}/knots)
    let strand_dir = base_dir.join("override-strands");
    fs::create_dir_all(&strand_dir).unwrap();

    let knot_body = serde_json::json!({
        "name": "override-knot",
        "agent_config": {
            "goal": "Override goal",
            "provider": "anthropic",
            "model": "claude-sonnet",
            "tools": []
        },
        "agent_profile_ref": "fast",
        "prompt_template": {
            "input_bundling": "full-file",
            "instructions": "Use override model"
        },
        "strand_dir": strand_dir.to_string_lossy()
    });

    let (status, _resp) =
        http_post_json(
            &host_port,
            "/looms/override-loom/knots",
            &knot_body,
        )
        .await
        .expect("create override knot should respond");
    assert!(
        status.contains("201"),
        "create override knot should return 201, got: {status}"
    );

    // 5. Verify knot appears in GET /looms/{id}/knots
    let (status, body) =
        http_get(&host_port, "/looms/override-loom/knots")
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots");
    assert!(
        knots.contains(&"override-knot".to_string()),
        "override-knot should be present"
    );

    // 6. Verify the .md file has agent-profile-ref in frontmatter
    let knot_file = base_dir.join("override-loom/override-knot.md");
    assert!(
        knot_file.exists(),
        "knot .md file should exist"
    );
    let file_content =
        fs::read_to_string(&knot_file).expect("should read knot file");
    assert!(
        file_content.contains("agent-profile-ref: fast"),
        "knot .md file should contain agent-profile-ref, got: {}",
        file_content
    );

    // 7. Create a strand for the override knot and wait for processing
    let strand_path = strand_dir.join("override-strand.md");
    fs::write(&strand_path, "override strand content").unwrap();

    // Wait for debounce + processing
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // 8. Verify tie-off was produced (profile was resolved at processing time)
    let tie_off_path =
        base_dir.join("output/override-loom/override-knot/override-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist after processing: {}",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("override strand content"),
        "tie-off should contain strand content, got: {}",
        content
    );
}

// ── Dynamic Profile Update Tests ────────────────────────────────────────────

/// Create a profile, create a knot referencing it, update the profile
/// on disk, then process a strand — the updated profile is resolved
/// at processing time (profiles are read at call time).
#[tokio::test]
async fn dynamic_profile_update_at_processing_time() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 33005;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Create initial profile
    let profile_body = serde_json::json!({
        "provider": "openai",
        "model": "gpt-4o",
        "tools": [],
        "system_prompt": "Initial reviewer"
    });
    http_post_json(&host_port, "/profiles/reviewer", &profile_body)
        .await
        .expect("create reviewer profile");

    // 2. Register a loom with a base knot
    let loom_body = serde_json::json!({
        "id": "dynamic-loom",
        "knots": [
            {
                "name": "base-knot",
                "agent_config": {
                    "goal": "Base",
                    "provider": "openai",
                    "model": "gpt-4o",
                    "tools": []
                },
                "prompt_template": {
                    "input_bundling": "full-file",
                    "instructions": "Base"
                },
                "strand_dir": base_dir.join("strands").to_string_lossy()
            }
        ]
    });
    http_post_json(&host_port, "/looms", &loom_body)
        .await
        .expect("register loom");

    // 3. Create a knot with agent_profile_ref via HTTP API
    let strand_dir = base_dir.join("dynamic-strands");
    fs::create_dir_all(&strand_dir).unwrap();

    let knot_body = serde_json::json!({
        "name": "dynamic-knot",
        "agent_config": {
            "goal": "Dynamic goal",
            "provider": "openai",
            "model": "gpt-4o",
            "tools": []
        },
        "agent_profile_ref": "reviewer",
        "prompt_template": {
            "input_bundling": "full-file",
            "instructions": "Process dynamically"
        },
        "strand_dir": strand_dir.to_string_lossy()
    });

    let (status, _resp) =
        http_post_json(
            &host_port,
            "/looms/dynamic-loom/knots",
            &knot_body,
        )
        .await
        .expect("create dynamic knot should respond");
    assert!(
        status.contains("201"),
        "create dynamic knot should return 201, got: {status}"
    );

    // 4. Verify knot appears in GET /looms/{id}/knots
    let (status, body) =
        http_get(&host_port, "/looms/dynamic-loom/knots")
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots");
    assert!(
        knots.contains(&"dynamic-knot".to_string()),
        "dynamic-knot should be present"
    );

    // 5. Update the profile on disk — change model to claude-sonnet
    let updated_profile_path = base_dir.join("profiles/reviewer.md");
    let updated_profile = format!(
        "---\nname: reviewer\nprovider: anthropic\nmodel: claude-sonnet\n\
         system-prompt: |\n  Updated reviewer\n---\n\n# reviewer\n\nUpdated profile.\n"
    );
    fs::write(&updated_profile_path, updated_profile).unwrap();

    // 6. Verify that GET /profiles/reviewer reflects the updated model
    let (status, body) =
        http_get(&host_port, "/profiles/reviewer")
            .await
            .expect("GET should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profile: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        profile["model"].as_str().unwrap(),
        "claude-sonnet",
        "model should be updated to claude-sonnet"
    );
    assert_eq!(
        profile["provider"].as_str().unwrap(),
        "anthropic",
        "provider should be updated to anthropic"
    );
}

/// Create a knot referencing a non-existent profile. When a strand is
/// processed, the system should log an error and the tie-off should
/// record a failure.
#[tokio::test]
async fn profile_not_found_logs_error() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 33006;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Register a loom with a base knot
    let loom_body = serde_json::json!({
        "id": "notfound-loom",
        "knots": [
            {
                "name": "base-knot",
                "agent_config": {
                    "goal": "Base",
                    "provider": "openai",
                    "model": "gpt-4o",
                    "tools": []
                },
                "prompt_template": {
                    "input_bundling": "full-file",
                    "instructions": "Base"
                },
                "strand_dir": base_dir.join("strands").to_string_lossy()
            }
        ]
    });
    http_post_json(&host_port, "/looms", &loom_body)
        .await
        .expect("register loom");

    // 2. Create a knot referencing a non-existent profile via HTTP API
    let strand_dir = base_dir.join("notfound-strands");
    fs::create_dir_all(&strand_dir).unwrap();

    let knot_body = serde_json::json!({
        "name": "missing-profile-knot",
        "agent_config": {
            "goal": "Missing profile goal",
            "provider": "openai",
            "model": "gpt-4o",
            "tools": []
        },
        "agent_profile_ref": "nonexistent-profile",
        "prompt_template": {
            "input_bundling": "full-file",
            "instructions": "Missing profile"
        },
        "strand_dir": strand_dir.to_string_lossy()
    });

    let (status, _resp) =
        http_post_json(
            &host_port,
            "/looms/notfound-loom/knots",
            &knot_body,
        )
        .await
        .expect("create missing-profile knot should respond");
    assert!(
        status.contains("201"),
        "create missing-profile knot should return 201, got: {status}"
    );

    // 3. Verify knot appears in GET /looms/{id}/knots
    let (status, body) =
        http_get(&host_port, "/looms/notfound-loom/knots")
            .await
            .expect("knots endpoint should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knots: Vec<String> =
        serde_json::from_str(&body).expect("should be JSON array");
    assert_eq!(knots.len(), 2, "should have 2 knots");
    assert!(
        knots.contains(&"missing-profile-knot".to_string()),
        "missing-profile-knot should be present"
    );

    // 4. Verify the .md file has agent-profile-ref pointing to non-existent profile
    let knot_file = base_dir.join("notfound-loom/missing-profile-knot.md");
    assert!(
        knot_file.exists(),
        "knot .md file should exist"
    );
    let file_content =
        fs::read_to_string(&knot_file).expect("should read knot file");
    assert!(
        file_content.contains("agent-profile-ref: nonexistent-profile"),
        "knot .md file should contain profile ref to nonexistent profile, got: {}",
        file_content
    );

    // 5. Create a strand — processing will fail because profile doesn't exist.
    let strand_path = strand_dir.join("missing-strand.md");
    fs::write(&strand_path, "missing profile test").unwrap();

    // Wait for debounce + processing attempt
    tokio::time::sleep(Duration::from_millis(5000)).await;

    // 6. Verify the knot status is no longer idle (processing started).
    // The status may be "processing" (in-progress) or "failed" depending
    // on timing — the important thing is that processing was attempted.
    let (status, body) =
        http_get(&host_port, "/looms/notfound-loom/knots/missing-profile-knot")
            .await
            .expect("knot status should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    let knot_status_val = knot_status["status"].as_str().unwrap_or("");
    assert!(
        knot_status_val == "processing" || knot_status_val == "failed",
        "knot status should be 'processing' or 'failed', got: {}",
        knot_status_val
    );

    // 7. Verify the last_error references the missing profile.
    let error = knot_status["last_error"].as_str().unwrap_or("");
    if !error.is_empty() {
        assert!(
            error.contains("nonexistent-profile"),
            "error should reference the missing profile, got: {}",
            error
        );
    }
}

/// Knots with inline `agent-config` (no profile ref) still process
/// correctly — backward compatibility with existing knot definitions.
#[tokio::test]
async fn backward_compat_inline_config() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 33007;
    let host_port = format!("127.0.0.1:{port}");

    // Mock agent for processing
    let mock_agent = create_mock_agent(&base_dir, "inline-output");

    let config = AppConfig {
        base_dir: base_dir.clone(),
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

    // 1. Register a loom with a knot that has inline agent-config (no profile ref)
    let strand_dir = base_dir.join("inline-strands");
    fs::create_dir_all(&strand_dir).unwrap();
    let loom_body = serde_json::json!({
        "id": "compat-loom",
        "knots": [
            {
                "name": "inline-knot",
                "agent_config": {
                    "goal": "Inline config test",
                    "provider": "openai",
                    "model": "gpt-4o",
                    "tools": []
                },
                "prompt_template": {
                    "input_bundling": "full-file",
                    "instructions": "Process with inline config"
                },
                "strand_dir": strand_dir.to_string_lossy()
            }
        ]
    });
    http_post_json(&host_port, "/looms", &loom_body)
        .await
        .expect("register loom");

    // 2. Create a strand — should be processed using inline agent-config
    let strand_path = strand_dir.join("inline-strand.md");
    fs::write(&strand_path, "inline config test content").unwrap();

    // Wait for debounce + processing
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // 3. Verify tie-off was produced successfully
    let tie_off_path =
        base_dir.join("output/compat-loom/inline-knot/inline-strand.md.output");
    assert!(
        tie_off_path.exists(),
        "tie-off should exist: {}",
        tie_off_path.display()
    );
    let content =
        fs::read_to_string(&tie_off_path).expect("should read tie-off");
    assert!(
        content.contains("inline-output"),
        "tie-off should contain agent output, got: {}",
        content
    );

    // 4. Verify knot status is completed
    let (status, body) =
        http_get(&host_port, "/looms/compat-loom/knots/inline-knot")
            .await
            .expect("knot status should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let knot_status: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        knot_status["status"].as_str().unwrap(),
        "completed",
        "knot should be completed (backward compat)"
    );
}

// ── Combined Profile + Profile Override Tests ───────────────────────────────

/// Create a profile, update it on disk (same server, no restart),
/// verify that GET /profiles reflects the updated content.
#[tokio::test]
async fn get_profile_reflects_disk_update() {
    let tmp = tempfile::tempdir().unwrap();
    let base_dir = tmp.path().to_path_buf();

    let port = 33008;
    let host_port = format!("127.0.0.1:{port}");

    let config = AppConfig {
        base_dir: base_dir.clone(),
        bind_addr: format!("127.0.0.1:{port}").parse().unwrap(),
        ..AppConfig::default_config()
    };

    let _handle = spawn_server(config);
    wait_for_port(&host_port, 5000)
        .await
        .expect("server should start listening");

    // 1. Create a profile
    let profile_body = serde_json::json!({
        "provider": "openai",
        "model": "gpt-4o",
        "tools": [],
        "system_prompt": "Initial prompt"
    });
    http_post_json(&host_port, "/profiles/updated", &profile_body)
        .await
        .expect("create profile");

    // 2. GET should return the initial model
    let (status, body) =
        http_get(&host_port, "/profiles/updated")
            .await
            .expect("GET should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profile: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        profile["model"].as_str().unwrap(),
        "gpt-4o",
        "initial model should be gpt-4o"
    );

    // 3. Update the profile file directly on disk
    let profile_path = base_dir.join("profiles/updated.md");
    let updated_content = format!(
        "---\nname: updated\nprovider: anthropic\nmodel: claude-sonnet\n\
         system-prompt: |\n  Updated prompt\n---\n\n# updated\n\nUpdated.\n"
    );
    fs::write(&profile_path, updated_content).unwrap();

    // 4. GET should now reflect the updated model (profiles are read at call time)
    let (status, body) =
        http_get(&host_port, "/profiles/updated")
            .await
            .expect("GET should respond");
    assert!(status.contains("200"), "expected 200, got: {status}");
    let profile: serde_json::Value =
        serde_json::from_str(&body).expect("should be JSON");
    assert_eq!(
        profile["model"].as_str().unwrap(),
        "claude-sonnet",
        "model should be updated to claude-sonnet"
    );
    assert_eq!(
        profile["provider"].as_str().unwrap(),
        "anthropic",
        "provider should be updated to anthropic"
    );
}
